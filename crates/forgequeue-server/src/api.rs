use std::{convert::Infallible, io::Cursor, net::SocketAddr, str::FromStr, sync::Arc};

use axum::{
    Json, Router,
    body::Body,
    extract::{ConnectInfo, DefaultBodyLimit, Multipart, Path, Query, State},
    http::{
        HeaderMap, HeaderName, HeaderValue, Method, StatusCode,
        header::{AUTHORIZATION, CONTENT_DISPOSITION, CONTENT_TYPE},
    },
    middleware::{self, Next},
    response::{
        Html, IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use bytes::Bytes;
use forgequeue_core::{
    Job, JobDetail, JobKind, JobPage, JobStatus, ProblemDetails, SessionResponse,
};
use futures_util::Stream;
use image::ImageReader;
use metrics_exporter_prometheus::PrometheusHandle;
use rand::{RngCore, rngs::OsRng};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use sqlx::postgres::PgListener;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi, ToSchema};
use uuid::Uuid;

use crate::{
    config::Config,
    db::{CreateJobOutcome, Database, NewJob, Session, UploadLimit, UploadLimits},
    error::{AppError, AppResult, REQUEST_ID},
    processors::validate_pdf_page_count,
    storage::BlobStore,
};

const SESSION_BYTES: usize = 32;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: Database,
    pub storage: BlobStore,
    pub events: broadcast::Sender<Uuid>,
    pub metrics: PrometheusHandle,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct JobListQuery {
    status: Option<String>,
    kind: Option<String>,
    cursor: Option<Uuid>,
}

#[allow(dead_code)]
#[derive(Debug, ToSchema)]
struct UploadBody {
    #[schema(format = Binary)]
    file: String,
}

struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "bearer_auth",
                SecurityScheme::Http(
                    HttpBuilder::new()
                        .scheme(HttpAuthScheme::Bearer)
                        .bearer_format("opaque anonymous session token")
                        .build(),
                ),
            );
        }
    }
}

#[derive(OpenApi)]
#[openapi(
    paths(
        create_session,
        create_job,
        list_jobs,
        get_job,
        job_events,
        cancel_job,
        retry_job,
        delete_job,
        download_output,
        live,
        ready,
        metrics_endpoint,
    ),
    components(schemas(
        Job,
        JobDetail,
        JobKind,
        JobPage,
        JobStatus,
        ProblemDetails,
        SessionResponse,
        JobListQuery,
        UploadBody,
    )),
    modifiers(&SecurityAddon),
    tags(
        (name = "sessions", description = "Sesiones anónimas"),
        (name = "jobs", description = "Procesamiento durable"),
        (name = "system", description = "Estado y observabilidad")
    ),
    info(
        title = "ForgeQueue API",
        version = "0.1.0",
        description = "Cola durable para procesar imágenes y PDFs."
    )
)]
pub struct ApiDoc;

pub fn openapi_json() -> String {
    ApiDoc::openapi()
        .to_pretty_json()
        .expect("OpenAPI document must serialize")
}

pub fn router(state: AppState) -> Router {
    let cors_origin = state
        .config
        .cors_origin
        .parse::<HeaderValue>()
        .expect("validated CORS_ORIGIN");
    let request_id_header = HeaderName::from_static("x-request-id");
    let body_limit = state.config.max_upload_bytes + 1024 * 1024;

    Router::new()
        .route("/api/v1/sessions", post(create_session))
        .route("/api/v1/jobs", post(create_job).get(list_jobs))
        .route("/api/v1/jobs/{id}", get(get_job).delete(delete_job))
        .route("/api/v1/jobs/{id}/events", get(job_events))
        .route("/api/v1/jobs/{id}/cancel", post(cancel_job))
        .route("/api/v1/jobs/{id}/retry", post(retry_job))
        .route(
            "/api/v1/jobs/{job_id}/outputs/{output_id}",
            get(download_output),
        )
        .route("/api/openapi.json", get(openapi_document))
        .route("/docs", get(docs))
        .route("/health/live", get(live))
        .route("/health/ready", get(ready))
        .route("/metrics", get(metrics_endpoint))
        .layer(DefaultBodyLimit::max(body_limit))
        .layer(
            CorsLayer::new()
                .allow_origin(cors_origin)
                .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::OPTIONS])
                .allow_headers([
                    AUTHORIZATION,
                    CONTENT_TYPE,
                    HeaderName::from_static("idempotency-key"),
                ])
                .expose_headers([request_id_header.clone()]),
        )
        .layer(TraceLayer::new_for_http())
        .layer(middleware::from_fn(request_id_context))
        .with_state(state)
}

