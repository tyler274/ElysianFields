-- M3 Increment 2: Standalone Curie Mobile PostgreSQL 16/17 relational schema.
-- Strictly isolated from Curie Clinic tables; runtime API connects as an unprivileged
-- role with zero DDL authority, and `/readyz` verifies schema_migrations + journal replay.

CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at_secs BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS mobile_accounts (
    account_id TEXT PRIMARY KEY,
    incarnation BIGINT NOT NULL CHECK (incarnation >= 1),
    email TEXT NOT NULL UNIQUE,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    deleted_at_incarnation BIGINT
);

CREATE TABLE IF NOT EXISTS mobile_deleted_incarnation_fences (
    account_id TEXT PRIMARY KEY,
    max_deleted_incarnation BIGINT NOT NULL CHECK (max_deleted_incarnation >= 1)
);

CREATE TABLE IF NOT EXISTS mobile_sessions (
    session_id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES mobile_accounts(account_id) ON DELETE CASCADE,
    incarnation BIGINT NOT NULL CHECK (incarnation >= 1),
    created_at_secs BIGINT NOT NULL,
    expires_at_secs BIGINT NOT NULL,
    revoked BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE TABLE IF NOT EXISTS portal_connections (
    portal_id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES mobile_accounts(account_id) ON DELETE CASCADE,
    vendor TEXT NOT NULL,
    display_name TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('connected', 'disconnected')),
    last_refresh_secs BIGINT NOT NULL,
    daily_refresh_interval_secs BIGINT NOT NULL DEFAULT 86400
);

CREATE TABLE IF NOT EXISTS patient_records (
    account_id TEXT NOT NULL REFERENCES mobile_accounts(account_id) ON DELETE CASCADE,
    record_id TEXT NOT NULL,
    incarnation BIGINT NOT NULL CHECK (incarnation >= 1),
    version BIGINT NOT NULL CHECK (version >= 1),
    source_kind_json JSONB NOT NULL,
    portal_id TEXT,
    bucket TEXT NOT NULL,
    confidentiality TEXT NOT NULL CHECK (confidentiality IN ('Standard', 'Part2Sud', 'Part2SudCounselingNotes')),
    part2_notice TEXT,
    original_date TEXT NOT NULL,
    title TEXT NOT NULL,
    summary TEXT NOT NULL,
    native_bytes BYTEA NOT NULL,
    removed_upstream BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (account_id, record_id)
);

CREATE TABLE IF NOT EXISTS health_samples (
    account_id TEXT NOT NULL REFERENCES mobile_accounts(account_id) ON DELETE CASCADE,
    idempotency_key TEXT NOT NULL,
    sample_id TEXT NOT NULL,
    metric TEXT NOT NULL,
    value_milli BIGINT NOT NULL,
    unit TEXT NOT NULL,
    recorded_at_secs BIGINT NOT NULL,
    platform TEXT NOT NULL,
    PRIMARY KEY (account_id, idempotency_key, sample_id)
);

CREATE TABLE IF NOT EXISTS push_device_tokens (
    device_id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES mobile_accounts(account_id) ON DELETE CASCADE,
    platform TEXT NOT NULL CHECK (platform IN ('ios_apns', 'android_fcm')),
    token TEXT NOT NULL,
    proactive_enabled BOOLEAN NOT NULL DEFAULT TRUE
);

CREATE TABLE IF NOT EXISTS for_my_doctor_sends (
    send_id TEXT PRIMARY KEY,
    idempotency_key TEXT NOT NULL UNIQUE,
    account_id TEXT NOT NULL REFERENCES mobile_accounts(account_id) ON DELETE CASCADE,
    practice_id TEXT NOT NULL,
    snapshot_json JSONB NOT NULL,
    status TEXT NOT NULL,
    created_at_secs BIGINT NOT NULL
);

INSERT INTO schema_migrations (version, applied_at_secs)
VALUES (1, 0)
ON CONFLICT (version) DO NOTHING;
