#!/usr/bin/env bash
set -euo pipefail

# Self-contained disposable PostgreSQL migration & constraint proof script.
# Spins up an ephemeral initdb/postgres cluster on a private Unix socket,
# applies both Curie Clinic (`0001_m2_clinic_slice.sql`) and Curie Mobile
# (`0001_m3_mobile.sql`) schemas into isolated databases, verifies idempotent
# re-application, and asserts PK/FK/UNIQUE/CHECK constraints + Part 2 &
# account_incarnation invariants.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

CLINIC_SQL="${REPO_ROOT}/crates/ef-clinic-server/migrations/0001_m2_clinic_slice.sql"
MOBILE_SQL="${REPO_ROOT}/crates/ef-mobile-server/migrations/0001_m3_mobile.sql"

if [[ ! -f "${CLINIC_SQL}" ]]; then
  echo "ERROR: missing clinic migration file: ${CLINIC_SQL}" >&2
  exit 1
fi
if [[ ! -f "${MOBILE_SQL}" ]]; then
  echo "ERROR: missing mobile migration file: ${MOBILE_SQL}" >&2
  exit 1
fi

# Locate PostgreSQL binaries (supports Nix shell, Ubuntu /usr/lib/postgresql/*/bin, or PATH).
if ! command -v psql >/dev/null 2>&1 || [[ -z "${PGHOST:-}" && ! -x "$(command -v initdb || true)" ]]; then
  for candidate in /usr/lib/postgresql/18/bin /usr/lib/postgresql/17/bin /usr/lib/postgresql/16/bin; do
    if [[ -x "${candidate}/psql" ]]; then
      export PATH="${candidate}:${PATH}"
      break
    fi
  done
fi

USE_EXTERNAL_PG=0
if [[ -n "${PGHOST:-}" ]]; then
  USE_EXTERNAL_PG=1
  required_bins=(createdb dropdb psql)
else
  required_bins=(initdb pg_ctl createdb psql)
fi

for bin in "${required_bins[@]}"; do
  if ! command -v "${bin}" >/dev/null 2>&1; then
    echo "ERROR: required PostgreSQL tool '${bin}' not found on PATH" >&2
    exit 1
  fi
done

WORK_DIR=""
PGDATA=""
PG_CONN_ARGS=()

cleanup() {
  if [[ "${USE_EXTERNAL_PG}" -eq 1 ]]; then
    dropdb --if-exists ef_clinic_test >/dev/null 2>&1 || true
    dropdb --if-exists ef_mobile_test >/dev/null 2>&1 || true
  else
    if [[ -n "${PGDATA}" && -d "${PGDATA}" ]] && pg_ctl -D "${PGDATA}" status >/dev/null 2>&1; then
      pg_ctl -D "${PGDATA}" -m immediate stop >/dev/null 2>&1 || true
    fi
    if [[ -n "${WORK_DIR}" ]]; then
      rm -rf "${WORK_DIR}"
    fi
  fi
}
trap cleanup EXIT

if [[ "${USE_EXTERNAL_PG}" -eq 1 ]]; then
  echo "==> Using external PostgreSQL server at ${PGHOST}:${PGPORT:-5432}"
  dropdb --if-exists ef_clinic_test >/dev/null 2>&1 || true
  dropdb --if-exists ef_mobile_test >/dev/null 2>&1 || true
else
  # PostgreSQL refuses to run as root; guard clearly if invoked unexpectedly as UID 0.
  if [[ "$(id -u)" -eq 0 ]]; then
    echo "ERROR: run verify_postgres_migrations.sh as a non-root user (initdb forbids root)" >&2
    exit 1
  fi

  WORK_DIR="$(mktemp -d -t ef-pg-migrations-XXXXXX)"
  PGDATA="${WORK_DIR}/pgdata"
  PGSOCK="${WORK_DIR}/sock"
  PGLOG="${WORK_DIR}/postgres.log"
  mkdir -p "${PGSOCK}"

  echo "==> Initializing disposable PostgreSQL cluster in ${PGDATA}"
  initdb -D "${PGDATA}" --no-locale --encoding=UTF8 --auth=trust >/dev/null

  echo "==> Starting PostgreSQL on local Unix socket ${PGSOCK}"
  pg_ctl -D "${PGDATA}" -l "${PGLOG}" -o "-k ${PGSOCK} -h '' -F" start >/dev/null
  PG_CONN_ARGS=(-h "${PGSOCK}")
fi

