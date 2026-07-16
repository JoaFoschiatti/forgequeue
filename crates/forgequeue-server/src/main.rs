mod api;
mod config;
mod db;
mod error;
mod processors;
mod storage;
mod worker;

use std::{future::IntoFuture, path::PathBuf, sync::Arc};

use anyhow::{Context, Result, anyhow};
use axum::{Router, http::StatusCode, response::IntoResponse, routing::get};
use clap::{Parser, Subcommand};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use crate::{
    api::{AppState, pg_event_bridge},
    config::Config,
    db::Database,
    storage::BlobStore,
    worker::{Worker, cleanup_loop, cleanup_once},
};

#[derive(Debug, Parser)]
#[command(
    name = "forgequeue",
    version,
    about = "Procesador durable de imágenes y PDFs"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Ejecuta solamente la API HTTP.
    Api,
    /// Ejecuta un worker con concurrencia uno.
    Worker,
    /// Ejecuta API, worker y limpieza en un proceso para la demo gratuita.
    All,
    /// Borra artefactos y metadatos expirados una vez.
    Cleanup,
    /// Exporta el contrato OpenAPI sin conectarse a servicios externos.
    Openapi {
        #[arg(short, long, default_value = "openapi.json")]
        output: PathBuf,
    },
}

struct Runtime {
    config: Arc<Config>,
    db: Database,
    storage: BlobStore,
    metrics: PrometheusHandle,
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    let cli = Cli::parse();
    if let Some(Command::Openapi { output }) = &cli.command {
        tokio::fs::write(output, api::openapi_json())
            .await
            .with_context(|| format!("failed to write {}", output.display()))?;
        tracing::info!(path = %output.display(), "OpenAPI document exported");
        return Ok(());
    }

    let runtime = bootstrap().await?;
    match cli.command.unwrap_or(Command::All) {
        Command::Api => run_api(runtime, false).await,
        Command::Worker => run_worker(runtime).await,
        Command::All => run_api(runtime, true).await,
        Command::Cleanup => {
            let (artifacts, metadata) = cleanup_once(&runtime.db, &runtime.storage).await?;
            tracing::info!(artifacts, metadata, "cleanup command finished");
            Ok(())
        }
        Command::Openapi { .. } => unreachable!(),
    }
}

async fn bootstrap() -> Result<Runtime> {
    let config = Arc::new(Config::from_env()?);
    let db = Database::connect(&config.database_url).await?;
    db.migrate().await?;
    let storage = BlobStore::from_config(&config).await?;
    storage
        .verify()
        .await
        .context("failed to verify object store")?;
    let metrics = PrometheusBuilder::new()
        .install_recorder()
        .context("failed to install Prometheus recorder")?;
    Ok(Runtime {
        config,
        db,
        storage,
        metrics,
    })
}

async fn run_worker(runtime: Runtime) -> Result<()> {
    let shutdown = CancellationToken::new();
    let signal_shutdown = shutdown.clone();
    tokio::spawn(async move {
        wait_for_shutdown().await;
        signal_shutdown.cancel();
    });
    let metrics_server = runtime.config.worker_metrics_address.map(|address| {
        tokio::spawn(run_worker_metrics(
            address,
            runtime.metrics.clone(),
            shutdown.clone(),
        ))
    });
    let worker_result = Worker::new(runtime.config, runtime.db, runtime.storage)
        .run(shutdown.clone())
        .await;
    shutdown.cancel();
    if let Some(server) = metrics_server {
        match server.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::error!(%error, "worker metrics server failed"),
            Err(error) => tracing::error!(%error, "worker metrics task panicked"),
        }
    }
    worker_result
}

async fn run_worker_metrics(
    address: std::net::SocketAddr,
    metrics: PrometheusHandle,
    shutdown: CancellationToken,
) -> Result<()> {
    let app = Router::new()
        .route("/health/live", get(|| async { StatusCode::OK }))
        .route(
            "/metrics",
            get(move || {
                let metrics = metrics.clone();
                async move {
                    (
                        [(
                            axum::http::header::CONTENT_TYPE,
                            "text/plain; version=0.0.4",
                        )],
                        metrics.render(),
                    )
                        .into_response()
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .with_context(|| format!("failed to bind worker metrics at {address}"))?;
    tracing::info!(%address, "worker metrics listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await
        .context("worker metrics server failed")
}

async fn run_api(runtime: Runtime, with_worker: bool) -> Result<()> {
    let shutdown = CancellationToken::new();
    let signal_shutdown = shutdown.clone();
    tokio::spawn(async move {
        wait_for_shutdown().await;
        signal_shutdown.cancel();
    });

    let (events, _) = tokio::sync::broadcast::channel(256);
    let state = AppState {
        config: runtime.config.clone(),
        db: runtime.db.clone(),
        storage: runtime.storage.clone(),
        events: events.clone(),
        metrics: runtime.metrics,
    };
    let mut background = tokio::task::JoinSet::new();
    background.spawn(pg_event_bridge(
        runtime.db.clone(),
        events,
        shutdown.clone(),
    ));

    if with_worker {
        background.spawn(
            Worker::new(
                runtime.config.clone(),
                runtime.db.clone(),
                runtime.storage.clone(),
            )
            .run(shutdown.clone()),
        );
        background.spawn(cleanup_loop(
            runtime.db.clone(),
            runtime.storage.clone(),
            shutdown.clone(),
        ));
    }

    let listener = tokio::net::TcpListener::bind(runtime.config.bind_address)
        .await
        .with_context(|| format!("failed to bind {}", runtime.config.bind_address))?;
    tracing::info!(
        address = %runtime.config.bind_address,
        mode = if with_worker { "all" } else { "api" },
        "ForgeQueue listening"
    );
    let server = axum::serve(
        listener,
        api::router(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown({
        let shutdown = shutdown.clone();
        async move { shutdown.cancelled().await }
    })
    .into_future();
    tokio::pin!(server);
    let mut background_failure = None;
    let server_result = tokio::select! {
        result = &mut server => result,
        task = background.join_next() => {
            if !shutdown.is_cancelled() {
                let error = match task {
                    Some(Ok(Ok(()))) => anyhow!("background task stopped unexpectedly"),
                    Some(Ok(Err(error))) => error.context("background task failed"),
                    Some(Err(error)) => anyhow!(error).context("background task panicked"),
                    None => anyhow!("all background tasks stopped unexpectedly"),
                };
                tracing::error!(%error, "stopping after background failure");
                background_failure = Some(error);
            }
            shutdown.cancel();
            server.await
        }
    };
    shutdown.cancel();
    while let Some(task) = background.join_next().await {
        match task {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::error!(%error, "background task failed"),
            Err(error) => tracing::error!(%error, "background task panicked"),
        }
    }
    server_result.context("HTTP server failed")?;
    if let Some(error) = background_failure {
        return Err(error);
    }
    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("forgequeue_server=info,tower_http=info"));
    if std::env::var("LOG_FORMAT").as_deref() == Ok("json") {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .json()
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .compact()
            .init();
    }
}

async fn wait_for_shutdown() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
