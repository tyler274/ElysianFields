#!/usr/bin/env bash
# Initializes strictly isolated Clinic and Mobile databases inside PostgreSQL 18
# and applies each product's owning SQL migrations from mounted read-only dirs.
set -euo pipefail

psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" <<'SQL'
CREATE DATABASE ef_clinic;
CREATE DATABASE ef_mobile;
SQL

for sql_file in /migrations/clinic/*.sql; do
  [ -f "$sql_file" ] || continue
  psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname ef_clinic -f "$sql_file"
done

for sql_file in /migrations/mobile/*.sql; do
  [ -f "$sql_file" ] || continue
  psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname ef_mobile -f "$sql_file"
done