PG_VERSION="$(psql "${PG_CONN_ARGS[@]}" -d postgres -v ON_ERROR_STOP=1 -tAc "SELECT version();")"
echo "==> Connected: ${PG_VERSION}"

createdb "${PG_CONN_ARGS[@]}" ef_clinic_test
createdb "${PG_CONN_ARGS[@]}" ef_mobile_test

echo "==> Applying clinic migration (pass 1 + idempotent pass 2)"
psql "${PG_CONN_ARGS[@]}" -d ef_clinic_test -v ON_ERROR_STOP=1 -f "${CLINIC_SQL}" >/dev/null
psql "${PG_CONN_ARGS[@]}" -d ef_clinic_test -v ON_ERROR_STOP=1 -f "${CLINIC_SQL}" >/dev/null

echo "==> Applying mobile migration (pass 1 + idempotent pass 2)"
psql "${PG_CONN_ARGS[@]}" -d ef_mobile_test -v ON_ERROR_STOP=1 -f "${MOBILE_SQL}" >/dev/null
psql "${PG_CONN_ARGS[@]}" -d ef_mobile_test -v ON_ERROR_STOP=1 -f "${MOBILE_SQL}" >/dev/null

echo "==> Verifying Curie Clinic relational constraints & invariants"
psql "${PG_CONN_ARGS[@]}" -d ef_clinic_test -v ON_ERROR_STOP=1 <<'SQL'
-- Valid vertical-slice rows across all core tables.
INSERT INTO practices (practice_id, display_name, created_at_secs)
VALUES ('practice-a', 'Synthetic Practice A', 1760000000);

INSERT INTO physician_accounts (physician_id, practice_id, email, display_name, role, standing)
VALUES ('phys-a1', 'practice-a', 'doc.a@example.invalid', 'Dr. Synthetic A', 'standard', 'active');

INSERT INTO verification_tokens (token_hash, email, token_kind, expires_at_secs)
VALUES ('sha256-tok-1', 'doc.a@example.invalid', 'sign_in_link', 1760086400);

INSERT INTO sessions (session_id, physician_id, practice_id, created_at_secs, idle_deadline_secs, absolute_end_secs)
VALUES ('sess-1', 'phys-a1', 'practice-a', 1760000000, 1760043200, 1760604800);

INSERT INTO patient_workspaces (workspace_id, practice_id, display_name)
VALUES ('ws-a1', 'practice-a', 'Synthetic Patient A1');

INSERT INTO workspace_panel_memberships (workspace_id, physician_id)
VALUES ('ws-a1', 'phys-a1');

INSERT INTO cited_sources (
    workspace_id, source_key, citation_number, source_label, title,
    record_type, recorded_at, record_status, status_class, origin,
    original_language, quote, body, readable_parts_json, structured_fields_json
) VALUES (
    'ws-a1', 'src-note-1', 1, 'Progress Note [1]', 'Synthetic Follow-Up',
    'progress_note', '2026-01-12T09:00:00Z', 'final', 'settled', 'source_record',
    'en', 'Creatinine 1.02 mg/dL', 'Synthetic note body', '[]'::jsonb, '[]'::jsonb
);

INSERT INTO chat_exchanges (
    work_item_id, workspace_id, chat_id, current_run_id, prompt_text,
    steps_json, answer_state_json, frozen_recipe_version
) VALUES (
    'wi-1', 'ws-a1', 'chat-1', 'run-1', 'Review labs',
    '{"kind":"read","steps":[]}'::jsonb, '{"kind":"working"}'::jsonb, 'm2-clinic-v1'
);

INSERT INTO chat_stream_events (workspace_id, chat_id, event_id, event_json)
VALUES ('ws-a1', 'chat-1', 1, '{"event":"delta_chunk","seq":1}'::jsonb);

INSERT INTO command_idempotency_receipts (workspace_id, chat_id, idempotency_key, receipt_json)
VALUES ('ws-a1', 'chat-1', 'idem-1', '{"work_item_id":"wi-1"}'::jsonb);

INSERT INTO composer_drafts (workspace_id, chat_id, draft_text, revision)
VALUES ('ws-a1', 'chat-1', 'Repeat BMP in 2 weeks.', 1);

