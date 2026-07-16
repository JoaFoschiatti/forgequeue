use std::{str::FromStr, time::Duration};

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use forgequeue_core::{
    Job, JobAttempt, JobDetail, JobKind, JobOutput, JobPage, JobStatus, retry_delay,
};
use sqlx::{FromRow, PgPool, Postgres, Transaction, postgres::PgPoolOptions};
use uuid::Uuid;

const MIGRATION_LOCK_ID: i64 = 7_271_703_117;
const UPLOAD_LIMIT_LOCK_ID: i64 = 7_271_703_118;

#[derive(Clone)]
pub struct Database {
    pool: PgPool,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub id: Uuid,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct StoredJob {
    pub view: Job,
    pub session_id: Uuid,
    pub input_object_key: String,
    pub input_sha256: String,
}

#[derive(Debug, Clone)]
pub struct ClaimedJob {
    pub job: StoredJob,
    pub attempt_id: Uuid,
}

#[derive(Debug, Clone)]
pub struct StoredOutput {
    pub view: JobOutput,
    pub object_key: String,
}

#[derive(Debug, Clone)]
pub struct CleanupCandidate {
    pub job_id: Uuid,
    pub object_prefix: String,
}

#[derive(Debug, Clone)]
pub struct NewJob<'a> {
    pub id: Uuid,
    pub session_id: Uuid,
    pub kind: JobKind,
    pub original_name: &'a str,
    pub input_object_key: &'a str,
    pub input_content_type: &'a str,
    pub input_size: i64,
    pub input_sha256: &'a str,
    pub idempotency_key: Option<&'a str>,
    pub client_ip_hash: &'a [u8],
    pub retry_of_job_id: Option<Uuid>,
    pub metadata_retention: Duration,
}

#[derive(Debug, Clone)]
pub struct NewOutput<'a> {
    pub job_id: Uuid,
    pub name: &'a str,
    pub object_key: &'a str,
    pub content_type: &'a str,
    pub size: i64,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub page_number: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct FinishFailure<'a> {
    pub job_id: Uuid,
    pub attempt_id: Uuid,
    pub worker_id: &'a str,
    pub code: &'a str,
    pub detail: &'a str,
    pub retryable: bool,
    pub artifact_retention: Duration,
}

#[derive(Debug, Clone, Copy)]
pub struct UploadLimits {
    pub session_hourly: i64,
    pub ip_daily: i64,
    pub global_daily: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadLimit {
    SessionHourly,
    IpDaily,
    GlobalDaily,
}

#[derive(Debug, Clone)]
pub enum CreateJobOutcome {
    Created(StoredJob),
    Existing(StoredJob),
    LimitExceeded(UploadLimit),
}

#[derive(Debug, Clone, FromRow)]
struct JobRow {
    id: Uuid,
    session_id: Uuid,
    kind: String,
    status: String,
    progress: i16,
    stage: String,
    original_name: String,
    input_object_key: String,
    input_content_type: String,
    input_size: i64,
    input_sha256: String,
    attempt_count: i32,
    max_attempts: i32,
    last_error_code: Option<String>,
    last_error_detail: Option<String>,
    retry_of_job_id: Option<Uuid>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    completed_at: Option<DateTime<Utc>>,
    artifacts_expire_at: Option<DateTime<Utc>>,
}

#[derive(Debug, FromRow)]
struct AttemptRow {
    id: Uuid,
    number: i32,
    worker_id: String,
    status: String,
    error_code: Option<String>,
    error_detail: Option<String>,
    started_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, FromRow)]
struct OutputRow {
    id: Uuid,
    name: String,
    object_key: String,
    content_type: String,
    size: i64,
    width: Option<i32>,
    height: Option<i32>,
    page_number: Option<i32>,
    created_at: DateTime<Utc>,
}

impl Database {
    pub async fn connect(database_url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(12)
            .acquire_timeout(Duration::from_secs(10))
            .connect(database_url)
            .await
            .context("failed to connect to PostgreSQL")?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn migrate(&self) -> Result<()> {
        let mut lock = self.pool.acquire().await?;
        sqlx::query("SELECT pg_advisory_lock($1)")
            .bind(MIGRATION_LOCK_ID)
            .execute(&mut *lock)
            .await?;
        let result = sqlx::migrate!("../../migrations").run(&self.pool).await;
        sqlx::query("SELECT pg_advisory_unlock($1)")
            .bind(MIGRATION_LOCK_ID)
            .execute(&mut *lock)
            .await?;
        result.context("database migration failed")?;
        Ok(())
    }

    pub async fn ready(&self) -> bool {
        sqlx::query_scalar::<_, i32>("SELECT 1")
            .fetch_one(&self.pool)
            .await
            .is_ok()
    }

    pub async fn create_session(&self, token_hash: &[u8], lifetime: Duration) -> Result<Session> {
        let id = Uuid::now_v7();
        let expires_at = Utc::now()
            + chrono::Duration::from_std(lifetime).context("invalid session lifetime")?;
        sqlx::query(
            "INSERT INTO anonymous_sessions (id, token_hash, expires_at) VALUES ($1, $2, $3)",
        )
        .bind(id)
        .bind(token_hash)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;
        Ok(Session { id, expires_at })
    }

    pub async fn authenticate_session(&self, token_hash: &[u8]) -> Result<Option<Session>> {
        let row = sqlx::query_as::<_, (Uuid, DateTime<Utc>)>(
            "SELECT id, expires_at FROM anonymous_sessions \
             WHERE token_hash = $1 AND expires_at > now()",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(id, expires_at)| Session { id, expires_at }))
    }