async fn request_id_context(mut request: axum::http::Request<Body>, next: Next) -> Response {
    let request_id = Uuid::new_v4().to_string();
    let header_value = HeaderValue::from_str(&request_id).expect("UUID request id is valid");
    request.headers_mut().insert(
        HeaderName::from_static("x-request-id"),
        header_value.clone(),
    );

    REQUEST_ID
        .scope(request_id, async move {
            let mut response = next.run(request).await;
            response
                .headers_mut()
                .insert(HeaderName::from_static("x-request-id"), header_value);
            response
        })
        .await
}

#[utoipa::path(
    post,
    path = "/api/v1/sessions",
    tag = "sessions",
    responses(
        (status = 201, description = "Sesión creada", body = SessionResponse),
        (status = 500, description = "Error interno", body = ProblemDetails)
    )
)]
async fn create_session(State(state): State<AppState>) -> AppResult<impl IntoResponse> {
    let mut random = [0_u8; SESSION_BYTES];
    OsRng.fill_bytes(&mut random);
    let token = format!("fq_{}", URL_SAFE_NO_PAD.encode(random));
    let session = state
        .db
        .create_session(&sha256(token.as_bytes()), state.config.metadata_retention)
        .await
        .map_err(AppError::Internal)?;
    Ok((
        StatusCode::CREATED,
        Json(SessionResponse {
            token,
            expires_at: session.expires_at,
        }),
    ))
}

