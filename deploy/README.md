# Production Containers and Local Multi-Service Stack

This directory holds the rootless OCI container definitions and local composition stack for the three headless Elysian Fields server binaries.

## Images

Each `Containerfile` uses a multi-stage Rust release build (`rust:1.99.1-slim-bookworm`) copied into a minimal `debian:bookworm-slim` runtime image running as non-root UID/GID `10001` with a single compiled binary, zero baked-in secrets, and built-in HTTP healthchecks:

| Containerfile | Binary | Port | Health / Readiness Probe | Prometheus Metrics (Zero-PHI) | Writable State Volumes |
| --- | --- | --- | --- | --- | --- |
| [`Containerfile.clinic`](./Containerfile.clinic) | `ef-clinic-server` | `8088` | `GET /elysianfields/` and `GET /elysianfields/api/health` | `GET /elysianfields/api/metrics` | `/var/lib/elysian/clinic` (plus read-only `/srv/elysianfields/web` mounted at `/elysianfields/`) |
| [`Containerfile.mobile`](./Containerfile.mobile) | `ef-mobile-server` | `8089` | `GET /elysianfields/mobile-api/readyz` | `GET /elysianfields/mobile-api/metrics` | `/var/lib/elysian/mobile-data` and independent `/var/lib/elysian/mobile-journal` |
| [`Containerfile.public`](./Containerfile.public) | `ef-public-site` | `8082` | `GET /health` and `GET /readyz` | `GET /metrics` | None (stateless) |

## Isolated PostgreSQL 18 Compose Stack

[`docker-compose.yml`](./docker-compose.yml) runs `postgres:18` alongside all three rootless server containers with `read_only: true` filesystems and `no-new-privileges:true`. On first database initialization, [`postgres-init/01-init-isolated-dbs.sh`](./postgres-init/01-init-isolated-dbs.sh) creates strictly isolated `ef_clinic` and `ef_mobile` databases and applies each product's owning SQL migrations directly from `crates/ef-clinic-server/migrations/` and `crates/ef-mobile-server/migrations/`.

```sh
docker compose -f deploy/docker-compose.yml up --build
```

## Unified Single-Origin Gateway

To front all three services behind a single hardened HTTP origin on `127.0.0.1:8080` using [`Caddyfile`](./Caddyfile) (`/elysianfields/api/*` -> `clinic:8088` with zero-buffer SSE flushing, `/elysianfields/mobile-api/*` -> `mobile:8089`, `/elysianfields/*` -> `clinic:8088`, and `/*` -> `public-site:8082`):

```sh
docker compose -f deploy/docker-compose.yml --profile gateway up --build
```

Provide real database credentials via environment variables (`EF_PG_USER`, `EF_PG_PASSWORD`) outside local development; never commit provider or database secrets into image layers or compose files.

## Local Native 3-Server Launcher

To build and launch all three live servers directly on localhost without containers (using ephemeral ports and temporary WAL/journal directories cleaned up on exit):

```sh
# Automated end-to-end smoke check (starts all 3 servers, verifies endpoints, exits cleanly)
./scripts/run_local_stack.sh --smoke

# Interactive local stack (stays running until Ctrl-C)
./scripts/run_local_stack.sh
```