-- Negative constraint proofs using transactional DO blocks.
DO $$
BEGIN
    BEGIN
        INSERT INTO physician_accounts (physician_id, practice_id, email, display_name, role, standing)
        VALUES ('phys-bad-role', 'practice-a', 'bad@example.invalid', 'Bad', 'superuser', 'active');
        RAISE EXCEPTION 'expected CHECK violation for invalid physician role';
    EXCEPTION WHEN check_violation THEN
        NULL;
    END;

    BEGIN
        INSERT INTO physician_accounts (physician_id, practice_id, email, display_name, role, standing)
        VALUES ('phys-dup-email', 'practice-a', 'doc.a@example.invalid', 'Dup', 'standard', 'active');
        RAISE EXCEPTION 'expected UNIQUE violation for duplicate physician email';
    EXCEPTION WHEN unique_violation THEN
        NULL;
    END;

    BEGIN
        INSERT INTO sessions (session_id, physician_id, practice_id, created_at_secs, idle_deadline_secs, absolute_end_secs)
        VALUES ('sess-orphan', 'phys-missing', 'practice-a', 1760000000, 1760043200, 1760604800);
        RAISE EXCEPTION 'expected FK violation for orphan session';
    EXCEPTION WHEN foreign_key_violation THEN
        NULL;
    END;

    BEGIN
        INSERT INTO cited_sources (
            workspace_id, source_key, citation_number, source_label, title,
            record_type, recorded_at, record_status, status_class, origin,
            original_language, quote, body
        ) VALUES (
            'ws-a1', 'src-bad-status', 2, 'Note [2]', 'Bad',
            'progress_note', '2026-01-12', 'final', 'unknown_status', 'source_record',
            'en', 'q', 'b'
        );
        RAISE EXCEPTION 'expected CHECK violation for invalid status_class';
    EXCEPTION WHEN check_violation THEN
        NULL;
    END;

    BEGIN
        INSERT INTO composer_drafts (workspace_id, chat_id, draft_text, revision)
        VALUES ('ws-a1', 'chat-neg', 'bad', -1);
        RAISE EXCEPTION 'expected CHECK violation for negative draft revision';
    EXCEPTION WHEN check_violation THEN
        NULL;
    END;

    BEGIN
        INSERT INTO command_idempotency_receipts (workspace_id, chat_id, idempotency_key, receipt_json)
        VALUES ('ws-a1', 'chat-1', 'idem-1', '{"work_item_id":"wi-dup"}'::jsonb);
        RAISE EXCEPTION 'expected PK violation for duplicate idempotency_key';
    EXCEPTION WHEN unique_violation THEN
        NULL;
    END;
END $$;

-- Verify ON DELETE CASCADE clears child rows cleanly.
DELETE FROM practices WHERE practice_id = 'practice-a';
DO $$
DECLARE remaining INT;
BEGIN
    SELECT COUNT(*) INTO remaining FROM chat_exchanges;
    IF remaining <> 0 THEN
        RAISE EXCEPTION 'expected 0 chat_exchanges after practice cascade delete, got %', remaining;
    END IF;
END $$;
SQL

echo "==> Verifying Curie Mobile relational constraints & Part 2 / incarnation invariants"
psql "${PG_CONN_ARGS[@]}" -d ef_mobile_test -v ON_ERROR_STOP=1 <<'SQL'
-- Verify schema_migrations seeded version 1 idempotently.
DO $$
DECLARE mig_count INT;
BEGIN
    SELECT COUNT(*) INTO mig_count FROM schema_migrations WHERE version = 1;
    IF mig_count <> 1 THEN
        RAISE EXCEPTION 'expected exactly 1 row for schema_migrations version=1, got %', mig_count;
    END IF;
END $$;

-- Valid standalone Curie Mobile rows across all tables.
INSERT INTO mobile_accounts (account_id, incarnation, email)
VALUES ('mob-acct-1', 1, 'patient.a@example.invalid');

INSERT INTO mobile_deleted_incarnation_fences (account_id, max_deleted_incarnation)
VALUES ('mob-acct-deleted', 2);

INSERT INTO mobile_sessions (session_id, account_id, incarnation, created_at_secs, expires_at_secs)
VALUES ('mob-sess-1', 'mob-acct-1', 1, 1760000000, 1760604800);

INSERT INTO portal_connections (portal_id, account_id, vendor, display_name, state, last_refresh_secs)
VALUES ('portal-1', 'mob-acct-1', 'epic_my_chart', 'Synthetic Portal', 'connected', 1760000000);