#[utoipa::path(
    post,
    path = "/api/v1/jobs",
    tag = "jobs",
    request_body(content = UploadBody, content_type = "multipart/form-data", description = "Archivo JPEG, PNG, WebP o PDF"),
    responses(
        (status = 202, description = "Trabajo encolado", body = Job),
        (status = 200, description = "Trabajo idempotente existente", body = Job),
        (status = 401, body = ProblemDetails),
        (status = 422, body = ProblemDetails),
        (status = 429, body = ProblemDetails)
    ),
    security(("bearer_auth" = []))
)]
async fn create_job(
    State(state): State<AppState>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> AppResult<impl IntoResponse> {
    let session = authenticate(&state, &headers).await?;
    let idempotency_key = idempotency_key(&headers)?;
    if let Some(key) = idempotency_key.as_deref()
        && let Some(existing) = state
            .db
            .find_idempotent_job(session.id, key)
            .await
            .map_err(AppError::Internal)?
    {
        return Ok((StatusCode::OK, Json(existing.view)));
    }

    let client_ip_hash = client_ip_hash(&state, &headers, address);
    enforce_rate_limits(&state, session.id, &client_ip_hash).await?;
    let upload = read_upload(&state, &mut multipart).await?;
    let id = Uuid::now_v7();
    let object_key = format!("sessions/{}/jobs/{id}/input", session.id);
    state.storage.put(&object_key, upload.bytes.clone()).await?;
    let new_job = NewJob {
        id,
        session_id: session.id,
        kind: upload.kind,
        original_name: &upload.filename,
        input_object_key: &object_key,
        input_content_type: &upload.content_type,
        input_size: upload.bytes.len() as i64,
        input_sha256: &upload.sha256,
        idempotency_key: idempotency_key.as_deref(),
        client_ip_hash: &client_ip_hash,
        retry_of_job_id: None,
        metadata_retention: state.config.metadata_retention,
    };
    let job = match state
        .db
        .create_job(new_job, Some(upload_limits(&state)))
        .await
    {
        Ok(CreateJobOutcome::Created(job)) => job,
        Ok(CreateJobOutcome::Existing(existing)) => {
            let _ = state.storage.delete(&object_key).await;
            return Ok((StatusCode::OK, Json(existing.view)));
        }
        Ok(CreateJobOutcome::LimitExceeded(limit)) => {
            let _ = state.storage.delete(&object_key).await;
            return Err(rate_limit_error(&state, limit));
        }
        Err(error) => {
            let _ = state.storage.delete(&object_key).await;
            if let Some(key) = idempotency_key.as_deref()
                && let Some(existing) = state
                    .db
                    .find_idempotent_job(session.id, key)
                    .await
                    .map_err(AppError::Internal)?
            {
                return Ok((StatusCode::OK, Json(existing.view)));
            }
            return Err(AppError::Internal(error));
        }
    };
    metrics::counter!("forgequeue_jobs_created_total", "kind" => job.view.kind.as_str())
        .increment(1);
    Ok((StatusCode::ACCEPTED, Json(job.view)))
}

#[utoipa::path(
    get,
    path = "/api/v1/jobs",
    tag = "jobs",
    params(
        ("status" = Option<String>, Query, description = "Estado snake_case"),
        ("kind" = Option<String>, Query, description = "image o pdf"),
        ("cursor" = Option<Uuid>, Query, description = "Cursor de paginación")
    ),
    responses((status = 200, body = JobPage), (status = 401, body = ProblemDetails)),
    security(("bearer_auth" = []))
)]
async fn list_jobs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<JobListQuery>,
) -> AppResult<Json<JobPage>> {
    let session = authenticate(&state, &headers).await?;
    let status = query
        .status
        .as_deref()
        .map(JobStatus::from_str)
        .transpose()
        .map_err(|_| AppError::Validation("Estado desconocido.".to_owned()))?;
    let kind = query
        .kind
        .as_deref()
        .map(JobKind::from_str)
        .transpose()
        .map_err(|_| AppError::Validation("Tipo de trabajo desconocido.".to_owned()))?;
    let page = state
        .db
        .list_jobs(session.id, status, kind, query.cursor, 25)
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(page))
}

#[utoipa::path(
    get,
    path = "/api/v1/jobs/{id}",
    tag = "jobs",
    params(("id" = Uuid, Path)),
    responses((status = 200, body = JobDetail), (status = 404, body = ProblemDetails)),
    security(("bearer_auth" = []))
)]
async fn get_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> AppResult<Json<JobDetail>> {
    let session = authenticate(&state, &headers).await?;
    Ok(Json(job_detail(&state, session.id, id).await?))
}

#[utoipa::path(
    get,
    path = "/api/v1/jobs/{id}/events",
    tag = "jobs",
    params(("id" = Uuid, Path)),
    responses((status = 200, description = "Flujo SSE"), (status = 404, body = ProblemDetails)),
    security(("bearer_auth" = []))
)]
async fn job_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> AppResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    let session = authenticate(&state, &headers).await?;
    let mut receiver = state.events.subscribe();
    let initial = job_detail(&state, session.id, id).await?;
    let stream_state = state.clone();
    let stream = async_stream::stream! {
        yield Ok(Event::default().event("job.snapshot").json_data(&initial).unwrap_or_else(|_| Event::default()));
        if initial.job.status.is_terminal() {
            return;
        }
        loop {
            match receiver.recv().await {
                Ok(changed_id) if changed_id == id => {
                    match stream_state.db.get_job_detail(session.id, id).await {
                        Ok(Some(detail)) => {
                            let terminal = detail.job.status.is_terminal();
                            yield Ok(Event::default().event("job.updated").json_data(&detail).unwrap_or_else(|_| Event::default()));
                            if terminal {
                                break;
                            }
                        }
                        Ok(None) => break,
                        Err(error) => {
                            tracing::warn!(%error, %id, "failed to refresh SSE job");
                        }
                    }
                }
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    if let Ok(Some(detail)) = stream_state.db.get_job_detail(session.id, id).await {
                        let terminal = detail.job.status.is_terminal();
                        yield Ok(Event::default().event("job.snapshot").json_data(&detail).unwrap_or_else(|_| Event::default()));
                        if terminal {
                            break;
                        }
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

#[utoipa::path(
    post,
    path = "/api/v1/jobs/{id}/cancel",
    tag = "jobs",
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Job), (status = 404, body = ProblemDetails)),
    security(("bearer_auth" = []))
)]
async fn cancel_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Job>> {
    let session = authenticate(&state, &headers).await?;
    let job = state
        .db
        .request_cancel(session.id, id, state.config.artifact_retention)
        .await
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::NotFound("Trabajo no encontrado.".to_owned()))?;
    if !job.view.status.can_cancel()
        && !matches!(
            job.view.status,
            JobStatus::CancelRequested | JobStatus::Cancelled
        )
    {
        return Err(AppError::Conflict(
            "El trabajo ya terminó y no puede cancelarse.".to_owned(),
        ));
    }
    Ok(Json(job.view))
}

