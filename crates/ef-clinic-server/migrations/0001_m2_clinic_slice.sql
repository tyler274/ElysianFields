-- M2 Curie Clinic vertical slice durable relational schema (PostgreSQL 16/17).
-- Mirrors the authoritative domain records persisted by `DurableClinicStore`.

CREATE TABLE IF NOT EXISTS practices (
    practice_id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    created_at_secs BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS physician_accounts (
    physician_id TEXT PRIMARY KEY,
    practice_id TEXT NOT NULL REFERENCES practices(practice_id) ON DELETE CASCADE,
    email TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('standard', 'admin')),
    standing TEXT NOT NULL CHECK (standing IN ('active', 'deactivated')),
    sessions_ended_before BIGINT
);

CREATE TABLE IF NOT EXISTS verification_tokens (
    token_hash TEXT PRIMARY KEY,
    email TEXT NOT NULL REFERENCES physician_accounts(email) ON DELETE CASCADE,
    token_kind TEXT NOT NULL CHECK (token_kind IN ('sign_in_link', 'colleague_invite')),
    expires_at_secs BIGINT NOT NULL,
    consumed_at_secs BIGINT
);

CREATE TABLE IF NOT EXISTS sessions (
    session_id TEXT PRIMARY KEY,
    physician_id TEXT NOT NULL REFERENCES physician_accounts(physician_id) ON DELETE CASCADE,
    practice_id TEXT NOT NULL REFERENCES practices(practice_id) ON DELETE CASCADE,
    created_at_secs BIGINT NOT NULL,
    idle_deadline_secs BIGINT NOT NULL,
    absolute_end_secs BIGINT NOT NULL,
    revoked_at_secs BIGINT
);

CREATE INDEX IF NOT EXISTS idx_sessions_physician ON sessions(physician_id);

CREATE TABLE IF NOT EXISTS patient_workspaces (
    workspace_id TEXT PRIMARY KEY,
    practice_id TEXT NOT NULL REFERENCES practices(practice_id) ON DELETE CASCADE,
    display_name TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS workspace_panel_memberships (
    workspace_id TEXT NOT NULL REFERENCES patient_workspaces(workspace_id) ON DELETE CASCADE,
    physician_id TEXT NOT NULL REFERENCES physician_accounts(physician_id) ON DELETE CASCADE,
    PRIMARY KEY (workspace_id, physician_id)
);

CREATE TABLE IF NOT EXISTS cited_sources (
    workspace_id TEXT NOT NULL REFERENCES patient_workspaces(workspace_id) ON DELETE CASCADE,
    source_key TEXT NOT NULL,
    citation_number INTEGER NOT NULL,
    source_label TEXT NOT NULL,
    title TEXT NOT NULL,
    record_type TEXT NOT NULL,
    recorded_at TEXT NOT NULL,
    record_status TEXT NOT NULL,
    status_class TEXT NOT NULL CHECK (status_class IN ('settled', 'flagged')),
    origin TEXT NOT NULL CHECK (origin IN ('source_record', 'prepared_by_assistant')),
    original_language TEXT NOT NULL,
    quote TEXT NOT NULL,
    body TEXT NOT NULL,
    readable_parts_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    structured_fields_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    withdrawn_at_secs BIGINT,
    PRIMARY KEY (workspace_id, source_key)
);

CREATE TABLE IF NOT EXISTS chat_exchanges (
    work_item_id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES patient_workspaces(workspace_id) ON DELETE CASCADE,
    chat_id TEXT NOT NULL,
    current_run_id TEXT NOT NULL,
    prompt_text TEXT NOT NULL,
    kept_partial_text TEXT NOT NULL DEFAULT '',
    steps_json JSONB NOT NULL,
    answer_state_json JSONB NOT NULL,
    failed_attempts INTEGER NOT NULL DEFAULT 0,
    next_retry_at_secs BIGINT,
    lease_worker_id TEXT,
    lease_claimed_at_secs BIGINT,
    lease_expires_at_secs BIGINT,
    lease_last_progress_at_secs BIGINT,
    frozen_recipe_version TEXT NOT NULL,
    next_delta_seq BIGINT NOT NULL DEFAULT 1,
    created_order BIGSERIAL NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_chat_exchanges_workspace_chat
    ON chat_exchanges(workspace_id, chat_id, created_order);

CREATE TABLE IF NOT EXISTS chat_stream_events (
    workspace_id TEXT NOT NULL REFERENCES patient_workspaces(workspace_id) ON DELETE CASCADE,
    chat_id TEXT NOT NULL,
    event_id BIGINT NOT NULL,
    event_json JSONB NOT NULL,
    PRIMARY KEY (workspace_id, chat_id, event_id)
);

CREATE TABLE IF NOT EXISTS command_idempotency_receipts (
    workspace_id TEXT NOT NULL REFERENCES patient_workspaces(workspace_id) ON DELETE CASCADE,
    chat_id TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    receipt_json JSONB NOT NULL,
    PRIMARY KEY (workspace_id, chat_id, idempotency_key)
);

CREATE TABLE IF NOT EXISTS composer_drafts (
    workspace_id TEXT NOT NULL REFERENCES patient_workspaces(workspace_id) ON DELETE CASCADE,
    chat_id TEXT NOT NULL,
    draft_text TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK (revision >= 0),
    PRIMARY KEY (workspace_id, chat_id)
);
