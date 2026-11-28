# CI, Verification, and Qualification Status

Elysian Fields pairs a single local verification entry point with scoped, time-budgeted pull-request CI and an independent 7-job Cross-Platform Qualification pipeline.

## Local Verification (`scripts/dev-check.sh`)

Run the same checks locally before pushing:

```sh
scripts/dev-check.sh --tier touched      # Changed crates + Python tests + doc/wall checks (default)
scripts/dev-check.sh --tier pre-commit   # Staged formatting, Clippy, and script tests
scripts/dev-check.sh --tier full         # Full 28-crate fmt, host + wasm32 Clippy (-D warnings),
                                         # cargo test --locked, Python tests, wall/ledger/disposition,
                                         # mdbook build, and offline lychee link checks
```

Workspace-wide lints in `Cargo.toml` (`disallowed_methods`, `fn_params_excessive_bools`, `map_err_ignore`, `struct_excessive_bools`, `unused_result_ok`) run with `-D warnings` on both host and `wasm32-unknown-unknown` targets. Supply-chain compliance is verified by `cargo deny --all-features check advisories bans licenses sources` against `deny.toml`.

## Fast Pull-Request CI (`.github/workflows/ci.yml`)

- **Per-Deployable Scoping (`scripts/ci_scope.py`)**: Classifies changed paths into `docs_only`, `native`, `wasm`, `desktop`, `mobile`, `server`, `client_only`, and `full`, and computes isolated `touched_crates` on PRs.
- **Headless / Desktop Split**: Splits `cargo clippy` and `cargo test` into `--workspace --exclude ef-*-desktop` followed by `-p ef-*-desktop` so desktop GPU/render Bevy features (`wgpu`, `naga`, `bevy_render`) never unify into headless crates.
- **Trimmed Dev Debuginfo**: `[profile.dev]` uses `debug = "line-tables-only"` and `[profile.dev.package."*"]` uses `debug = false`, keeping debug test binaries compact and fast to link across all three desktop OSes.
- **Offline mdBook Integrity Gate (`scripts/check_book.py`)**: Validates `book/book.toml`, `book/src/SUMMARY.md` chapter existence and non-emptiness, every relative markdown link across `book/src/**/*.md`, zero `TYL-` issue-tracker IDs, and zero dated changelog headings; enforced in both `.github/workflows/ci.yml` (`lightweight` job) and `scripts/dev-check.sh`.
- **Supply-Chain & SBOM Verification (`cargo-deny`, `scripts/generate_sbom.py`)**: Enforces advisories, bans, licenses, and sources on PRs, `main`, and pre-release (`release.yml`), and generates/verifies a deterministic 509-component CycloneDX 1.5 SBOM (`python3 scripts/generate_sbom.py --verify`).
- **Hard Time Budgets (`docs/ci/budgets.json`, `scripts/ci_timings.py`)**: Every CI run renders an HTML timing report and fails the `result` gate if any job or total exceeds its budget. Client-only merges to `main` package `/elysianfields/` static assets without rebuilding server images.

## 7-Job Cross-Platform Qualification (`.github/workflows/cross-platform.yml`)

Triggered on `workflow_dispatch`, weekly schedule, and PRs touching workflow, deployment, platform, desktop, web, or QA paths:

1. **`desktop-macos`** (`macos-15`, Apple Silicon ARM64): full 28-crate tests, native release server binaries, `ef-clinic-desktop` + `ef-mobile-desktop` release builds + `--headless-smoke` + `.app` packaging, `aarch64-apple-ios-sim` cross-check, `swiftc -typecheck` + `swift test`, and bounded `ElyMalloc` probe.
2. **`desktop-windows`** (`windows-latest`, x86_64 MSVC): full 28-crate tests, native `.exe` server binaries, `ef-clinic-desktop.exe` + `ef-mobile-desktop.exe` release builds + `--headless-smoke` + `.zip` packaging, and bounded `ElyMalloc` probe.
3. **`desktop-linux-native`** (`ubuntu-24.04`): `ef-clinic-desktop` + `ef-mobile-desktop` release builds + `--headless-smoke` + `.tar.gz` packaging.
4. **`postgres-18`** (`ubuntu-24.04` + `postgres:18` service): isolated Clinic/Mobile SQL migrations, idempotency, constraint/cascade checks, and live `ef-pgwire` durability & two-instance advisory-lock failover (`scripts/verify_pgwire_live.sh`).
5. **`browsers-e2e`** (`ubuntu-24.04` + Noto fonts + Playwright Chromium, Firefox, WebKit): 3-browser E2E covering the 4-tab Clinic browser workstation, Mobile Chromium/WebKit PWA Health Trends & offline SW, compiled Bevy Wasm `WebAssembly.compile` under strict CSP + Brotli-11 sizes, 12-locale `< 20 KiB` MV3 popup, 42-view 7-script multilingual QA, and WCAG 2.1 AA theme/zoom/Privacy Cover/Dei UX QA.
6. **`mobile-android-build`** (`ubuntu-24.04`): `aarch64-linux-android` NDK cross-check across mobile crates plus `kotlinc` + `java` execution of Android shim tests.
7. **`oci-container-build`** (`ubuntu-24.04`): builds and smoke-tests all three rootless `USER 10001:10001` OCI images (`public`, `clinic`, `mobile`) and runs the live 5-container Docker Compose + Caddy gateway E2E (`scripts/verify_compose_stack.sh --no-build`, covering searchable `/dei`, XSS escaping, and authenticated `/elysianfields/api/today`).

## Honest Qualification Status and Hardware Gates

`specs/ledger.toml` and `docs/m5/disposition.json` track every requirement and surface with automated validation (`scripts/check_ledger.py`, `scripts/check_disposition.py`):

- **Qualified**: Linux, macOS ARM64, and Windows MSVC desktop & server suites; PostgreSQL 18 migrations and live `ef-pgwire` failover; Chromium, Firefox, and WebKit desktop + mobile-emulated web/PWA shells; compiled Wasm under strict CSP; Chrome MV3 popup; rootless OCI containers and Caddy gateway composition.
- **Honestly Hardware-Gated (`blocked-no-hardware`)**: Physical on-device Android (`android-native`: Health Connect, Keystore hardware backing, FCM, TalkBack) and physical on-device iOS (`ios-native`: HealthKit, Keychain Secure Enclave, APNs, VoiceOver) remain marked `blocked` until executed on real mobile hardware; cross-compilation and unit/shim tests never substitute for on-device runtime proof.