#[utoipa::path(
    post,
    path = "/api/v1/jobs/{id}/retry",
    tag = "jobs",
    params(("id" = Uuid, Path)),
    responses((status = 202, body = Job), (status = 409, body = ProblemDetails)),
    security(("bearer_auth" = []))
)]
async fn retry_job(
    State(state): State<AppState>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> AppResult<impl IntoResponse> {
    let session = authenticate(&state, &headers).await?;
    let original = state
        .db
        .get_job_for_session(session.id, id)
        .await
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::NotFound("Trabajo no encontrado.".to_owned()))?;
    if original.view.status != JobStatus::DeadLettered {
        return Err(AppError::Conflict(
            "Sólo se puede reintentar un trabajo fallido definitivamente.".to_owned(),
        ));
    }
    let client_ip_hash = client_ip_hash(&state, &headers, address);
    enforce_rate_limits(&state, session.id, &client_ip_hash).await?;
    let bytes = state
        .storage
        .get(&original.input_object_key)
        .await
        .map_err(|error| {
            if matches!(error, object_store::Error::NotFound { .. }) {
                AppError::Conflict("El archivo original ya expiró.".to_owned())
            } else {
                AppError::Storage(error)
            }
        })?;
    let new_id = Uuid::now_v7();
    let object_key = format!("sessions/{}/jobs/{new_id}/input", session.id);
    state.storage.put(&object_key, bytes).await?;
    let new_job = match state
        .db
        .create_job(
            NewJob {
                id: new_id,
                session_id: session.id,
                kind: original.view.kind,
                original_name: &original.view.original_name,
                input_object_key: &object_key,
                input_content_type: &original.view.input_content_type,
                input_size: original.view.input_size,
                input_sha256: &original.input_sha256,
                idempotency_key: None,
                client_ip_hash: &client_ip_hash,
                retry_of_job_id: Some(id),
                metadata_retention: state.config.metadata_retention,
            },
            Some(upload_limits(&state)),
        )
        .await
    {
        Ok(CreateJobOutcome::Created(job)) => job,
        Ok(CreateJobOutcome::Existing(existing)) => {
            let _ = state.storage.delete(&object_key).await;
            existing
        }
        Ok(CreateJobOutcome::LimitExceeded(limit)) => {
            let _ = state.storage.delete(&object_key).await;
            return Err(rate_limit_error(&state, limit));
        }
        Err(error) => {
            let _ = state.storage.delete(&object_key).await;
            return Err(AppError::Internal(error));
        }
    };
    metrics::counter!("forgequeue_jobs_created_total", "kind" => new_job.view.kind.as_str())
        .increment(1);
    Ok((StatusCode::ACCEPTED, Json(new_job.view)))
}