    pub async fn upload_counts(&self, session_id: Uuid, ip_hash: &[u8]) -> Result<(i64, i64, i64)> {
        let session_count = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM upload_events WHERE session_id = $1 \
             AND created_at >= now() - interval '1 hour'",
        )
        .bind(session_id)
        .fetch_one(&self.pool)
        .await?;
        let ip_count = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM upload_events WHERE client_ip_hash = $1 \
             AND created_at >= now() - interval '24 hours'",
        )
        .bind(ip_hash)
        .fetch_one(&self.pool)
        .await?;
        let global_count = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM upload_events WHERE created_at >= now() - interval '24 hours'",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok((session_count, ip_count, global_count))
    }

    pub async fn find_idempotent_job(
        &self,
        session_id: Uuid,
        key: &str,
    ) -> Result<Option<StoredJob>> {
        let row = sqlx::query_as::<_, JobRow>(
            "SELECT * FROM jobs WHERE session_id = $1 AND idempotency_key = $2",
        )
        .bind(session_id)
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;
        row.map(StoredJob::try_from).transpose()
    }

    pub async fn create_job(
        &self,
        input: NewJob<'_>,
        limits: Option<UploadLimits>,
    ) -> Result<CreateJobOutcome> {
        let mut transaction = self.pool.begin().await?;
        if limits.is_some() {
            sqlx::query("SELECT pg_advisory_xact_lock($1)")
                .bind(UPLOAD_LIMIT_LOCK_ID)
                .execute(&mut *transaction)
                .await?;
        }

        if let Some(key) = input.idempotency_key
            && let Some(row) = sqlx::query_as::<_, JobRow>(
                "SELECT * FROM jobs WHERE session_id = $1 AND idempotency_key = $2",
            )
            .bind(input.session_id)
            .bind(key)
            .fetch_optional(&mut *transaction)
            .await?
        {
            transaction.rollback().await?;
            return Ok(CreateJobOutcome::Existing(StoredJob::try_from(row)?));
        }

        if let Some(limits) = limits {
            let session_count = sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM upload_events WHERE session_id = $1
                 AND created_at >= now() - interval '1 hour'",
            )
            .bind(input.session_id)
            .fetch_one(&mut *transaction)
            .await?;
            if session_count >= limits.session_hourly {
                transaction.rollback().await?;
                return Ok(CreateJobOutcome::LimitExceeded(UploadLimit::SessionHourly));
            }

            let ip_count = sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM upload_events WHERE client_ip_hash = $1
                 AND created_at >= now() - interval '24 hours'",
            )
            .bind(input.client_ip_hash)
            .fetch_one(&mut *transaction)
            .await?;
            if ip_count >= limits.ip_daily {
                transaction.rollback().await?;
                return Ok(CreateJobOutcome::LimitExceeded(UploadLimit::IpDaily));
            }

            let global_count = sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM upload_events WHERE created_at >= now() - interval '24 hours'",
            )
            .fetch_one(&mut *transaction)
            .await?;
            if global_count >= limits.global_daily {
                transaction.rollback().await?;
                return Ok(CreateJobOutcome::LimitExceeded(UploadLimit::GlobalDaily));
            }
        }

        let row = insert_job_row(&mut transaction, &input).await?;
        let stored = StoredJob::try_from(row)?;
        transaction.commit().await?;
        self.notify(input.id).await?;
        Ok(CreateJobOutcome::Created(stored))
    }

    pub async fn get_job_for_session(
        &self,
        session_id: Uuid,
        id: Uuid,
    ) -> Result<Option<StoredJob>> {
        let row =
            sqlx::query_as::<_, JobRow>("SELECT * FROM jobs WHERE id = $1 AND session_id = $2")
                .bind(id)
                .bind(session_id)
                .fetch_optional(&self.pool)
                .await?;
        row.map(StoredJob::try_from).transpose()
    }

    pub async fn get_job_detail(&self, session_id: Uuid, id: Uuid) -> Result<Option<JobDetail>> {
        let Some(stored) = self.get_job_for_session(session_id, id).await? else {
            return Ok(None);
        };
        let attempts = sqlx::query_as::<_, AttemptRow>(
            "SELECT id, number, worker_id, status, error_code, error_detail, \
                    started_at, finished_at \
             FROM job_attempts WHERE job_id = $1 ORDER BY number",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(JobAttempt::from)
        .collect();
        let outputs = self
            .output_rows(id)
            .await?
            .into_iter()
            .map(|row| JobOutput::from(&row))
            .collect();
        Ok(Some(JobDetail {
            job: stored.view,
            attempts,
            outputs,
        }))
    }

    pub async fn list_jobs(
        &self,
        session_id: Uuid,
        status: Option<JobStatus>,
        kind: Option<JobKind>,
        cursor: Option<Uuid>,
        limit: i64,
    ) -> Result<JobPage> {
        let rows = sqlx::query_as::<_, JobRow>(
            "SELECT * FROM jobs j
             WHERE j.session_id = $1
               AND ($2::text IS NULL OR j.status = $2)
               AND ($3::text IS NULL OR j.kind = $3)
               AND ($4::uuid IS NULL OR (j.created_at, j.id) < (
                    SELECT c.created_at, c.id FROM jobs c WHERE c.id = $4 AND c.session_id = $1
               ))
             ORDER BY j.created_at DESC, j.id DESC
             LIMIT $5",
        )
        .bind(session_id)
        .bind(status.map(|value| value.as_str()))
        .bind(kind.map(|value| value.as_str()))
        .bind(cursor)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        let mut items = Vec::with_capacity(rows.len());
        for row in rows {
            items.push(StoredJob::try_from(row)?.view);
        }
        let next_cursor = (items.len() as i64 == limit)
            .then(|| items.last().map(|job| job.id))
            .flatten();
        Ok(JobPage { items, next_cursor })
    }

    pub async fn claim_next(
        &self,
        worker_id: &str,
        lease_duration: Duration,
    ) -> Result<Option<ClaimedJob>> {
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query_as::<_, JobRow>(
            "WITH candidate AS (
                SELECT id FROM jobs
                WHERE status IN ('queued', 'retry_scheduled')
                  AND available_at <= now()
                  AND attempt_count < max_attempts
                  AND metadata_expire_at > now()
                ORDER BY available_at, created_at
                FOR UPDATE SKIP LOCKED
                LIMIT 1
             )
             UPDATE jobs j
             SET status = 'running', stage = 'claimed', progress = GREATEST(progress, 5),
                 worker_id = $1, lease_until = now() + ($2 * interval '1 second'),
                 attempt_count = attempt_count + 1, updated_at = now(),
                 last_error_code = NULL, last_error_detail = NULL
             FROM candidate
             WHERE j.id = candidate.id
             RETURNING j.*",
        )
        .bind(worker_id)
        .bind(lease_duration.as_secs() as i64)
        .fetch_optional(&mut *transaction)
        .await?;

        let Some(row) = row else {
            transaction.rollback().await?;
            return Ok(None);
        };
        let attempt_id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO job_attempts (id, job_id, number, worker_id, status)
             VALUES ($1, $2, $3, $4, 'running')",
        )
        .bind(attempt_id)
        .bind(row.id)
        .bind(row.attempt_count)
        .bind(worker_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        self.notify(row.id).await?;
        Ok(Some(ClaimedJob {
            job: StoredJob::try_from(row)?,
            attempt_id,
        }))
    }

    pub async fn heartbeat(
        &self,
        job_id: Uuid,
        attempt_id: Uuid,
        worker_id: &str,
        lease_duration: Duration,
    ) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE jobs SET lease_until = now() + ($4 * interval '1 second'), updated_at = now()
             WHERE id = $1 AND worker_id = $3 AND status IN ('running', 'cancel_requested')
               AND lease_until > now()
               AND attempt_count = (
                    SELECT number FROM job_attempts
                    WHERE id = $2 AND job_id = $1 AND status = 'running'
               )",
        )
        .bind(job_id)
        .bind(attempt_id)
        .bind(worker_id)
        .bind(lease_duration.as_secs() as i64)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn is_cancel_requested(&self, job_id: Uuid) -> Result<bool> {
        let status = sqlx::query_scalar::<_, String>("SELECT status FROM jobs WHERE id = $1")
            .bind(job_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(matches!(
            status.as_deref(),
            Some("cancel_requested" | "cancelled")
        ))
    }

    pub async fn update_progress(
        &self,
        job_id: Uuid,
        attempt_id: Uuid,
        worker_id: &str,
        stage: &str,
        progress: u8,
    ) -> Result<bool> {
        let updated = sqlx::query(
            "UPDATE jobs SET stage = $4, progress = $5, updated_at = now()
             WHERE id = $1 AND status = 'running' AND worker_id = $3 AND lease_until > now()
               AND attempt_count = (
                    SELECT number FROM job_attempts
                    WHERE id = $2 AND job_id = $1 AND status = 'running'
               )",
        )
        .bind(job_id)
        .bind(attempt_id)
        .bind(worker_id)
        .bind(stage)
        .bind(i16::from(progress.min(99)))
        .execute(&self.pool)
        .await?;
        if updated.rows_affected() == 1 {
            self.notify(job_id).await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub async fn upsert_output(
        &self,
        output: NewOutput<'_>,
        attempt_id: Uuid,
        worker_id: &str,
    ) -> Result<Option<JobOutput>> {
        let mut transaction = self.pool.begin().await?;
        let owns_lease = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (
                SELECT 1 FROM jobs j
                JOIN job_attempts a ON a.job_id = j.id AND a.id = $2
                WHERE j.id = $1 AND j.status = 'running' AND j.worker_id = $3
                  AND j.lease_until > now() AND a.number = j.attempt_count
                  AND a.status = 'running'
                FOR UPDATE OF j
             )",
        )
        .bind(output.job_id)
        .bind(attempt_id)
        .bind(worker_id)
        .fetch_one(&mut *transaction)
        .await?;
        if !owns_lease {
            transaction.rollback().await?;
            return Ok(None);
        }
        let row = sqlx::query_as::<_, OutputRow>(
            "INSERT INTO job_outputs (
                id, job_id, name, object_key, content_type, size, width, height, page_number
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (job_id, name) DO UPDATE SET
                object_key = EXCLUDED.object_key,
                content_type = EXCLUDED.content_type,
                size = EXCLUDED.size,
                width = EXCLUDED.width,
                height = EXCLUDED.height,
                page_number = EXCLUDED.page_number,
                created_at = now()
             RETURNING *",
        )
        .bind(Uuid::now_v7())
        .bind(output.job_id)
        .bind(output.name)
        .bind(output.object_key)
        .bind(output.content_type)
        .bind(output.size)
        .bind(output.width)
        .bind(output.height)
        .bind(output.page_number)
        .fetch_one(&mut *transaction)
        .await?;
        transaction.commit().await?;
        self.notify(output.job_id).await?;
        Ok(Some(JobOutput::from(&row)))
    }

    pub async fn finish_success(
        &self,
        job_id: Uuid,
        attempt_id: Uuid,
        worker_id: &str,
        artifact_retention: Duration,
    ) -> Result<bool> {
        let mut transaction = self.pool.begin().await?;
        let updated = sqlx::query(
            "UPDATE jobs SET status = 'succeeded', stage = 'completed', progress = 100,
                 lease_until = NULL, worker_id = NULL, completed_at = now(), updated_at = now(),
                 artifacts_expire_at = now() + ($2 * interval '1 second')
             WHERE id = $1 AND status = 'running' AND worker_id = $3 AND lease_until > now()
               AND attempt_count = (
                    SELECT number FROM job_attempts
                    WHERE id = $4 AND job_id = $1 AND status = 'running'
               )",
        )
        .bind(job_id)
        .bind(artifact_retention.as_secs() as i64)
        .bind(worker_id)
        .bind(attempt_id)
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() == 0 {
            transaction.rollback().await?;
            return Ok(false);
        }
        sqlx::query(
            "UPDATE job_attempts SET status = 'succeeded', finished_at = now()
             WHERE id = $1 AND status = 'running'",
        )
        .bind(attempt_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        self.notify(job_id).await?;
        Ok(true)
    }

    pub async fn finish_cancelled(
        &self,
        job_id: Uuid,
        attempt_id: Uuid,
        worker_id: &str,
        artifact_retention: Duration,
    ) -> Result<bool> {
        let mut transaction = self.pool.begin().await?;
        let updated = sqlx::query(
            "UPDATE jobs SET status = 'cancelled', stage = 'cancelled', lease_until = NULL,
                 worker_id = NULL, completed_at = now(), updated_at = now(),
                 artifacts_expire_at = now() + ($2 * interval '1 second')
             WHERE id = $1 AND status = 'cancel_requested' AND worker_id = $3
               AND lease_until > now()
               AND attempt_count = (
                    SELECT number FROM job_attempts
                    WHERE id = $4 AND job_id = $1 AND status = 'running'
               )",
        )
        .bind(job_id)
        .bind(artifact_retention.as_secs() as i64)
        .bind(worker_id)
        .bind(attempt_id)
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() == 0 {
            transaction.rollback().await?;
            return Ok(false);
        }
        sqlx::query(
            "UPDATE job_attempts SET status = 'cancelled', finished_at = now()
             WHERE id = $1 AND status = 'running'",
        )
        .bind(attempt_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        self.notify(job_id).await?;
        Ok(true)
    }

    pub async fn finish_failure(&self, input: FinishFailure<'_>) -> Result<Option<JobStatus>> {
        let FinishFailure {
            job_id,
            attempt_id,
            worker_id,
            code,
            detail,
            retryable,
            artifact_retention,
        } = input;
        let mut transaction = self.pool.begin().await?;
        let ownership = sqlx::query_as::<_, (i32, i32)>(
            "SELECT j.attempt_count, j.max_attempts
             FROM jobs j
             JOIN job_attempts a ON a.job_id = j.id AND a.id = $2
             WHERE j.id = $1 AND j.status = 'running' AND j.worker_id = $3
               AND j.lease_until > now() AND a.number = j.attempt_count
               AND a.status = 'running'
             FOR UPDATE OF j",
        )
        .bind(job_id)
        .bind(attempt_id)
        .bind(worker_id)
        .fetch_optional(&mut *transaction)
        .await?;
        let Some((attempt_count, max_attempts)) = ownership else {
            transaction.rollback().await?;
            return Ok(None);
        };
        let next_delay = retry_delay(attempt_count);
        let status = if !retryable || attempt_count >= max_attempts || next_delay.is_none() {
            JobStatus::DeadLettered
        } else {
            JobStatus::RetryScheduled
        };
        let delay_seconds = next_delay.unwrap_or_default().as_secs() as i64;
        let terminal = status == JobStatus::DeadLettered;
        sqlx::query(
            "UPDATE jobs SET status = $2, stage = $3, last_error_code = $4,
                 last_error_detail = $5, lease_until = NULL, worker_id = NULL,
                 available_at = now() + ($6 * interval '1 second'), updated_at = now(),
                 completed_at = CASE WHEN $7 THEN now() ELSE NULL END,
                 artifacts_expire_at = CASE WHEN $7 THEN now() + ($8 * interval '1 second') ELSE NULL END
             WHERE id = $1",
        )
        .bind(job_id)
        .bind(status.as_str())
        .bind(if terminal { "dead_lettered" } else { "retry_wait" })
        .bind(code)
        .bind(detail)
        .bind(delay_seconds)
        .bind(terminal)
        .bind(artifact_retention.as_secs() as i64)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE job_attempts SET status = 'failed', error_code = $2,
                 error_detail = $3, finished_at = now() WHERE id = $1",
        )
        .bind(attempt_id)
        .bind(code)
        .bind(detail)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        self.notify(job_id).await?;
        Ok(Some(status))
    }

    pub async fn request_cancel(
        &self,
        session_id: Uuid,
        job_id: Uuid,
        artifact_retention: Duration,
    ) -> Result<Option<StoredJob>> {
        let row = sqlx::query_as::<_, JobRow>(
            "UPDATE jobs SET
                status = CASE
                    WHEN status IN ('queued', 'retry_scheduled') THEN 'cancelled'
                    WHEN status = 'running' THEN 'cancel_requested'
                    ELSE status
                END,
                stage = CASE
                    WHEN status IN ('queued', 'retry_scheduled') THEN 'cancelled'
                    WHEN status = 'running' THEN 'cancel_requested'
                    ELSE stage
                END,
                completed_at = CASE WHEN status IN ('queued', 'retry_scheduled') THEN now() ELSE completed_at END,
                artifacts_expire_at = CASE WHEN status IN ('queued', 'retry_scheduled')
                    THEN now() + ($3 * interval '1 second') ELSE artifacts_expire_at END,
                updated_at = now()
             WHERE id = $1 AND session_id = $2
             RETURNING *",
        )
        .bind(job_id)
        .bind(session_id)
        .bind(artifact_retention.as_secs() as i64)
        .fetch_optional(&self.pool)
        .await?;
        if row.is_some() {
            self.notify(job_id).await?;
        }
        row.map(StoredJob::try_from).transpose()
    }

    pub async fn recover_expired_leases(&self, artifact_retention: Duration) -> Result<Vec<Uuid>> {
        let retention_seconds = artifact_retention.as_secs() as i64;
        let mut transaction = self.pool.begin().await?;
        let cancelled = sqlx::query_scalar::<_, Uuid>(
            "UPDATE jobs SET status = 'cancelled', stage = 'cancelled', worker_id = NULL,
                 lease_until = NULL, completed_at = now(), updated_at = now(),
                 artifacts_expire_at = now() + ($1 * interval '1 second')
             WHERE status = 'cancel_requested' AND lease_until < now()
             RETURNING id",
        )
        .bind(retention_seconds)
        .fetch_all(&mut *transaction)
        .await?;
        let recovered = sqlx::query_scalar::<_, Uuid>(
            "UPDATE jobs SET
                status = CASE WHEN attempt_count >= max_attempts THEN 'dead_lettered' ELSE 'retry_scheduled' END,
                stage = CASE WHEN attempt_count >= max_attempts THEN 'dead_lettered' ELSE 'lease_recovered' END,
                available_at = now() + (CASE attempt_count
                    WHEN 1 THEN interval '5 seconds'
                    WHEN 2 THEN interval '30 seconds'
                    ELSE interval '0 seconds'
                END),
                worker_id = NULL, lease_until = NULL, updated_at = now(),
                last_error_code = 'worker_lost',
                last_error_detail = 'El worker dejó de renovar su lease.',
                completed_at = CASE WHEN attempt_count >= max_attempts THEN now() ELSE NULL END,
                artifacts_expire_at = CASE WHEN attempt_count >= max_attempts
                    THEN now() + ($1 * interval '1 second') ELSE NULL END
             WHERE status = 'running' AND lease_until < now()
             RETURNING id",
        )
        .bind(retention_seconds)
        .fetch_all(&mut *transaction)
        .await?;
        if !cancelled.is_empty() {
            sqlx::query(
                "UPDATE job_attempts SET status = 'cancelled', error_code = 'cancelled',
                     error_detail = 'El trabajo se canceló después de perder el worker.',
                     finished_at = now()
                 WHERE job_id = ANY($1) AND status = 'running'",
            )
            .bind(&cancelled)
            .execute(&mut *transaction)
            .await?;
        }
        if !recovered.is_empty() {
            sqlx::query(
                "UPDATE job_attempts SET status = 'failed', error_code = 'worker_lost',
                     error_detail = 'El worker dejó de renovar su lease.', finished_at = now()
                 WHERE job_id = ANY($1) AND status = 'running'",
            )
            .bind(&recovered)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        let mut ids = cancelled;
        ids.extend(recovered);
        for id in &ids {
            self.notify(*id).await?;
        }
        Ok(ids)
    }

    pub async fn output_for_session(
        &self,
        session_id: Uuid,
        job_id: Uuid,
        output_id: Uuid,
    ) -> Result<Option<StoredOutput>> {
        let row = sqlx::query_as::<_, OutputRow>(
            "SELECT o.* FROM job_outputs o
             JOIN jobs j ON j.id = o.job_id
             WHERE o.id = $1 AND o.job_id = $2 AND j.session_id = $3
               AND j.artifacts_deleted_at IS NULL",
        )
        .bind(output_id)
        .bind(job_id)
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(StoredOutput::from))
    }

    pub async fn delete_job(&self, session_id: Uuid, job_id: Uuid) -> Result<bool> {
        let deleted = sqlx::query(
            "DELETE FROM jobs WHERE id = $1 AND session_id = $2
             AND status IN ('succeeded', 'cancelled', 'dead_lettered', 'expired')",
        )
        .bind(job_id)
        .bind(session_id)
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(deleted == 1)
    }

    pub async fn cleanup_candidates(&self, limit: i64) -> Result<Vec<CleanupCandidate>> {
        let rows = sqlx::query_as::<_, (Uuid, Uuid)>(
            "SELECT j.id, j.session_id FROM jobs j
             WHERE j.artifacts_deleted_at IS NULL
               AND (
                    (j.artifacts_expire_at IS NOT NULL AND j.artifacts_expire_at <= now())
                    OR j.metadata_expire_at <= now()
               )
             ORDER BY COALESCE(j.artifacts_expire_at, j.metadata_expire_at)
             LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(job_id, session_id)| CleanupCandidate {
                job_id,
                object_prefix: format!("sessions/{session_id}/jobs/{job_id}"),
            })
            .collect())
    }

    pub async fn mark_artifacts_deleted(&self, job_id: Uuid) -> Result<()> {
        sqlx::query(
            "UPDATE jobs SET status = 'expired', stage = 'expired', artifacts_deleted_at = now(),
                 updated_at = now() WHERE id = $1",
        )
        .bind(job_id)
        .execute(&self.pool)
        .await?;
        self.notify(job_id).await
    }

    pub async fn purge_expired_metadata(&self) -> Result<u64> {
        let deleted_jobs = sqlx::query(
            "DELETE FROM jobs
             WHERE metadata_expire_at <= now()
               AND artifacts_deleted_at IS NOT NULL",
        )
        .execute(&self.pool)
        .await?
        .rows_affected();
        sqlx::query("DELETE FROM upload_events WHERE created_at < now() - interval '24 hours'")
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "DELETE FROM anonymous_sessions s
             WHERE s.expires_at <= now()
               AND NOT EXISTS (SELECT 1 FROM jobs j WHERE j.session_id = s.id)",
        )
        .execute(&self.pool)
        .await?;
        Ok(deleted_jobs)
    }

    pub async fn notify(&self, job_id: Uuid) -> Result<()> {
        sqlx::query("SELECT pg_notify('forgequeue_job_events', $1)")
            .bind(job_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn output_rows(&self, job_id: Uuid) -> Result<Vec<OutputRow>> {
        Ok(sqlx::query_as::<_, OutputRow>(
            "SELECT * FROM job_outputs WHERE job_id = $1 ORDER BY created_at, name",
        )
        .bind(job_id)
        .fetch_all(&self.pool)
        .await?)
    }
}

async fn insert_job_row(
    transaction: &mut Transaction<'_, Postgres>,
    input: &NewJob<'_>,
) -> Result<JobRow> {
    let retention_seconds = input.metadata_retention.as_secs() as i64;
    sqlx::query(
        "UPDATE anonymous_sessions
         SET expires_at = GREATEST(expires_at, now() + ($2 * interval '1 second'))
         WHERE id = $1",
    )
    .bind(input.session_id)
    .bind(retention_seconds)
    .execute(&mut **transaction)
    .await?;
    let row = sqlx::query_as::<_, JobRow>(
        "INSERT INTO jobs (
            id, session_id, kind, status, original_name, input_object_key,
            input_content_type, input_size, input_sha256, idempotency_key,
            client_ip_hash, retry_of_job_id, metadata_expire_at
         ) VALUES ($1, $2, $3, 'queued', $4, $5, $6, $7, $8, $9, $10, $11,
            now() + ($12 * interval '1 second'))
         RETURNING *",
    )
    .bind(input.id)
    .bind(input.session_id)
    .bind(input.kind.as_str())
    .bind(input.original_name)
    .bind(input.input_object_key)
    .bind(input.input_content_type)
    .bind(input.input_size)
    .bind(input.input_sha256)
    .bind(input.idempotency_key)
    .bind(input.client_ip_hash)
    .bind(input.retry_of_job_id)
    .bind(retention_seconds)
    .fetch_one(&mut **transaction)
    .await?;
    sqlx::query(
        "INSERT INTO upload_events (job_id, session_id, client_ip_hash, created_at)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(row.id)
    .bind(row.session_id)
    .bind(input.client_ip_hash)
    .bind(row.created_at)
    .execute(&mut **transaction)
    .await?;
    Ok(row)
}

impl TryFrom<JobRow> for StoredJob {
    type Error = anyhow::Error;

    fn try_from(row: JobRow) -> Result<Self> {
        let kind = JobKind::from_str(&row.kind).map_err(|error| anyhow!(error))?;
        let status = JobStatus::from_str(&row.status).map_err(|error| anyhow!(error))?;
        let progress = u8::try_from(row.progress).context("invalid stored progress")?;
        Ok(Self {
            view: Job {
                id: row.id,
                kind,
                status,
                progress,
                stage: row.stage,
                original_name: row.original_name,
                input_content_type: row.input_content_type,
                input_size: row.input_size,
                attempt_count: row.attempt_count,
                max_attempts: row.max_attempts,
                last_error_code: row.last_error_code,
                last_error_detail: row.last_error_detail,
                retry_of_job_id: row.retry_of_job_id,
                created_at: row.created_at,
                updated_at: row.updated_at,
                completed_at: row.completed_at,
                artifacts_expire_at: row.artifacts_expire_at,
            },
            session_id: row.session_id,
            input_object_key: row.input_object_key,
            input_sha256: row.input_sha256,
        })
    }
}

impl From<AttemptRow> for JobAttempt {
    fn from(row: AttemptRow) -> Self {
        Self {
            id: row.id,
            number: row.number,
            worker_id: row.worker_id,
            status: row.status,
            error_code: row.error_code,
            error_detail: row.error_detail,
            started_at: row.started_at,
            finished_at: row.finished_at,
        }
    }
}

impl From<&OutputRow> for JobOutput {
    fn from(row: &OutputRow) -> Self {
        Self {
            id: row.id,
            name: row.name.clone(),
            content_type: row.content_type.clone(),
            size: row.size,
            width: row.width,
            height: row.height,
            page_number: row.page_number,
            created_at: row.created_at,
        }
    }
}

impl From<OutputRow> for StoredOutput {
    fn from(row: OutputRow) -> Self {
        Self {
            view: JobOutput::from(&row),
            object_key: row.object_key,
        }
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    async fn isolated_database() -> Result<Option<(Database, Database, String)>> {
        if std::env::var("FORGEQUEUE_DATABASE_TESTS").as_deref() != Ok("1") {
            return Ok(None);
        }

        let database_url = std::env::var("DATABASE_URL")
            .context("DATABASE_URL is required when FORGEQUEUE_DATABASE_TESTS=1")?;
        let admin = Database::connect(&database_url).await?;
        let schema = format!("forgequeue_test_{}", Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE SCHEMA \"{schema}\""))
            .execute(admin.pool())
            .await?;

        let separator = if database_url.contains('?') { '&' } else { '?' };
        let scoped_url = format!("{database_url}{separator}options=-csearch_path%3D{schema}");
        let database = Database::connect(&scoped_url).await?;
        database.migrate().await?;
        Ok(Some((database, admin, schema)))
    }

    async fn insert_job(
        database: &Database,
        session_id: Uuid,
        idempotency_key: Option<&str>,
    ) -> Result<StoredJob> {
        let id = Uuid::now_v7();
        let object_key = format!("sessions/{session_id}/jobs/{id}/input");
        match database
            .create_job(
                NewJob {
                    id,
                    session_id,
                    kind: JobKind::Image,
                    original_name: "fixture.png",
                    input_object_key: &object_key,
                    input_content_type: "image/png",
                    input_size: 128,
                    input_sha256: "fixture-sha256",
                    idempotency_key,
                    client_ip_hash: b"integration-test-ip",
                    retry_of_job_id: None,
                    metadata_retention: Duration::from_secs(86_400),
                },
                None,
            )
            .await?
        {
            CreateJobOutcome::Created(job) | CreateJobOutcome::Existing(job) => Ok(job),
            CreateJobOutcome::LimitExceeded(limit) => {
                Err(anyhow!("unexpected upload limit in test: {limit:?}"))
            }
        }
    }

    async fn insert_limited_job(
        database: &Database,
        session_id: Uuid,
        id: Uuid,
    ) -> Result<CreateJobOutcome> {
        insert_job_with_limits(
            database,
            session_id,
            id,
            b"limited-integration-test-ip",
            UploadLimits {
                session_hourly: 1,
                ip_daily: 100,
                global_daily: 100,
            },
        )
        .await
    }

    async fn insert_job_with_limits(
        database: &Database,
        session_id: Uuid,
        id: Uuid,
        client_ip_hash: &[u8],
        limits: UploadLimits,
    ) -> Result<CreateJobOutcome> {
        let object_key = format!("sessions/{session_id}/jobs/{id}/input");
        database
            .create_job(
                NewJob {
                    id,
                    session_id,
                    kind: JobKind::Image,
                    original_name: "limited.png",
                    input_object_key: &object_key,
                    input_content_type: "image/png",
                    input_size: 64,
                    input_sha256: "limited-sha256",
                    idempotency_key: None,
                    client_ip_hash,
                    retry_of_job_id: None,
                    metadata_retention: Duration::from_secs(86_400),
                },
                Some(limits),
            )
            .await
    }

    #[tokio::test]
    async fn postgres_queue_contract_is_concurrent_idempotent_and_recoverable() -> Result<()> {
        let Some((database, admin, schema)) = isolated_database().await? else {
            eprintln!("PostgreSQL integration test skipped; set FORGEQUEUE_DATABASE_TESTS=1");
            return Ok(());
        };

        let result = async {
            let session = database
                .create_session(b"session-one", Duration::from_secs(3_600))
                .await?;
            let other_session = database
                .create_session(b"session-two", Duration::from_secs(3_600))
                .await?;

            let idempotent = insert_job(&database, session.id, Some("same-upload")).await?;
            let extended_session_expiry = sqlx::query_scalar::<_, DateTime<Utc>>(
                "SELECT expires_at FROM anonymous_sessions WHERE id = $1",
            )
            .bind(session.id)
            .fetch_one(database.pool())
            .await?;
            assert!(
                extended_session_expiry > Utc::now() + chrono::Duration::hours(23),
                "creating a job must keep its session alive through metadata retention"
            );
            let found = database
                .find_idempotent_job(session.id, "same-upload")
                .await?
                .context("idempotent job must be found")?;
            assert_eq!(found.view.id, idempotent.view.id);
            let duplicate = insert_job(&database, session.id, Some("same-upload")).await?;
            assert_eq!(
                duplicate.view.id, idempotent.view.id,
                "a duplicate idempotency key must reuse the original job"
            );
            assert!(
                database
                    .get_job_for_session(other_session.id, idempotent.view.id)
                    .await?
                    .is_none(),
                "jobs must remain isolated between anonymous sessions"
            );

            let limited_session = database
                .create_session(b"session-limited", Duration::from_secs(3_600))
                .await?;
            let (limited_a, limited_b) = tokio::join!(
                insert_limited_job(&database, limited_session.id, Uuid::now_v7()),
                insert_limited_job(&database, limited_session.id, Uuid::now_v7()),
            );
            let limited_outcomes = [limited_a?, limited_b?];
            assert_eq!(
                limited_outcomes
                    .iter()
                    .filter(|outcome| matches!(outcome, CreateJobOutcome::Created(_)))
                    .count(),
                1,
                "the atomic session limit must admit exactly one concurrent upload"
            );
            assert_eq!(
                limited_outcomes
                    .iter()
                    .filter(|outcome| matches!(
                        outcome,
                        CreateJobOutcome::LimitExceeded(UploadLimit::SessionHourly)
                    ))
                    .count(),
                1,
                "the other concurrent upload must be rate limited"
            );
            let mut created_limited_job = None;
            for outcome in &limited_outcomes {
                if let CreateJobOutcome::Created(job) = outcome {
                    created_limited_job = Some(job.view.id);
                    database
                        .request_cancel(limited_session.id, job.view.id, Duration::from_secs(3_600))
                        .await?;
                }
            }
            let created_limited_job = created_limited_job.context("one limited job must exist")?;
            assert!(
                database
                    .delete_job(limited_session.id, created_limited_job)
                    .await?,
                "the terminal limited job must be deletable"
            );
            assert!(
                matches!(
                    insert_limited_job(&database, limited_session.id, Uuid::now_v7()).await?,
                    CreateJobOutcome::LimitExceeded(UploadLimit::SessionHourly)
                ),
                "deleting job metadata must not refund the hourly quota"
            );

            let ip_session_a = database
                .create_session(b"ip-session-a", Duration::from_secs(3_600))
                .await?;
            let ip_session_b = database
                .create_session(b"ip-session-b", Duration::from_secs(3_600))
                .await?;
            let ip_limits = UploadLimits {
                session_hourly: 100,
                ip_daily: 1,
                global_daily: 100,
            };
            let ip_created = insert_job_with_limits(
                &database,
                ip_session_a.id,
                Uuid::now_v7(),
                b"shared-ip",
                ip_limits,
            )
            .await?;
            let CreateJobOutcome::Created(ip_created) = ip_created else {
                return Err(anyhow!("the first shared-IP upload must be created"));
            };
            database
                .request_cancel(
                    ip_session_a.id,
                    ip_created.view.id,
                    Duration::from_secs(3_600),
                )
                .await?;
            assert!(matches!(
                insert_job_with_limits(
                    &database,
                    ip_session_b.id,
                    Uuid::now_v7(),
                    b"shared-ip",
                    ip_limits,
                )
                .await?,
                CreateJobOutcome::LimitExceeded(UploadLimit::IpDaily)
            ));

            let global_session_a = database
                .create_session(b"global-session-a", Duration::from_secs(3_600))
                .await?;
            let global_session_b = database
                .create_session(b"global-session-b", Duration::from_secs(3_600))
                .await?;
            let existing_global = database
                .upload_counts(global_session_a.id, b"unused-ip")
                .await?
                .2;
            let global_limits = UploadLimits {
                session_hourly: 100,
                ip_daily: 100,
                global_daily: existing_global + 1,
            };
            let global_created = insert_job_with_limits(
                &database,
                global_session_a.id,
                Uuid::now_v7(),
                b"global-ip-a",
                global_limits,
            )
            .await?;
            let CreateJobOutcome::Created(global_created) = global_created else {
                return Err(anyhow!("the first globally limited upload must be created"));
            };
            database
                .request_cancel(
                    global_session_a.id,
                    global_created.view.id,
                    Duration::from_secs(3_600),
                )
                .await?;
            assert!(matches!(
                insert_job_with_limits(
                    &database,
                    global_session_b.id,
                    Uuid::now_v7(),
                    b"global-ip-b",
                    global_limits,
                )
                .await?,
                CreateJobOutcome::LimitExceeded(UploadLimit::GlobalDaily)
            ));

            let second = insert_job(&database, session.id, None).await?;
            let third = insert_job(&database, session.id, None).await?;
            let (first_claim, second_claim, third_claim) = tokio::join!(
                database.claim_next("worker-a", Duration::from_secs(60)),
                database.claim_next("worker-b", Duration::from_secs(60)),
                database.claim_next("worker-c", Duration::from_secs(60)),
            );
            let claims = [
                first_claim?.context("worker-a must claim a job")?,
                second_claim?.context("worker-b must claim a job")?,
                third_claim?.context("worker-c must claim a job")?,
            ];
            let claimed_ids: std::collections::HashSet<_> =
                claims.iter().map(|claim| claim.job.view.id).collect();
            assert_eq!(
                claimed_ids.len(),
                3,
                "SKIP LOCKED must prevent double claims"
            );
            assert!(claimed_ids.contains(&idempotent.view.id));
            assert!(claimed_ids.contains(&second.view.id));
            assert!(claimed_ids.contains(&third.view.id));

            let abandoned = &claims[0];
            sqlx::query("UPDATE jobs SET lease_until = now() - interval '1 second' WHERE id = $1")
                .bind(abandoned.job.view.id)
                .execute(database.pool())
                .await?;
            assert!(
                !database
                    .heartbeat(
                        abandoned.job.view.id,
                        abandoned.attempt_id,
                        "worker-a",
                        Duration::from_secs(60),
                    )
                    .await?,
                "an expired lease must not be revived by a late heartbeat"
            );
            let recovered = database
                .recover_expired_leases(Duration::from_secs(3_600))
                .await?;
            assert_eq!(recovered, vec![abandoned.job.view.id]);
            let recovered_job = database
                .get_job_for_session(session.id, abandoned.job.view.id)
                .await?
                .context("recovered job must exist")?;
            assert_eq!(recovered_job.view.status, JobStatus::RetryScheduled);
            assert_eq!(
                recovered_job.view.last_error_code.as_deref(),
                Some("worker_lost")
            );
            let recovery_delay = sqlx::query_scalar::<_, f64>(
                "SELECT EXTRACT(EPOCH FROM (available_at - now()))::float8
                 FROM jobs WHERE id = $1",
            )
            .bind(abandoned.job.view.id)
            .fetch_one(database.pool())
            .await?;
            assert!((3.0..=5.1).contains(&recovery_delay));

            sqlx::query("UPDATE jobs SET available_at = now() WHERE id = $1")
                .bind(abandoned.job.view.id)
                .execute(database.pool())
                .await?;

            let reclaimed = database
                .claim_next("worker-recovery", Duration::from_secs(60))
                .await?
                .context("recovered job must be claimable after its backoff")?;
            assert_eq!(reclaimed.job.view.id, abandoned.job.view.id);
            assert_eq!(reclaimed.job.view.attempt_count, 2);
            assert!(
                !database
                    .heartbeat(
                        abandoned.job.view.id,
                        abandoned.attempt_id,
                        "worker-a",
                        Duration::from_secs(60),
                    )
                    .await?,
                "a stale attempt must not renew the current lease"
            );
            assert!(
                !database
                    .update_progress(
                        abandoned.job.view.id,
                        abandoned.attempt_id,
                        "worker-a",
                        "stale_progress",
                        99,
                    )
                    .await?,
                "a stale attempt must not overwrite current progress"
            );
            assert!(
                !database
                    .finish_success(
                        abandoned.job.view.id,
                        abandoned.attempt_id,
                        "worker-a",
                        Duration::from_secs(3_600),
                    )
                    .await?,
                "a stale worker must not finalize a newer lease"
            );
            let still_running = database
                .get_job_for_session(session.id, abandoned.job.view.id)
                .await?
                .context("reclaimed job must still exist")?;
            assert_eq!(still_running.view.status, JobStatus::Running);
            assert_eq!(still_running.view.attempt_count, 2);
            let stale_output = database
                .upsert_output(
                    NewOutput {
                        job_id: reclaimed.job.view.id,
                        name: "stale.webp",
                        object_key: "deterministic/stale.webp",
                        content_type: "image/webp",
                        size: 1,
                        width: Some(1),
                        height: Some(1),
                        page_number: None,
                    },
                    abandoned.attempt_id,
                    "worker-a",
                )
                .await?;
            assert!(
                stale_output.is_none(),
                "a stale attempt must not publish outputs"
            );

            let failed = &claims[1];
            let status = database
                .finish_failure(FinishFailure {
                    job_id: failed.job.view.id,
                    attempt_id: failed.attempt_id,
                    worker_id: "worker-b",
                    code: "temporary_failure",
                    detail: "fallo transitorio de prueba",
                    retryable: true,
                    artifact_retention: Duration::from_secs(3_600),
                })
                .await?;
            assert_eq!(status, Some(JobStatus::RetryScheduled));
            let seconds_until_retry = sqlx::query_scalar::<_, f64>(
                "SELECT EXTRACT(EPOCH FROM (available_at - now()))::float8 FROM jobs WHERE id = $1",
            )
            .bind(failed.job.view.id)
            .fetch_one(database.pool())
            .await?;
            assert!((3.0..=5.1).contains(&seconds_until_retry));

            sqlx::query("UPDATE jobs SET available_at = now() WHERE id = $1")
                .bind(failed.job.view.id)
                .execute(database.pool())
                .await?;
            let second_failed_attempt = database
                .claim_next("worker-b-retry-2", Duration::from_secs(60))
                .await?
                .context("the first retry must be claimable")?;
            assert_eq!(second_failed_attempt.job.view.id, failed.job.view.id);
            assert_eq!(second_failed_attempt.job.view.attempt_count, 2);
            assert_eq!(
                database
                    .finish_failure(FinishFailure {
                        job_id: failed.job.view.id,
                        attempt_id: second_failed_attempt.attempt_id,
                        worker_id: "worker-b-retry-2",
                        code: "temporary_failure",
                        detail: "segundo fallo transitorio de prueba",
                        retryable: true,
                        artifact_retention: Duration::from_secs(3_600),
                    })
                    .await?,
                Some(JobStatus::RetryScheduled)
            );
            let second_retry_delay = sqlx::query_scalar::<_, f64>(
                "SELECT EXTRACT(EPOCH FROM (available_at - now()))::float8 FROM jobs WHERE id = $1",
            )
            .bind(failed.job.view.id)
            .fetch_one(database.pool())
            .await?;
            assert!((28.0..=30.1).contains(&second_retry_delay));

            sqlx::query("UPDATE jobs SET available_at = now() WHERE id = $1")
                .bind(failed.job.view.id)
                .execute(database.pool())
                .await?;
            let third_failed_attempt = database
                .claim_next("worker-b-retry-3", Duration::from_secs(60))
                .await?
                .context("the second retry must be claimable")?;
            assert_eq!(third_failed_attempt.job.view.id, failed.job.view.id);
            assert_eq!(third_failed_attempt.job.view.attempt_count, 3);
            assert_eq!(
                database
                    .finish_failure(FinishFailure {
                        job_id: failed.job.view.id,
                        attempt_id: third_failed_attempt.attempt_id,
                        worker_id: "worker-b-retry-3",
                        code: "temporary_failure",
                        detail: "tercer fallo transitorio de prueba",
                        retryable: true,
                        artifact_retention: Duration::from_secs(3_600),
                    })
                    .await?,
                Some(JobStatus::DeadLettered)
            );

            database
                .upsert_output(
                    NewOutput {
                        job_id: reclaimed.job.view.id,
                        name: "preview.webp",
                        object_key: "deterministic/preview.webp",
                        content_type: "image/webp",
                        size: 100,
                        width: Some(320),
                        height: Some(200),
                        page_number: None,
                    },
                    reclaimed.attempt_id,
                    "worker-recovery",
                )
                .await?;
            database
                .upsert_output(
                    NewOutput {
                        job_id: reclaimed.job.view.id,
                        name: "preview.webp",
                        object_key: "deterministic/preview.webp",
                        content_type: "image/webp",
                        size: 101,
                        width: Some(320),
                        height: Some(200),
                        page_number: None,
                    },
                    reclaimed.attempt_id,
                    "worker-recovery",
                )
                .await?;
            let output_count = sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM job_outputs WHERE job_id = $1 AND name = 'preview.webp'",
            )
            .bind(reclaimed.job.view.id)
            .fetch_one(database.pool())
            .await?;
            assert_eq!(output_count, 1, "output writes must be idempotent");

            assert!(
                database
                    .finish_success(
                        reclaimed.job.view.id,
                        reclaimed.attempt_id,
                        "worker-recovery",
                        Duration::ZERO,
                    )
                    .await?,
                "the current lease owner must be able to finish"
            );
            let cleanup = database.cleanup_candidates(100).await?;
            let candidate = cleanup
                .iter()
                .find(|candidate| candidate.job_id == reclaimed.job.view.id)
                .context("the completed job must become eligible for artifact cleanup")?;
            assert_eq!(
                candidate.object_prefix,
                format!(
                    "sessions/{}/jobs/{}",
                    reclaimed.job.session_id, reclaimed.job.view.id
                )
            );
            database
                .mark_artifacts_deleted(reclaimed.job.view.id)
                .await?;
            let expired = database
                .get_job_for_session(session.id, reclaimed.job.view.id)
                .await?
                .context("expired metadata must remain visible")?;
            assert_eq!(expired.view.status, JobStatus::Expired);

            sqlx::query(
                "UPDATE jobs SET metadata_expire_at = now() - interval '1 second' WHERE id = $1",
            )
            .bind(reclaimed.job.view.id)
            .execute(database.pool())
            .await?;
            assert!(database.purge_expired_metadata().await? >= 1);
            assert!(
                database
                    .get_job_for_session(session.id, reclaimed.job.view.id)
                    .await?
                    .is_none(),
                "expired metadata must be purged"
            );

            let orphaned = insert_job(&database, session.id, None).await?;
            sqlx::query(
                "UPDATE jobs SET metadata_expire_at = now() - interval '1 second' WHERE id = $1",
            )
            .bind(orphaned.view.id)
            .execute(database.pool())
            .await?;
            database.purge_expired_metadata().await?;
            assert!(
                database
                    .get_job_for_session(session.id, orphaned.view.id)
                    .await?
                    .is_some(),
                "metadata must remain until object deletion has succeeded"
            );
            let orphan_cleanup = database.cleanup_candidates(100).await?;
            assert!(
                orphan_cleanup
                    .iter()
                    .any(|candidate| candidate.job_id == orphaned.view.id),
                "an unfinished expired job must expose its object prefix for cleanup"
            );
            database.mark_artifacts_deleted(orphaned.view.id).await?;
            assert!(database.purge_expired_metadata().await? >= 1);
            assert!(
                database
                    .get_job_for_session(session.id, orphaned.view.id)
                    .await?
                    .is_none(),
                "unfinished expired metadata must be purged after its objects are removed"
            );

            Ok::<_, anyhow::Error>(())
        }
        .await;

        database.pool.close().await;
        sqlx::query(&format!("DROP SCHEMA \"{schema}\" CASCADE"))
            .execute(admin.pool())
            .await?;
        admin.pool.close().await;
        result
    }
}
