-- Keep quota accounting independent from user-visible job metadata. A user can
-- delete a completed job, but that must not refund an upload within the active
-- one-hour / 24-hour rate-limit windows.
CREATE TABLE upload_events (
    job_id UUID PRIMARY KEY,
    session_id UUID NOT NULL,
    client_ip_hash BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO upload_events (job_id, session_id, client_ip_hash, created_at)
SELECT id, session_id, client_ip_hash, created_at
FROM jobs
ON CONFLICT (job_id) DO NOTHING;

CREATE INDEX upload_events_session_created_idx
    ON upload_events(session_id, created_at DESC);
CREATE INDEX upload_events_ip_created_idx
    ON upload_events(client_ip_hash, created_at DESC);
CREATE INDEX upload_events_created_idx
    ON upload_events(created_at DESC);