#[utoipa::path(
    delete,
    path = "/api/v1/jobs/{id}",
    tag = "jobs",
    params(("id" = Uuid, Path)),
    responses((status = 204), (status = 409, body = ProblemDetails)),
    security(("bearer_auth" = []))
)]
async fn delete_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> AppResult<StatusCode> {
    let session = authenticate(&state, &headers).await?;
    let job = state
        .db
        .get_job_for_session(session.id, id)
        .await
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::NotFound("Trabajo no encontrado.".to_owned()))?;
    if !job.view.status.is_terminal() {
        return Err(AppError::Conflict(
            "Cancelá el trabajo antes de eliminarlo.".to_owned(),
        ));
    }
    state
        .storage
        .delete_prefix(&format!("sessions/{}/jobs/{id}", session.id))
        .await?;
    state
        .db
        .delete_job(session.id, id)
        .await
        .map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/api/v1/jobs/{job_id}/outputs/{output_id}",
    tag = "jobs",
    params(("job_id" = Uuid, Path), ("output_id" = Uuid, Path)),
    responses((status = 200, description = "Artefacto"), (status = 404, body = ProblemDetails)),
    security(("bearer_auth" = []))
)]
async fn download_output(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((job_id, output_id)): Path<(Uuid, Uuid)>,
) -> AppResult<Response> {
    let session = authenticate(&state, &headers).await?;
    let output = state
        .db
        .output_for_session(session.id, job_id, output_id)
        .await
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::NotFound("Resultado no encontrado o expirado.".to_owned()))?;
    let bytes = state.storage.get(&output.object_key).await?;
    let disposition = format!("inline; filename=\"{}\"", output.view.name.replace('"', ""));
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, output.view.content_type)
        .header(CONTENT_DISPOSITION, disposition)
        .body(Body::from(bytes))
        .map_err(|error| AppError::Internal(error.into()))
}

#[utoipa::path(get, path = "/health/live", tag = "system", responses((status = 200)))]
async fn live() -> StatusCode {
    StatusCode::OK
}

#[utoipa::path(get, path = "/health/ready", tag = "system", responses((status = 200), (status = 503)))]
async fn ready(State(state): State<AppState>) -> StatusCode {
    if state.db.ready().await {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

#[utoipa::path(get, path = "/metrics", tag = "system", responses((status = 200)))]
async fn metrics_endpoint(State(state): State<AppState>) -> impl IntoResponse {
    (
        [(CONTENT_TYPE, "text/plain; version=0.0.4")],
        state.metrics.render(),
    )
}

async fn openapi_document() -> impl IntoResponse {
    ([(CONTENT_TYPE, "application/json")], openapi_json())
}

async fn docs() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html lang="es"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>ForgeQueue API</title></head>
<body><script id="api-reference" data-url="/api/openapi.json"></script><script src="https://cdn.jsdelivr.net/npm/@scalar/api-reference"></script></body></html>"#,
    )
}

pub async fn pg_event_bridge(
    db: Database,
    events: broadcast::Sender<Uuid>,
    shutdown: CancellationToken,
) -> anyhow::Result<()> {
    loop {
        if shutdown.is_cancelled() {
            return Ok(());
        }
        let mut listener = match PgListener::connect_with(db.pool()).await {
            Ok(listener) => listener,
            Err(error) => {
                tracing::warn!(%error, "failed to connect PostgreSQL listener");
                tokio::select! {
                    _ = shutdown.cancelled() => return Ok(()),
                    _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {}
                }
                continue;
            }
        };
        if let Err(error) = listener.listen("forgequeue_job_events").await {
            tracing::warn!(%error, "failed to subscribe PostgreSQL listener");
            tokio::select! {
                _ = shutdown.cancelled() => return Ok(()),
                _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {}
            }
            continue;
        }
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => return Ok(()),
                notification = listener.recv() => match notification {
                    Ok(notification) => {
                        if let Ok(id) = Uuid::parse_str(notification.payload()) {
                            let _ = events.send(id);
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "PostgreSQL listener disconnected");
                        break;
                    }
                }
            }
        }
    }
}

