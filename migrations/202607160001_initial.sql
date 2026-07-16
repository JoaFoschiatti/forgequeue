CREATE TABLE anonymous_sessions (
    id UUID PRIMARY KEY,
    token_hash BYTEA NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE jobs (
    id UUID PRIMARY KEY,
    session_id UUID NOT NULL REFERENCES anonymous_sessions(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('image', 'pdf')),
    status TEXT NOT NULL CHECK (status IN (
        'queued', 'running', 'retry_scheduled', 'succeeded',
        'cancel_requested', 'cancelled', 'dead_lettered', 'expired'
    )),
    progress SMALLINT NOT NULL DEFAULT 0 CHECK (progress BETWEEN 0 AND 100),
    stage TEXT NOT NULL DEFAULT 'queued',
    original_name TEXT NOT NULL,
    input_object_key TEXT NOT NULL,
    input_content_type TEXT NOT NULL,
    input_size BIGINT NOT NULL CHECK (input_size >= 0),
    input_sha256 TEXT NOT NULL,
    idempotency_key TEXT,
    client_ip_hash BYTEA NOT NULL,
    available_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    lease_until TIMESTAMPTZ,
    worker_id TEXT,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 3,
    last_error_code TEXT,
    last_error_detail TEXT,
    retry_of_job_id UUID REFERENCES jobs(id) ON DELETE SET NULL,
    artifacts_deleted_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ,
    artifacts_expire_at TIMESTAMPTZ,
    metadata_expire_at TIMESTAMPTZ NOT NULL DEFAULT (now() + interval '24 hours')
);

CREATE UNIQUE INDEX jobs_session_idempotency_idx
    ON jobs(session_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;
CREATE INDEX jobs_claim_idx ON jobs(status, available_at, created_at);
CREATE INDEX jobs_session_created_idx ON jobs(session_id, created_at DESC);
CREATE INDEX jobs_ip_created_idx ON jobs(client_ip_hash, created_at DESC);

CREATE TABLE job_attempts (
    id UUID PRIMARY KEY,
    job_id UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    number INTEGER NOT NULL,
    worker_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('running', 'succeeded', 'failed', 'cancelled')),
    error_code TEXT,
    error_detail TEXT,
    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at TIMESTAMPTZ,
    UNIQUE(job_id, number)
);

CREATE TABLE job_outputs (
    id UUID PRIMARY KEY,
    job_id UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    object_key TEXT NOT NULL,
    content_type TEXT NOT NULL,
    size BIGINT NOT NULL CHECK (size >= 0),
    width INTEGER,
    height INTEGER,
    page_number INTEGER,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(job_id, name)
);
