# Workspace Crate Map

The workspace comprises **28 Rust crates** sharing edition `2024`, a single pinned toolchain (`rust-toolchain.toml`), and workspace-wide Clippy lints (`[workspace.lints.clippy]` in `Cargo.toml`).

## Foundation and Qualification Probes

| Crate | Target Surface | Ownership |
| --- | --- | --- |
| `elysian-smoke` | Native + Wasm | Minimal workspace smoke crate validating host and `wasm32-unknown-unknown` builds and lints. |
| `elysian-bevy-probe` | Native + Wasm | Minimal Bevy `0.20.1` + AccessKit `0.24.1` compilation and dependency baseline probe. |
| `m1-accessibility` | Native + Wasm | Single-owner AccessKit tree projection, modal focus trapping, and keyboard navigation contracts. |
| `m1-streaming-perf` | Native + Wasm | Virtualized streaming transcript benchmark, 5,000-row lab table, idle redraw zero-frame proof, and 1-hour soak harness. |
| `m1-text-i18n` | Native + Wasm | 12-locale catalog + 5 complex-script shaping probes, grapheme/cluster boundaries (Hebrew Nikud, Arabic Harakat, Devanagari conjuncts, CJK, Vietnamese), and BiDi isolation. |
| `m1-mobile-lifecycle` | Native + Wasm | Compact `EFSS` v1 binary state-restoration protocol, CRC-32 validation, and mobile background/foreground lifecycle transitions. |
| `m1-web-delivery` | Native + Wasm | WebGPU/WebGL2 backend detection, in-place context-loss recovery, `/elysianfields` subpath resolution, and strict CSP verifier. |

## Shared Domain, UI Catalog, and Persistence

| Crate | Target Surface | Ownership |
| --- | --- | --- |
| `ef-core` | Native + Wasm | Typed identifiers (`WorkspaceId`, `PhysicianId`, `PracticeId`, `SourceKey`, `SessionId`), shared locales, core error types, and zero-PHI telemetry (`telemetry`: structured JSON access logs, `X-Request-Id`, `ShutdownSignal`). |
| `ef-ui-catalog` | Native + Wasm | Compile-time exhaustive 12-locale UI catalog (`107` keys × `12` locales = `1,284` strings), `LocalizedChrome`, WCAG 2.1 AA Dark/Light color tokens, multiline `ComposerSurface`, and OS-native desktop preferences (`prefs`). |
| `ef-connected-care` | Native + Wasm | Consent-gated invitation, DOB verification (with permanent lockout), explicit Share, 42 CFR Part 2 notice enforcement, and disconnection bridge between Clinic and Mobile. |
| `ef-harness` | Headless native | Product-neutral streaming LLM orchestration, retry/backoff state machine, idempotency receipts, and stub provider for deterministic tests. |
| `ef-pgwire` | Headless native | Zero-dependency PostgreSQL Wire Protocol v3.0 client with FIPS-180-4 SHA-256, `SCRAM-SHA-256`, parameterized Extended Query, transactions, `ping()`, and session advisory locks. |
| `ef-boundaries` | Test-only native | Automated architecture boundary verifier enforcing product isolation, headless server zero-Bevy rules, product-neutral `ef-harness`, and client secret scanning. |

## Curie Clinic

| Crate | Target Surface | Ownership |
| --- | --- | --- |
| `ef-clinic-domain` | Native + Wasm | Authoritative Clinic auth, sessions, anti-enumeration sign-in, patient workspaces, cited sources, and streaming chat state machines. |
| `ef-clinic-workflows` | Headless native | Today queue (`TodaySaysSo`), MessageMatching (`ingest`/`decide`/`undo`), MessageReply draft/send lifecycle, and PatientMemory workflows. |
| `ef-clinical-lib` | Headless native | De-identified clinical reference library and `ConceptQuery` PHI fail-closed validator. |
| `ef-encoder` | Headless native | Offline 256-dimensional workspace-scoped semantic chart search index with zero cross-patient leakage. |
| `ef-patient-bank` | Headless native | Synthetic clinical patient fixtures and deterministic corpus generator for integration testing and migration rehearsal. |
| `ef-clinic-server` | Headless binary | HTTP/1.1 + SSE server under `/elysianfields/` with PostgreSQL 18 / WAL durability, advisory-lock failover, rate limiting, static web serving, `/api/health`, zero-PHI `/api/metrics`, structured JSON access logs (`EF_ACCESS_LOG_JSON=1`), `X-Request-Id`, and graceful `ShutdownSignal` drain. |
| `ef-clinic-client` | Native + Wasm | Bevy `0.20.1` Curie Clinic client plugin, pure-`TcpStream` `LiveHttpClinicTransport`, and zero-dep Wasm `cdylib` ABI (`wasm_abi`). |
| `ef-clinic-desktop` | Native desktop binary | Standalone windowed Bevy `0.20.1` desktop GPU application for Curie Clinic across Linux, macOS, and Windows with progressive token-by-token SSE streaming, mid-flight `WorkStop` cancel/retry, `Ctrl+K` roster search, and live TCP worker E2E. |

## Curie Mobile

| Crate | Target Surface | Ownership |
| --- | --- | --- |
| `ef-mobile-domain` | Native + Wasm | Standalone patient accounts, incarnation fences, privacy-receipt non-resurrection journal, portal sync, HealthKit/Health Connect outbox, and For My Doctor payloads. |
| `ef-mobile-server` | Headless binary | HTTP/1.1 service under `/elysianfields/mobile-api/` with isolated PostgreSQL 18 / WAL storage, fail-closed `/readyz` restore-replay gate, Connected Care endpoints, zero-PHI `/metrics`, structured JSON access logs, `X-Request-Id`, and graceful `ShutdownSignal` drain. |
| `ef-mobile-client` | Native + Wasm | Bevy `0.20.1` Curie Mobile client plugin, pure-`TcpStream` `LiveHttpMobileTransport`, and zero-dep Wasm `cdylib` ABI (`wasm_abi`). |
| `ef-mobile-desktop` | Native desktop binary | Standalone windowed Bevy `0.20.1` mobile-proportioned desktop application for Curie Mobile across Linux, macOS, and Windows with interactive 4-field forms, `OwedRecoveryBannerState` + Flush Owed Sign-Outs, and live TCP worker E2E. |

## Public Site, Browser Extension, and Migration Rehearsal

| Crate | Target Surface | Ownership |
| --- | --- | --- |
| `ef-public-site` | Headless binary | Standalone HTTP/1.1 server rendering JavaScript-free semantic HTML for `/`, `/mobile`, `/privacy`, `/terms`, `/clinical-transparency`, `/invite` (`noindex`), `/dei` crisis-first recovery finder, `/health`, `/readyz`, `/metrics`, structured JSON access logs, `X-Request-Id`, and graceful `ShutdownSignal` drain. |
| `ef-extension` | Headless native | Chrome MV3 connector pairing state machine, memory-only session semantics, and strict CSP/manifest verification. |
| `ef-migration-rehearsal` | Headless native | Shadow-mode diff parity, single-writer domain fencing, pending-job drain/rollback, privacy-receipt replay, 468-patient crash-resume, and live 3-server cross-product TCP E2E suite. |