struct Upload {
    filename: String,
    content_type: String,
    kind: JobKind,
    bytes: Bytes,
    sha256: String,
}

async fn read_upload(state: &AppState, multipart: &mut Multipart) -> AppResult<Upload> {
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| AppError::Validation(format!("Formulario inválido: {error}")))?
    {
        if field.name() != Some("file") {
            continue;
        }
        let filename = sanitize_filename(field.file_name().unwrap_or("archivo"));
        let bytes = field.bytes().await.map_err(|error| {
            AppError::Validation(format!("No se pudo leer el archivo: {error}"))
        })?;
        if bytes.is_empty() {
            return Err(AppError::Validation("El archivo está vacío.".to_owned()));
        }
        if bytes.len() > state.config.max_upload_bytes {
            return Err(AppError::Validation(format!(
                "El archivo supera el máximo de {} MiB.",
                state.config.max_upload_bytes / 1024 / 1024
            )));
        }
        let (kind, content_type) = detect_upload_kind(
            &bytes,
            state.config.max_image_pixels,
            state.config.max_pdf_pages,
        )?;
        return Ok(Upload {
            filename,
            content_type,
            kind,
            sha256: hex_sha256(&bytes),
            bytes,
        });
    }
    Err(AppError::Validation(
        "El campo multipart 'file' es obligatorio.".to_owned(),
    ))
}

fn detect_upload_kind(
    bytes: &[u8],
    max_image_pixels: u64,
    max_pdf_pages: usize,
) -> AppResult<(JobKind, String)> {
    let detected = infer::get(bytes)
        .ok_or_else(|| AppError::Validation("Formato de archivo desconocido.".to_owned()))?;
    match detected.mime_type() {
        "image/jpeg" | "image/png" | "image/webp" => {
            validate_image(bytes, max_image_pixels)?;
            Ok((JobKind::Image, detected.mime_type().to_owned()))
        }
        "application/pdf" => {
            validate_pdf_page_count(bytes, max_pdf_pages)
                .map_err(|error| AppError::Validation(format!("PDF inválido: {error}")))?;
            Ok((JobKind::Pdf, detected.mime_type().to_owned()))
        }
        other => Err(AppError::Validation(format!(
            "El formato {other} no está permitido."
        ))),
    }
}

fn validate_image(bytes: &[u8], max_pixels: u64) -> AppResult<()> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| AppError::Validation(format!("Imagen inválida: {error}")))?;
    let (width, height) = reader
        .into_dimensions()
        .map_err(|error| AppError::Validation(format!("Imagen inválida: {error}")))?;
    let pixels = u64::from(width) * u64::from(height);
    if pixels > max_pixels {
        return Err(AppError::Validation(format!(
            "La imagen tiene {pixels} píxeles; el máximo es {max_pixels}."
        )));
    }
    Ok(())
}

async fn authenticate(state: &AppState, headers: &HeaderMap) -> AppResult<Session> {
    let token = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| value.starts_with("fq_"))
        .ok_or_else(|| AppError::Unauthorized("Falta la sesión anónima.".to_owned()))?;
    state
        .db
        .authenticate_session(&sha256(token.as_bytes()))
        .await
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::Unauthorized("La sesión expiró o no es válida.".to_owned()))
}

async fn job_detail(state: &AppState, session_id: Uuid, id: Uuid) -> AppResult<JobDetail> {
    state
        .db
        .get_job_detail(session_id, id)
        .await
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::NotFound("Trabajo no encontrado.".to_owned()))
}

async fn enforce_rate_limits(state: &AppState, session_id: Uuid, ip_hash: &[u8]) -> AppResult<()> {
    let (session, ip, global) = state
        .db
        .upload_counts(session_id, ip_hash)
        .await
        .map_err(AppError::Internal)?;
    if session >= state.config.session_hourly_limit {
        return Err(rate_limit_error(state, UploadLimit::SessionHourly));
    }
    if ip >= state.config.ip_daily_limit {
        return Err(rate_limit_error(state, UploadLimit::IpDaily));
    }
    if global >= state.config.global_daily_limit {
        return Err(rate_limit_error(state, UploadLimit::GlobalDaily));
    }
    Ok(())
}

