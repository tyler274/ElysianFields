#!/usr/bin/env bash
set -euo pipefail

# Self-contained live PostgreSQL 18 durability proof for `ef-pgwire`, `ef-clinic-server`,
# and `ef-mobile-server`.
# - Uses an external PG18 server when PGHOST is set (e.g. GitHub Actions postgres:18 service).
# - Otherwise initializes an ephemeral PostgreSQL 18 TCP cluster on 127.0.0.1 with BOTH
#   SCRAM-SHA-256 password auth and a trust-auth test role, runs
#   `cargo test --locked -p ef-pgwire --test live_postgres18_durability -- --nocapture`,
#   and cleans up cleanly on EXIT.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

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
  required_bins=(psql createdb dropdb cargo)
else
  required_bins=(initdb pg_ctl psql createdb cargo)
fi

for bin in "${required_bins[@]}"; do
  if ! command -v "${bin}" >/dev/null 2>&1; then
    echo "ERROR: required tool '${bin}' not found on PATH" >&2
    exit 1
  fi
done

WORK_DIR=""
PGDATA=""
PG_ADMIN_ARGS=()
PG_TCP_HOST="127.0.0.1"
PG_TCP_PORT=""
SCRAM_USER="ef_scram_user"
SCRAM_PASS="ef_scram_test_pass_2026"
TRUST_USER="ef_trust_user"