INSERT INTO patient_records (
    account_id, record_id, incarnation, version, source_kind_json,
    portal_id, bucket, confidentiality, part2_notice, original_date,
    title, summary, native_bytes
) VALUES (
    'mob-acct-1', 'rec-part2-1', 1, 1, '{"kind":"kipu_behavioral_health"}'::jsonb,
    NULL, 'Notes', 'Part2Sud', '42 CFR Part 2 notice', '2026-01-12',
    'Synthetic SUD Note', 'Synthetic summary', decode('7b7d', 'hex')
);

INSERT INTO health_samples (
    account_id, idempotency_key, sample_id, metric, value_milli, unit, recorded_at_secs, platform
) VALUES (
    'mob-acct-1', 'hk-batch-1', 's-1', 'steps', 8420000, 'count', 1760000010, 'ios_health_kit'
);

INSERT INTO push_device_tokens (device_id, account_id, platform, token)
VALUES ('dev-ios-1', 'mob-acct-1', 'ios_apns', 'apns-tok-1');

INSERT INTO for_my_doctor_sends (
    send_id, idempotency_key, account_id, practice_id, snapshot_json, status, created_at_secs
) VALUES (
    'send-1', 'fmd-idem-1', 'mob-acct-1', 'practice-1', '{"records":[]}'::jsonb, 'sent', 1760000100
);

-- Negative constraint proofs for incarnation fencing, Part 2 closed enum, idempotency, and FKs.
DO $$
BEGIN
    BEGIN
        INSERT INTO mobile_accounts (account_id, incarnation, email)
        VALUES ('mob-zero-inc', 0, 'zero@example.invalid');
        RAISE EXCEPTION 'expected CHECK violation for account_incarnation = 0';
    EXCEPTION WHEN check_violation THEN
        NULL;
    END;

    BEGIN
        INSERT INTO mobile_deleted_incarnation_fences (account_id, max_deleted_incarnation)
        VALUES ('mob-bad-fence', 0);
        RAISE EXCEPTION 'expected CHECK violation for fence incarnation = 0';
    EXCEPTION WHEN check_violation THEN
        NULL;
    END;

    BEGIN
        INSERT INTO patient_records (
            account_id, record_id, incarnation, version, source_kind_json,
            bucket, confidentiality, original_date, title, summary, native_bytes
        ) VALUES (
            'mob-acct-1', 'rec-bad-conf', 1, 1, '{}'::jsonb,
            'Notes', 'Unclassified', '2026-01-12', 'Bad', 'Bad', decode('7b7d', 'hex')
        );
        RAISE EXCEPTION 'expected CHECK violation for invalid confidentiality tag';
    EXCEPTION WHEN check_violation THEN
        NULL;
    END;

    BEGIN
        INSERT INTO push_device_tokens (device_id, account_id, platform, token)
        VALUES ('dev-bad', 'mob-acct-1', 'web_push_invalid', 'tok');
        RAISE EXCEPTION 'expected CHECK violation for invalid push platform';
    EXCEPTION WHEN check_violation THEN
        NULL;
    END;

    BEGIN
        INSERT INTO for_my_doctor_sends (
            send_id, idempotency_key, account_id, practice_id, snapshot_json, status, created_at_secs
        ) VALUES (
            'send-dup', 'fmd-idem-1', 'mob-acct-1', 'practice-1', '{}'::jsonb, 'sent', 1760000101
        );
        RAISE EXCEPTION 'expected UNIQUE violation for duplicate for_my_doctor_sends idempotency_key';
    EXCEPTION WHEN unique_violation THEN
        NULL;
    END;

    BEGIN
        INSERT INTO health_samples (
            account_id, idempotency_key, sample_id, metric, value_milli, unit, recorded_at_secs, platform
        ) VALUES (
            'mob-acct-1', 'hk-batch-1', 's-1', 'steps', 9000000, 'count', 1760000011, 'ios_health_kit'
        );
        RAISE EXCEPTION 'expected PK violation for duplicate health sample';
    EXCEPTION WHEN unique_violation THEN
        NULL;
    END;

    BEGIN
        INSERT INTO patient_records (
            account_id, record_id, incarnation, version, source_kind_json,
            bucket, confidentiality, original_date, title, summary, native_bytes
        ) VALUES (
            'mob-missing-acct', 'rec-orphan', 1, 1, '{}'::jsonb,
            'Notes', 'Standard', '2026-01-12', 'Orphan', 'Orphan', decode('7b7d', 'hex')
        );
        RAISE EXCEPTION 'expected FK violation for orphan patient_record';
    EXCEPTION WHEN foreign_key_violation THEN
        NULL;
    END;
END $$;
SQL

echo "PostgreSQL migration & constraint proof PASS (${PG_VERSION})"
