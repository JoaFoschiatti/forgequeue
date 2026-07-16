use std::{sync::Arc, time::Instant};

use anyhow::Result;
use tokio_util::sync::CancellationToken;
use tracing::{Instrument, info_span};

use crate::{
    config::Config,
    db::{ClaimedJob, Database, FinishFailure, NewOutput},
    processors::{ProcessingError, ProcessorContext},
    storage::BlobStore,
};

#[derive(Clone)]
pub struct Worker {
    config: Arc<Config>,
    db: Database,
    storage: BlobStore,
    processors: ProcessorContext,
}

impl Worker {
    pub fn new(config: Arc<Config>, db: Database, storage: BlobStore) -> Self {
        let processors = ProcessorContext::new(config.clone(), db.clone(), storage.clone());
        Self {
            config,
            db,
            storage,
            processors,
        }
    }

    pub async fn run(self, shutdown: CancellationToken) -> Result<()> {
        tracing::info!(worker_id = %self.config.worker_id, "worker started");
        let recovery_interval = std::time::Duration::from_secs(10);
        let mut last_recovery = Instant::now() - recovery_interval;

        loop {
            if shutdown.is_cancelled() {
                break;
            }
            if last_recovery.elapsed() >= recovery_interval {
                let recovered = self
                    .db
                    .recover_expired_leases(self.config.artifact_retention)
                    .await?;
                if !recovered.is_empty() {
                    metrics::counter!("forgequeue_leases_recovered_total")
                        .increment(recovered.len() as u64);
                    tracing::warn!(count = recovered.len(), "recovered expired job leases");
                }
                last_recovery = Instant::now();
            }

            if let Some(claimed) = self
                .db
                .claim_next(&self.config.worker_id, self.config.lease_duration)
                .await?
            {
                let span = info_span!(
                    "process_job",
                    job_id = %claimed.job.view.id,
                    attempt_id = %claimed.attempt_id,
                    kind = %claimed.job.view.kind,
                );
                self.process_claimed(claimed).instrument(span).await?;
                continue;
            }

            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = tokio::time::sleep(self.config.poll_interval) => {}
            }
        }
        tracing::info!(worker_id = %self.config.worker_id, "worker stopped");
        Ok(())
    }

    async fn process_claimed(&self, claimed: ClaimedJob) -> Result<()> {
        let started = Instant::now();
        let job_id = claimed.job.view.id;
        let heartbeat_stop = CancellationToken::new();
        let heartbeat_handle = {
            let db = self.db.clone();
            let attempt_id = claimed.attempt_id;
            let worker_id = self.config.worker_id.clone();
            let interval = self.config.heartbeat_interval;
            let lease = self.config.lease_duration;
            let stop = heartbeat_stop.clone();
            tokio::spawn(async move {
                let mut ticker = tokio::time::interval(interval);
                loop {
                    tokio::select! {
                        _ = stop.cancelled() => break,
                        _ = ticker.tick() => {
                            match db.heartbeat(job_id, attempt_id, &worker_id, lease).await {
                                Ok(true) => {}
                                Ok(false) => break,
                                Err(error) => tracing::warn!(%error, %job_id, "heartbeat failed"),
                            }
                        }
                    }
                }
            })
        };

        let result = tokio::time::timeout(
            self.config.processing_timeout,
            self.processors.process(
                job_id,
                claimed.attempt_id,
                &self.config.worker_id,
                claimed.job.view.kind,
                &claimed.job.input_object_key,
                &claimed.job.input_sha256,
            ),
        )
        .await;
        let restart_after_timeout = result.is_err();

        let finalization_result = async {
            match result {
            Ok(Ok(artifacts)) => {
                if self.db.is_cancel_requested(job_id).await? {
                    let finalized = self
                        .db
                        .finish_cancelled(
                            job_id,
                            claimed.attempt_id,
                            &self.config.worker_id,
                            self.config.artifact_retention,
                        )
                        .await?;
                    if !finalized {
                        tracing::warn!(%job_id, "ignored stale cancellation result");
                    }
                } else {
                    let owns_lease = self
                        .db
                        .update_progress(
                            job_id,
                            claimed.attempt_id,
                            &self.config.worker_id,
                            "storing_outputs",
                            85,
                        )
                        .await?;
                    if !owns_lease {
                        if self.db.is_cancel_requested(job_id).await? {
                            let _ = self
                                .db
                                .finish_cancelled(
                                    job_id,
                                    claimed.attempt_id,
                                    &self.config.worker_id,
                                    self.config.artifact_retention,
                                )
                                .await?;
                        } else {
                            tracing::warn!(%job_id, "discarded outputs after lease loss");
                        }
                        return Ok(());
                    }
                    for artifact in artifacts {
                        let object_key = format!(
                            "sessions/{}/jobs/{}/outputs/{}",
                            claimed.job.session_id, job_id, artifact.name
                        );
                        let size = artifact.bytes.len() as i64;
                        self.storage.put(&object_key, artifact.bytes).await?;
                        let stored = self
                            .db
                            .upsert_output(
                                NewOutput {
                                    job_id,
                                    name: &artifact.name,
                                    object_key: &object_key,
                                    content_type: artifact.content_type,
                                    size,
                                    width: artifact.width,
                                    height: artifact.height,
                                    page_number: artifact.page_number,
                                },
                                claimed.attempt_id,
                                &self.config.worker_id,
                            )
                            .await?;
                        if stored.is_none() {
                            if self.db.is_cancel_requested(job_id).await? {
                                let _ = self
                                    .db
                                    .finish_cancelled(
                                        job_id,
                                        claimed.attempt_id,
                                        &self.config.worker_id,
                                        self.config.artifact_retention,
                                    )
                                    .await?;
                            } else {
                                tracing::warn!(%job_id, "discarded an output after lease loss");
                            }
                            return Ok(());
                        }
                    }
                    let finalized = self
                        .db
                        .finish_success(
                            job_id,
                            claimed.attempt_id,
                            &self.config.worker_id,
                            self.config.artifact_retention,
                        )
                        .await?;
                    if finalized {
                        metrics::counter!("forgequeue_jobs_completed_total", "kind" => claimed.job.view.kind.as_str())
                            .increment(1);
                    } else if self.db.is_cancel_requested(job_id).await? {
                        let cancelled = self
                            .db
                            .finish_cancelled(
                                job_id,
                                claimed.attempt_id,
                                &self.config.worker_id,
                                self.config.artifact_retention,
                            )
                            .await?;
                        if !cancelled {
                            tracing::warn!(%job_id, "ignored stale success after cancellation");
                        }
                    } else {
                        tracing::warn!(%job_id, "ignored stale success after lease loss");
                    }
                }
            }
            Ok(Err(ProcessingError::Cancelled)) => {
                let finalized = self
                    .db
                    .finish_cancelled(
                        job_id,
                        claimed.attempt_id,
                        &self.config.worker_id,
                        self.config.artifact_retention,
                    )
                    .await?;
                if !finalized {
                    tracing::warn!(%job_id, "ignored stale cancellation result");
                }
            }
            Ok(Err(ProcessingError::LeaseLost)) => {
                if self.db.is_cancel_requested(job_id).await? {
                    let _ = self
                        .db
                        .finish_cancelled(
                            job_id,
                            claimed.attempt_id,
                            &self.config.worker_id,
                            self.config.artifact_retention,
                        )
                        .await?;
                } else {
                    tracing::warn!(%job_id, "stopped processing after lease loss");
                }
            }
            Ok(Err(error)) => {
                let status = self
                    .db
                    .finish_failure(FinishFailure {
                        job_id,
                        attempt_id: claimed.attempt_id,
                        worker_id: &self.config.worker_id,
                        code: error.code(),
                        detail: &error.to_string(),
                        retryable: error.retryable(),
                        artifact_retention: self.config.artifact_retention,
                    })
                    .await?;
                if let Some(status) = status {
                    metrics::counter!("forgequeue_jobs_failed_total", "status" => status.as_str())
                        .increment(1);
                } else if self.db.is_cancel_requested(job_id).await? {
                    let _ = self
                        .db
                        .finish_cancelled(
                            job_id,
                            claimed.attempt_id,
                            &self.config.worker_id,
                            self.config.artifact_retention,
                        )
                        .await?;
                } else {
                    tracing::warn!(%job_id, "ignored stale failure after lease loss");
                }
            }
            Err(_) => {
                let error = ProcessingError::Temporary(format!(
                    "El procesamiento superó {} segundos.",
                    self.config.processing_timeout.as_secs()
                ));
                if self
                    .db
                    .finish_failure(FinishFailure {
                        job_id,
                        attempt_id: claimed.attempt_id,
                        worker_id: &self.config.worker_id,
                        code: "processing_timeout",
                        detail: &error.to_string(),
                        retryable: true,
                        artifact_retention: self.config.artifact_retention,
                    })
                    .await?
                    .is_none()
                {
                    if self.db.is_cancel_requested(job_id).await? {
                        let _ = self
                            .db
                            .finish_cancelled(
                                job_id,
                                claimed.attempt_id,
                                &self.config.worker_id,
                                self.config.artifact_retention,
                            )
                            .await?;
                    } else {
                        tracing::warn!(%job_id, "ignored stale timeout after lease loss");
                    }
                }
            }
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;

        heartbeat_stop.cancel();
        let _ = heartbeat_handle.await;
        finalization_result?;

        metrics::histogram!("forgequeue_job_duration_seconds", "kind" => claimed.job.view.kind.as_str())
            .record(started.elapsed().as_secs_f64());
        if restart_after_timeout {
            anyhow::bail!(
                "processing timed out; worker must restart to terminate blocking native work"
            );
        }
        Ok(())
    }
}

pub async fn cleanup_once(db: &Database, storage: &BlobStore) -> Result<(usize, u64)> {
    let candidates = db.cleanup_candidates(100).await?;
    for candidate in &candidates {
        storage.delete_prefix(&candidate.object_prefix).await?;
        db.mark_artifacts_deleted(candidate.job_id).await?;
    }
    let purged = db.purge_expired_metadata().await?;
    if !candidates.is_empty() || purged > 0 {
        tracing::info!(
            artifacts = candidates.len(),
            metadata = purged,
            "cleanup completed"
        );
    }
    Ok((candidates.len(), purged))
}

pub async fn cleanup_loop(
    db: Database,
    storage: BlobStore,
    shutdown: CancellationToken,
) -> Result<()> {
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(300));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            _ = ticker.tick() => {
                if let Err(error) = cleanup_once(&db, &storage).await {
                    tracing::warn!(%error, "cleanup pass failed");
                }
            }
        }
    }
    Ok(())
}