cleanup() {
  if [[ "${USE_EXTERNAL_PG}" -eq 1 ]]; then
    dropdb "${PG_ADMIN_ARGS[@]}" --if-exists ef_pgwire_probe_db >/dev/null 2>&1 || true
    dropdb "${PG_ADMIN_ARGS[@]}" --if-exists ef_pgwire_clinic_db >/dev/null 2>&1 || true
    dropdb "${PG_ADMIN_ARGS[@]}" --if-exists ef_pgwire_mobile_db >/dev/null 2>&1 || true
    dropdb "${PG_ADMIN_ARGS[@]}" --if-exists ef_pgwire_failover_db >/dev/null 2>&1 || true
    psql "${PG_ADMIN_ARGS[@]}" -d postgres -c "DROP ROLE IF EXISTS ${SCRAM_USER};" >/dev/null 2>&1 || true
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
  PG_TCP_HOST="${PGHOST}"
  PG_TCP_PORT="${PGPORT:-5432}"
  PG_ADMIN_ARGS=(-h "${PG_TCP_HOST}" -p "${PG_TCP_PORT}")
  if [[ -n "${PGUSER:-}" ]]; then
    PG_ADMIN_ARGS+=(-U "${PGUSER}")
  fi
  echo "==> Using external PostgreSQL server at ${PG_TCP_HOST}:${PG_TCP_PORT}"
  dropdb "${PG_ADMIN_ARGS[@]}" --if-exists ef_pgwire_probe_db >/dev/null 2>&1 || true
  dropdb "${PG_ADMIN_ARGS[@]}" --if-exists ef_pgwire_clinic_db >/dev/null 2>&1 || true
  dropdb "${PG_ADMIN_ARGS[@]}" --if-exists ef_pgwire_mobile_db >/dev/null 2>&1 || true
  dropdb "${PG_ADMIN_ARGS[@]}" --if-exists ef_pgwire_failover_db >/dev/null 2>&1 || true
  psql "${PG_ADMIN_ARGS[@]}" -d postgres -v ON_ERROR_STOP=1 <<SQL >/dev/null
SET password_encryption = 'scram-sha-256';
DROP ROLE IF EXISTS ${SCRAM_USER};
CREATE ROLE ${SCRAM_USER} LOGIN PASSWORD '${SCRAM_PASS}' SUPERUSER;
SQL
  export EF_PG_TEST_URL="postgres://${SCRAM_USER}:${SCRAM_PASS}@${PG_TCP_HOST}:${PG_TCP_PORT}/ef_pgwire_probe_db"
  export EF_PG_CLINIC_URL="postgres://${SCRAM_USER}:${SCRAM_PASS}@${PG_TCP_HOST}:${PG_TCP_PORT}/ef_pgwire_clinic_db"
  export EF_PG_MOBILE_URL="postgres://${SCRAM_USER}:${SCRAM_PASS}@${PG_TCP_HOST}:${PG_TCP_PORT}/ef_pgwire_mobile_db"
  export EF_PG_FAILOVER_URL="postgres://${SCRAM_USER}:${SCRAM_PASS}@${PG_TCP_HOST}:${PG_TCP_PORT}/ef_pgwire_failover_db"
else
  if [[ "$(id -u)" -eq 0 ]]; then
    echo "ERROR: run verify_pgwire_live.sh as a non-root user (initdb forbids root)" >&2
    exit 1
  fi

  WORK_DIR="$(mktemp -d -t ef-pgwire-live-XXXXXX)"
  PGDATA="${WORK_DIR}/pgdata"
  PGSOCK="${WORK_DIR}/sock"
  PGLOG="${WORK_DIR}/postgres.log"
  mkdir -p "${PGSOCK}"

  # Pick a free ephemeral TCP port on 127.0.0.1.
  PG_TCP_PORT="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()' 2>/dev/null || echo $((25432 + ($$ % 10000))))"

  echo "==> Initializing disposable PostgreSQL cluster in ${PGDATA} (TCP 127.0.0.1:${PG_TCP_PORT})"
  initdb -D "${PGDATA}" --no-locale --encoding=UTF8 --auth=trust >/dev/null

  cat > "${PGDATA}/pg_hba.conf" <<HBA
local   all             all                                     trust
host    all             ${TRUST_USER}   127.0.0.1/32            trust
host    all             all             127.0.0.1/32            scram-sha-256
host    all             all             ::1/128                 scram-sha-256
HBA

  pg_ctl -D "${PGDATA}" -l "${PGLOG}" -o "-k ${PGSOCK} -h 127.0.0.1 -p ${PG_TCP_PORT} -F" start >/dev/null
  PG_ADMIN_ARGS=(-h "${PGSOCK}" -p "${PG_TCP_PORT}")

  psql "${PG_ADMIN_ARGS[@]}" -d postgres -v ON_ERROR_STOP=1 <<SQL >/dev/null
SET password_encryption = 'scram-sha-256';
CREATE ROLE ${SCRAM_USER} LOGIN PASSWORD '${SCRAM_PASS}' SUPERUSER;
CREATE ROLE ${TRUST_USER} LOGIN SUPERUSER;
SQL

  export EF_PG_TEST_URL="postgres://${SCRAM_USER}:${SCRAM_PASS}@127.0.0.1:${PG_TCP_PORT}/ef_pgwire_probe_db"
  export EF_PG_TRUST_URL="postgres://${TRUST_USER}@127.0.0.1:${PG_TCP_PORT}/ef_pgwire_probe_db"
  export EF_PG_CLINIC_URL="postgres://${SCRAM_USER}:${SCRAM_PASS}@127.0.0.1:${PG_TCP_PORT}/ef_pgwire_clinic_db"
  export EF_PG_MOBILE_URL="postgres://${SCRAM_USER}:${SCRAM_PASS}@127.0.0.1:${PG_TCP_PORT}/ef_pgwire_mobile_db"
  export EF_PG_FAILOVER_URL="postgres://${SCRAM_USER}:${SCRAM_PASS}@127.0.0.1:${PG_TCP_PORT}/ef_pgwire_failover_db"
fi

PG_VERSION="$(psql "${PG_ADMIN_ARGS[@]}" -d postgres -v ON_ERROR_STOP=1 -tAc "SELECT version();")"
echo "==> Live PostgreSQL ready: ${PG_VERSION}"

createdb "${PG_ADMIN_ARGS[@]}" -O "${SCRAM_USER}" ef_pgwire_probe_db
createdb "${PG_ADMIN_ARGS[@]}" -O "${SCRAM_USER}" ef_pgwire_clinic_db
createdb "${PG_ADMIN_ARGS[@]}" -O "${SCRAM_USER}" ef_pgwire_mobile_db
createdb "${PG_ADMIN_ARGS[@]}" -O "${SCRAM_USER}" ef_pgwire_failover_db

echo "==> Running ef-pgwire live PostgreSQL 18 durability suite"
(cd "${REPO_ROOT}" && cargo test --locked -p ef-pgwire --test live_postgres18_durability -- --nocapture)

echo "Live PostgreSQL 18 ef-pgwire durability proof PASS (${PG_VERSION})"