fn upload_limits(state: &AppState) -> UploadLimits {
    UploadLimits {
        session_hourly: state.config.session_hourly_limit,
        ip_daily: state.config.ip_daily_limit,
        global_daily: state.config.global_daily_limit,
    }
}

fn rate_limit_error(state: &AppState, limit: UploadLimit) -> AppError {
    let detail = match limit {
        UploadLimit::SessionHourly => format!(
            "Alcanzaste el límite de {} trabajos por hora.",
            state.config.session_hourly_limit
        ),
        UploadLimit::IpDaily => "Esta dirección alcanzó el límite diario.".to_owned(),
        UploadLimit::GlobalDaily => "La demo alcanzó su capacidad diaria; probá mañana.".to_owned(),
    };
    AppError::RateLimited(detail)
}

fn client_ip_hash(state: &AppState, headers: &HeaderMap, address: SocketAddr) -> Vec<u8> {
    let client_ip = state
        .config
        .trust_proxy_headers
        .then(|| {
            headers
                .get("x-forwarded-for")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.split(',').next())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        })
        .flatten()
        .unwrap_or_else(|| address.ip().to_string());
    sha256(format!("{}:{client_ip}", state.config.rate_limit_salt).as_bytes())
}

fn idempotency_key(headers: &HeaderMap) -> AppResult<Option<String>> {
    let Some(value) = headers.get("idempotency-key") else {
        return Ok(None);
    };
    let value = value
        .to_str()
        .map_err(|_| AppError::Validation("Idempotency-Key inválida.".to_owned()))?
        .trim();
    if value.is_empty() || value.len() > 128 {
        return Err(AppError::Validation(
            "Idempotency-Key debe tener entre 1 y 128 caracteres.".to_owned(),
        ));
    }
    Ok(Some(value.to_owned()))
}

fn sanitize_filename(value: &str) -> String {
    let filename = value
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("archivo")
        .chars()
        .filter(|character| !character.is_control())
        .take(120)
        .collect::<String>();
    if filename.trim().is_empty() {
        "archivo".to_owned()
    } else {
        filename
    }
}

fn sha256(bytes: &[u8]) -> Vec<u8> {
    Sha256::digest(bytes).to_vec()
}

fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{detect_upload_kind, idempotency_key, sanitize_filename, validate_image};
    use axum::http::{HeaderMap, HeaderValue};
    use image::{DynamicImage, ImageFormat, RgbImage};

    #[test]
    fn strips_path_from_uploaded_filename() {
        assert_eq!(sanitize_filename("C:\\fakepath\\report.pdf"), "report.pdf");
        assert_eq!(sanitize_filename("../../photo.png"), "photo.png");
    }

    #[test]
    fn validates_idempotency_key_length() {
        let mut headers = HeaderMap::new();
        headers.insert("idempotency-key", HeaderValue::from_static("upload-123"));
        assert_eq!(
            idempotency_key(&headers).unwrap().as_deref(),
            Some("upload-123")
        );
    }

    #[test]
    fn image_pixel_limit_is_checked_before_full_decode() {
        let image = DynamicImage::ImageRgb8(RgbImage::new(2, 2));
        let mut encoded = Cursor::new(Vec::new());
        image.write_to(&mut encoded, ImageFormat::Png).unwrap();
        assert!(validate_image(encoded.get_ref(), 4).is_ok());
        assert!(validate_image(encoded.get_ref(), 3).is_err());
        let (kind, content_type) = detect_upload_kind(encoded.get_ref(), 4, 20).unwrap();
        assert_eq!(kind, forgequeue_core::JobKind::Image);
        assert_eq!(content_type, "image/png");
        assert!(detect_upload_kind(b"not really a png", 4, 20).is_err());
    }
}
