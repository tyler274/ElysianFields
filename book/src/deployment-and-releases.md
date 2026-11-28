# Deployment Containers and Releases

Elysian Fields ships rootless OCI server images, a 5-container local/staging Docker Compose stack behind a unified Caddy gateway, and deterministic 3-OS desktop + Wasm release bundles.

## Rootless Multi-Stage OCI Images (`deploy/`)

Each server has a dedicated multi-stage Containerfile compiling a single release binary with `rust:1.99.1-slim-bookworm` and copying only that binary (plus `/elysianfields/` static web assets for Clinic) into `debian:bookworm-slim` running as non-root `USER 10001:10001`:

| Image Definition | Binary | Port | Health / Readiness Probe |
| --- | --- | --- | --- |
| `deploy/Containerfile.clinic` | `ef-clinic-server` | `8088` | `GET /elysianfields/` |
| `deploy/Containerfile.mobile` | `ef-mobile-server` | `8089` | `GET /elysianfields/mobile-api/readyz` |
| `deploy/Containerfile.public` | `ef-public-site` | `8082` | `GET /health` & `GET /readyz` |

Images contain zero embedded secrets. Runtime containers run with `--read-only`, `--user 10001:10001`, `--cap-drop ALL`, and `--security-opt no-new-privileges:true`, writing WAL/journal state only to dedicated volumes or UID-10001 tmpfs mounts.

## 5-Container Compose Stack and Unified Caddy Gateway

`deploy/docker-compose.yml` composes `postgres:18` (initializing strictly isolated `ef_clinic` and `ef_mobile` databases via `deploy/postgres-init/01-init-isolated-dbs.sh`), `clinic`, `mobile`, `public-site`, and an optional hardened `gateway` (`caddy:2-alpine`, `--profile gateway`) configured by `deploy/Caddyfile`:

- Single origin `http://127.0.0.1:8080` routes:
  - `/elysianfields/mobile-api/*` → `mobile:8089`
  - `/elysianfields/api/*` → `clinic:8088` (with `flush_interval -1` for zero-buffered SSE)
  - `/elysianfields` & `/elysianfields/*` → `clinic:8088`
  - `/*` → `public-site:8082`
- Attaches `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`, and `Referrer-Policy: no-referrer` on every response.
- Verified end-to-end (including live Clinic auth write, PostgreSQL 18 WAL row check, container restart replay, and zero-PHI `/metrics`) via `scripts/verify_compose_stack.sh`.

For container-free local testing, `scripts/run_local_stack.sh --smoke` launches all three native server binaries on ephemeral localhost ports with a temporary WAL directory and tears them down cleanly on exit.

## 3-OS Desktop Packaging and Automated Releases

`scripts/package_desktop.py` (pure Python stdlib, deterministic metadata, `--verify` SHA-256 verification) packages compiled `ef-clinic-desktop` and `ef-mobile-desktop` release binaries alongside clean-room SVG icons (`assets/desktop/curie-{clinic,mobile}.svg`):

- **`linux-x86_64`**: `elysian-fields-desktop-<ver>-linux-x86_64.tar.gz` containing `bin/ef-{clinic,mobile}-desktop` (`0755`), Freedesktop `.desktop` entries, SVG icons, and a rootless `~/.local` `install.sh`.
- **`macos-arm64`**: `elysian-fields-desktop-<ver>-macos-arm64.tar.gz` containing `Curie Clinic.app` and `Curie Mobile.app` bundles with valid XML `Info.plist` (`NSHighResolutionCapable=true`, `LSMinimumSystemVersion=13.0`).
- **`windows-x86_64`**: `elysian-fields-desktop-<ver>-windows-x86_64.zip` containing `ef-{clinic,mobile}-desktop.exe`, `.cmd` double-click launchers, and `README-Windows.txt`.

`.github/workflows/release.yml` runs on `v*.*.*` tag pushes and manual `workflow_dispatch`, builds and smoke-tests all three desktop bundles plus the `/elysianfields/` Wasm static client archive, aggregates and verifies unified `SHA256SUMS.txt`, and publishes the GitHub Release on version tags.
