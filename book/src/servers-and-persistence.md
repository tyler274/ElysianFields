# Headless Servers and PostgreSQL Persistence

All three backend binaries (`ef-clinic-server`, `ef-mobile-server`, and `ef-public-site`) are strictly headless native Rust HTTP/1.1 servers built directly on `std::net::TcpListener` with zero async runtime bloat and zero third-party web frameworks.

## Pure-Rust PostgreSQL Wire Protocol (`ef-pgwire`)

`crates/ef-pgwire` implements PostgreSQL Wire Protocol v3.0 in pure Rust (zero external database driver or crypto crates):

- **Authentication**: `trust`, `cleartext`, and RFC 5802 / RFC 7677 `SCRAM-SHA-256` backed by pure-Rust FIPS-180-4 SHA-256, HMAC-SHA-256, PBKDF2-HMAC-SHA-256, Base64, and constant-time comparison. Connection passwords are always redacted as `[REDACTED]` in `Debug` output.
- **Parameterized Extended Query**: Every write and lookup uses `Parse`/`Bind`/`Describe`/`Execute`/`Sync` with typed `PgParam` values — zero SQL string interpolation.
- **Transactions, Health, and Advisory Locks**: Provides `in_transaction`, `ping()` (`SELECT 1`), and parameterized session advisory-lock helpers (`try_advisory_lock`, `advisory_lock`, `advisory_unlock`) for single-writer fencing across replicas.

## Isolated Databases and SQL Migrations

Product separation extends through the database layer:

| Product | Database | Owning Migrations | Durable WAL Table |
| --- | --- | --- | --- |
| Curie Clinic (`ef-clinic-server`) | `ef_clinic` | `crates/ef-clinic-server/migrations/0001_m2_clinic_slice.sql` | `ef_clinic_wal_events` |
| Curie Mobile (`ef-mobile-server`) | `ef_mobile` | `crates/ef-mobile-server/migrations/0001_m3_mobile.sql` | `ef_mobile_wal_events` |

Migrations are idempotent (`IF NOT EXISTS`, `ON CONFLICT DO NOTHING`) and verified against live PostgreSQL 18 via `scripts/verify_postgres_migrations.sh` and `scripts/verify_pgwire_live.sh`.

## Multi-Instance Failover, SSE Resume, and Readiness Gates

- **Curie Clinic (`ef-clinic-server`)**:
  - Connects to PostgreSQL 18 when `EF_CLINIC_DATABASE_URL` (or `DATABASE_URL`) is set, or falls back to local file WAL (`EF_CLINIC_DATA_DIR`) for offline/embedded testing.
  - Uses PostgreSQL session advisory locks to serialize mutations across concurrent instances and incrementally replays `WHERE seq > $1` WAL rows before reads.
  - Streams LLM responses via Server-Sent Events (`text/event-stream`) with monotonic `id:` fields supporting zero-duplicate, zero-gap `Last-Event-ID` resume across instance failover.
  - Enforces per-IP sliding-window rate limiting (`429 Too Many Requests` + `Retry-After`), 64 KiB auth body caps (`413`), and fail-closed `GET /elysianfields/api/health` (`200` when DB healthy, `503` when backing store is unreachable).
- **Curie Mobile (`ef-mobile-server`)**:
  - Maintains an independent privacy-receipt journal (`EF_MOBILE_JOURNAL_DIR`) separate from primary state (`EF_MOBILE_DATA_DIR` / `EF_MOBILE_DATABASE_URL`).
  - `GET /elysianfields/mobile-api/readyz` fails closed (`503`) if schema migrations, restore-replay verification, or backing database health checks fail, preventing resurrection of deleted accounts after backup restoration.
- **Public Site (`ef-public-site`)**:
  - Strictly stateless server rendering semantic HTML with zero JavaScript requirement.
  - Guarantees crisis helplines (`988`, `911`, `SAMHSA`) render before facility listings on `/dei`, marks `/invite` with `noindex, nofollow`, blocks `/grafana/` with `404` + `no-store`, and exposes `/health` and `/readyz`.

## Zero-PHI Prometheus Observability

Every server exposes a zero-dependency Prometheus text exposition endpoint (`text/plain; version=0.0.4; charset=utf-8`, `Cache-Control: private, no-store, max-age=0`) carrying only low-cardinality operational metrics and strictly zero PHI, emails, tokens, or patient identifiers:

- **Clinic**: `GET /elysianfields/api/metrics` (`ef_clinic_http_requests_total{status_class=...}`, `ef_clinic_sse_streams_total`, `ef_clinic_wal_seq`, `ef_clinic_uptime_seconds`, `ef_clinic_db_ready`).
- **Mobile**: `GET /elysianfields/mobile-api/metrics` (`ef_mobile_http_requests_total{status_class=...}`, `ef_mobile_wal_seq`, `ef_mobile_uptime_seconds`, `ef_mobile_db_ready`).
- **Public**: `GET /metrics` (`ef_public_http_requests_total{status_class=...}`, `ef_public_uptime_seconds`).
