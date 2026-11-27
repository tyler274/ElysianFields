# Elysian Fields Scope Inventory

Complete finite scope classification of the pinned `dabney-moe` reference (`01b762838d639a51f82e9d7d15e788e02bac772f`). Every workspace member (all **43** crates in root `Cargo.toml`), native `platform/` bridge, Chrome MV3 extension component, public route family, and background service/worker is assigned exactly one classification:

- **`replace`**: Behavioral clean-room reimplementation behind the wall in the Elysian Realm (Bevy for interactive clients; headless Rust for services/backends/harness; semantic HTML for public/legal web; ordinary JS/TS for extension worker/content scripts).
- **`retain-as-tooling`**: Development, CI, build, verification, sandbox, or operator observability tooling retained outside product replacement.
- **`reuse-as-approved-asset`**: Non-code static data, declarative platform metadata, or brand/legal assets reused directly after gatekeeper approval.
- **`out-of-scope`**: Distinct standalone surface not part of the Curie Clinic / Curie Mobile clinical replacement; continues independent operation with **zero surface disappearance** (flagged explicitly for M0 owner sign-off).

Machine-readable companion: [`scope-inventory.json`](scope-inventory.json) (programmatically verified 1-to-1 against all 43 members in root `Cargo.toml` with zero omissions or duplicates).

## Summary Counts (43 Workspace Members)

| Classification | Count | Members |
| --- | ---: | --- |
| `replace` | 34 | All Curie Clinic, Curie Mobile, Bridge, Shared product, Public Site, Dei, and Patient Bank crates |
| `retain-as-tooling` | 6 | `android`, `availability-probe`, `boundaries`, `observability-relay`, `workspace-tools`, `mole-sandbox` |
| `reuse-as-approved-asset` | 0 | *(No Rust code crates are reused under D1; approved assets are listed below and in `allowed-inputs.md`)* |
| `out-of-scope` | 3 | `ancestry`, `ancestry-app`, `ancestry-fit` *(standalone `dabney.moe/dux/` population-genetics tool; flagged for M0 owner confirmation)* |
| **Total** | **43** | **100% of root `Cargo.toml` `workspace.members`** |

*Note on documentation drift observed in reference:* `book/src/crate-map.md` states "40 Rust crates" in its opening sentence (and `book/src/clean-room-reclamation.md` cites "38 workspace crates"), while root `Cargo.toml` and `EXPORT-MANIFEST.json` at `01b762838` contain **43** members. This inventory covers all 43.

## 1. All 43 Workspace Members

| # | Path | Package | Zone | Class | Justification |
| --- | --- | --- | --- | --- | --- |
| 1 | `crates/agent-skills` | `agent-skills` | Shared | `replace` | Runtime clinical skill catalog loader & manifest validator; reimplemented under D1. |
| 2 | `crates/ancestry` | `ancestry` | Ancestry | `out-of-scope` | Pure-Rust population-genetics engine with zero clinical dependency; stays on independent `dabney.moe/dux/` deploy (owner sign-off required). |
| 3 | `crates/ancestry-app` | `ancestry-app` | Ancestry | `out-of-scope` | Standalone `dabney.moe/dux/` web utility; separate from clinical care suite, no surface removal (owner sign-off required). |
| 4 | `crates/ancestry-fit` | `ancestry-fit` | Ancestry | `out-of-scope` | Offline native fitter/packer for G25 weights and CC0 AADR packs; belongs solely to standalone ancestry tool. |
| 5 | `crates/android` | `android` | Tooling | `retain-as-tooling` | Android AVD manager, APK build orchestration, and headless emulator runner; retained and adapted for Bevy Android APKs. |
| 6 | `crates/availability-probe` | `availability-probe` | Tooling | `retain-as-tooling` | Secret-free Cloud Run uptime/readiness probe binary; ops infrastructure with zero product logic. |
| 7 | `crates/boundaries` | `boundaries` | Tooling | `retain-as-tooling` | Architectural boundary & cross-product isolation verifier; retained and extended to replacement crates. |
| 8 | `crates/clinical-library` | `clinical-library` | Shared | `replace` | Product-neutral clinical literature store, nightly failure rules, and PubMed status pipeline; in scope under D1. |
| 9 | `crates/dei-site` | `dei-site` | Tooling | `replace` | Static site generator for Dei recovery finder (`dei.dabney.moe`); replaced as semantic HTML + clean-room search island. |
| 10 | `crates/encoder` | `encoder` | Shared | `replace` | Offline local semantic chart retrieval encoder & canonical serialization; in scope under D1. |
| 11 | `crates/extension-popup` | `extension-popup` | Shared | `replace` | Chrome MV3 popup UI; candidate for Bevy Wasm under strict MV3 CSP subject to M1 gate (narrow HTML fallback per D2). |
| 12 | `crates/front-door` | `front-door` | Bridge | `replace` | Connected-care authority: practice invitations, DOB verification, Share consent, link registry, deletion journal; in scope under D1. |
| 13 | `crates/gmail` | `gmail` | Shared | `replace` | Product-neutral Gmail API / SMTP AUTH adapter for sign-in links and clinic messages; in scope under D1. |
| 14 | `crates/harness` | `harness` | Shared | `replace` | Product-neutral bounded agent loop, provider/tool interfaces, `skill_view`, cancellation, quotas; in scope under D1. |
| 15 | `crates/http` | `curie-http` | Shared | `replace` | Shared HTTP client abstractions, IAP validation, and `IngressGuard` transport policy; in scope under D1. |
| 16 | `crates/local-product` | `local-product` | Physician | `replace` | Physician domain runtime, PostgreSQL store, source adapters, rounds, and clinical chat workflows; core backend under D1. |
| 17 | `crates/mobile` | `mobile` | Bridge | `replace` | Combined Android/iOS/web mode shell mounting Curie Mobile and physician views; replaced by Bevy mobile shell. |
| 18 | `crates/mobile-api` | `mobile-api` | Curie Mobile | `replace` | Patient backend API, migrations, privacy receipt delivery, and restore journal replay; core backend under D1. |
| 19 | `crates/mobile-app` | `mobile-app` | Curie Mobile | `replace` | Curie Mobile patient UI client (`You`, `For my doctor`, records, trends, sharing, account); replaced by Bevy client. |
| 20 | `crates/mobile-client` | `mobile-client` | Curie Mobile | `replace` | Typed HTTP/SSE client and local retention logic for `mobile-api`; replaced under D1. |
| 21 | `crates/mobile-health` | `mobile-health` | Curie Mobile | `replace` | On-device HealthKit/Health Connect sync, canonical mapping, and account-fenced transactional outbox; replaced under D1. |
| 22 | `crates/mobile-notify` | `mobile-notify` | Curie Mobile | `replace` | Push notification registration, APNs/FCM token handling, and withdrawal logic; replaced under D1. |
| 23 | `crates/mobile-stub` | `mobile-stub` | Curie Mobile | `replace` | Local offline stub backend for Curie Mobile development/tests; replaced behind the wall against new API contracts. |
| 24 | `crates/mobile-wire` | `mobile_wire` | Curie Mobile | `replace` | Curie Mobile wire models and OpenAPI DTOs; replaced with clean-room contract types under D1. |
| 25 | `crates/observability-relay` | `observability-relay` | Tooling | `retain-as-tooling` | Secret-free OTLP and staging Grafana IAP proxy service; operator observability tooling with zero product logic. |
| 26 | `crates/patient-bank` | `patient-bank` | Tooling | `replace` | Synthetic patient cohort generator, authoring checks, FHIR R4 builder, and scenario catalog; in scope per plan Section 3. |
| 27 | `crates/patient-work` | `patient-work` | Physician | `replace` | Patient workspace state machine, work grouping window, chat turns, reviews, and message workflows; replaced under D1. |
| 28 | `crates/physician-app` | `physician-app` | Physician | `replace` | Curie Clinic physician UI client (Today, workspace, streaming chat, citations, chart reader, message matching/reply); replaced by Bevy client. |
| 29 | `crates/physician-auth` | `physician-auth` | Physician | `replace` | Physician invite-only accounts, practice memberships, panel access, single-use links, PostgreSQL auth store; replaced under D1. |
| 30 | `crates/physician-bus` | `physician-bus` | Physician | `replace` | Lossy Valkey/Redis pub/sub invalidation bus for cross-instance physician workspace fan-out; replaced under D1. |
| 31 | `crates/physician-client` | `physician-client` | Physician | `replace` | Typed HTTP/SSE client for the physician API; replaced under D1. |
| 32 | `crates/physician-demo-api` | `physician-demo-api` | Physician | `replace` | Embeddable synthetic physician demo API with turn budgets and cloud/local GGUF inference; in scope under public site & demo. |
| 33 | `crates/physician-demo-db` | `physician-demo-db` | Physician | `replace` | Isolated per-visitor in-memory SQLite demo chart store and verbatim citation matcher; in scope under public site & demo. |
| 34 | `crates/physician-sample` | `physician-sample` | Physician | `replace` | Physician HTTP/SSE server, background worker (`physician-worker`), rounds job, and composite migrator; core backend under D1. |
| 35 | `crates/physician-wire` | `physician-wire` | Physician | `replace` | Shared physician wire DTOs; replaced with clean-room contract types under D1. |
| 36 | `crates/public-site` | `public-site` | Tooling | `replace` | Public marketing site, legal docs, recovery pages, interactive demo host, static export; replaced as semantic indexable HTML. |
| 37 | `crates/recovery-directory` | `recovery-directory` | Shared | `replace` | SAMHSA recovery facility search, Cache Storage shard state machine, and browser finder island; in scope per plan Section 3. |
| 38 | `crates/spruce` | `spruce` | Shared | `replace` | Product-neutral Spruce messaging channel adapter; in scope under D1. |
| 39 | `crates/telemetry` | `telemetry` | Shared | `replace` | OpenTelemetry tracing, metrics, log formatting, and HIPAA content redaction; in scope under D1. |
| 40 | `crates/ui` | `ui` | Shared | `replace` | Shared design tokens, components, 9-language i18n, motion, accessibility contracts, browser scripts; replaced by unified Bevy UI catalog. |
| 41 | `crates/ui-gallery` | `ui-gallery` | Tooling | `replace` | Interactive component showcase and visual proof target; replaced by Bevy component gallery. |
| 42 | `crates/workspace-tools` | `workspace-tools` | Tooling | `retain-as-tooling` | Workspace automation CLI (`install-doppler`, `staging-release`, `mirrors`, `recovery-import`); retained as dev/release tooling. |
| 43 | `crates/workspace-tools/sandbox` | `mole-sandbox` | Tooling | `retain-as-tooling` | Worktree seed manager and NixOS/Firecracker agent sandbox CLI; retained as dev/isolation tooling. |

## 2. Native `platform/` Bridges

| Path | Platform | Class | Justification |
| --- | --- | --- | --- |
| `platform/android/HealthConnectBridge.kt` | Android | `replace` | Native Health Connect query bridge; replaced by Bevy/Android platform adapter. |
| `platform/android/NotificationBridge.kt` | Android | `replace` | Native Android push notification bridge; replaced by Bevy/Android platform adapter. |
| `platform/android/SecureRetentionBridge.kt` | Android | `replace` | Native Android keystore secure retention bridge; replaced by Bevy/Android platform adapter. |
| `platform/android/DoctorPdfProvider.kt` | Android | `replace` | Native Android PDF FileProvider for `For my doctor` exports; replaced by Bevy/Android platform adapter. |
| `platform/android/HardwareKeyDispatch.kt` | Android | `replace` | Dioxus WebView hardware key dispatch; replaced by Bevy Android native input/IME adapter. |
| `platform/android/ModeShellTest.kt` | Android | `retain-as-tooling` | UI Automator emulator harness; kept as verifier/observer tooling with behavioral assertions restated for Bevy. |
| `platform/android/maven-central-mirror.gradle` | Android | `retain-as-tooling` | Gradle Maven Central mirror init script; retained as CI build tooling. |
| `platform/ios/HealthKitBridge.swift` | iOS | `replace` | Native iOS HealthKit query bridge; replaced by Bevy/iOS platform adapter. |
| `platform/ios/NotificationBridge.swift` | iOS | `replace` | Native iOS APNs registration/notification bridge; replaced by Bevy/iOS platform adapter. |
| `platform/ios/SecureRetentionBridge.swift` | iOS | `replace` | Native iOS Keychain secure retention bridge; replaced by Bevy/iOS platform adapter. |
| `platform/ios/CurieMobile.entitlements` | iOS | `reuse-as-approved-asset` | Declarative Apple HealthKit & background-delivery entitlements XML; reused directly. |
| `platform/ios/build_bridge.rs` | iOS | `retain-as-tooling` | Cargo build helper compiling Swift 5 static libraries via `xcrun swiftc`; retained as build tooling. |

## 3. Chrome Extension Components (`extension/`)

| Component | Paths | Class | Justification |
| --- | --- | --- | --- |
| MV3 Popup UI | `crates/extension-popup`, `extension/chrome/options.html`, `extension/chrome/popup.js` | `replace` | Candidate for Bevy Wasm popup under strict MV3 CSP (`'wasm-unsafe-eval'`), gated by M1 cold-start/size qualification (narrow HTML fallback per D2). |
| MV3 Background Service Worker & Connector | `extension/chrome/worker.js`, `extension/chrome/connector.js` | `replace` | Reimplemented as ordinary non-Bevy extension JS/TS (memory-only `chrome.storage.session` tokens, job envelope validation, durable SHA-256 outbox). |
| EHR Content / Page-Context Readers | `extension/chrome/readers/index.js`, `hint.js`, `hint-page.js` | `replace` | Reimplemented as ordinary non-Bevy extension scripts preserving declared named reads, response validation, and billing/Rx price redaction. |
| MV3 Manifest | `extension/chrome/manifest.json` | `replace` | Re-authored MV3 manifest preserving `optional_host_permissions` only, strict CSP, and pinned identity. |
| Extension Icons | `extension/chrome/icons/dabney-{16,32,48,128}.png` | `reuse-as-approved-asset` | Static PNG brand icons reused directly. |
| Local Pairing Sandbox & Offline Suite | `extension/sandbox/{bridge.py,run_agent.py,test_connector.mjs,extension-identity.json}` | `retain-as-tooling` | Local HTTP/MCP bridge and 11-case offline simulated-Chrome connector suite retained for black-box verification. |

## 4. Public Route Families

| Route Family | Routes | Reference Owner | Class | Justification |
| --- | --- | --- | --- | --- |
| Marketing & Product Overview | `/`, `/mobile` | `crates/public-site` | `replace` | Replaced as semantic, indexable HTML with clean-room interactive physician demo. |
| Versioned Legal & Transparency Docs | `/privacy`, `/terms`, `/clinical-transparency`, `/mobile/privacy`, `/mobile/privacy/:slug`, `/mobile/terms`, `/mobile/terms/:slug`, `/mobile/consumer-health-data` | `crates/public-site` | `replace` | Replaced as semantic, indexable HTML rendering verbatim approved legal JSON documents. |
| Connected-Care Invitation Landing | `/invite` | `crates/public-site` | `replace` | Replaced as `noindex` semantic HTML deep-link handoff & App Store / Play Store fallback. |
| Recovery Facility Directory | `/recovery`, `/recovery/:state`, `dei.dabney.moe/{, <ST>/, data/<ST>.json, data/index.json, sw.js}` | `crates/public-site`, `crates/dei-site`, `crates/recovery-directory` | `replace` | Replaced as zero-JS semantic prerendered HTML + clean-room search island consuming reused SAMHSA shards. |
| Crawler, Health & Fallback Endpoints | `/robots.txt`, `/sitemap.xml`, `/health`, `/readyz`, `/404` | `crates/public-site` | `replace` | Standard operational and SEO endpoints reimplemented in replacement public site. |
| Staging Observability Proxy Mount | `staging.curie.dabney.moe/grafana/*` | `crates/public-site`, `crates/observability-relay` | `retain-as-tooling` | IAP-gated staging-only operator proxy (never exposed in production); retained as ops tooling. |
| Pages-Only Personal Clarification | `curie.dabney.moe/personal-clarification/` | `crates/public-site/pages-only/personal-clarification/index.html` | `reuse-as-approved-asset` | Owner-authored static HTML personal statement published on its designated domain only. |
| Standalone Dux Ancestry & Genetics Web Page | `dabney.moe/dux/#data..#painting` (`/dux` → `/dux/`) | `crates/ancestry-app` (mounted by `DabneyMoeWebsite` `1946246`) | `out-of-scope` | Separate population-genetics client utility; continues independent static deployment at `/dux/` alongside `/elysianfields/`. |
| Elysian Fields Web Subpath *(Target)* | `dabney.moe/elysianfields/*` | Elysian Realm clients & APIs | `replace` | Target subpath for all replacement Bevy browser clients and backend APIs (must not assume domain root `/`). |

## 5. Background Services, Workers & Jobs

| Service / Binary | Reference Crate | Role | Class | Justification |
| --- | --- | --- | --- | --- |
| `physician-sample` | `crates/physician-sample` | Curie Clinic HTTP/SSE API server | `replace` | Core physician runtime; replaced as headless Rust service. |
| `physician-worker` | `crates/physician-sample` | Async leased chat-turn & reply-revision worker (`LISTEN` + 10s sweep + harness) | `replace` | Core durable execution worker; replaced as headless Rust worker. |
| `physician-rounds` | `crates/physician-sample` | Background patient-memory rounds upkeep job | `replace` | Core clinical memory upkeep job; replaced under D1. |
| `physician-migrate` (+ `physician-auth-migrate`, `front-door-migrate`, `physician-runtime-migrate`) | `crates/physician-sample`, `physician-auth`, `front-door`, `local-product` | Composite PostgreSQL migrator & synthetic chart loader | `replace` | Owns physician/auth/front-door schema evolution; replaced with clean-room persistence. |
| `mobile-api` | `crates/mobile-api` | Curie Mobile HTTP/SSE backend service | `replace` | Core patient runtime; replaced as headless Rust service. |
| `mobile-migrate` | `crates/mobile-api` | Curie Mobile PostgreSQL migrator | `replace` | Owns patient DB schema evolution; replaced with clean-room persistence. |
| `mobile-app-privacy-delivery` | `crates/mobile-api` | Cloud Run job delivering privacy/deletion receipts | `replace` | Core patient privacy receipt delivery; replaced under D1. |
| `mobile-privacy-replay` | `crates/mobile-api` | Independent privacy receipt journal restore-replay job | `replace` | Enforces deletion durability across DB restores; replaced under D1. |
| `mobile-map-front-door` | `crates/mobile-api` | Front-door connected-care mapping helper | `replace` | Part of connected-care bridge lifecycle; replaced under D1. |
| `clinical-library-server` (+ `clinical-library-migrate`, `clinical-library-pubmed-status`) | `crates/clinical-library` | Nightly clinical literature ingestion, PubMed status & query service | `replace` | Shared clinical reference pipeline; replaced under D1. |
| `mail-relay` | `crates/gmail` | SMTP AUTH / Gmail API outbound delivery service | `replace` | Delivers physician sign-in links & clinic mail; replaced under D1. |
| `encoder` | `crates/encoder` | Offline local BGE semantic chart retrieval encoder | `replace` | Shared local embedding service; replaced under D1. |
| `seed_demo_db` | `crates/physician-demo-db` | Build-time synthetic demo SQLite seeder | `replace` | Part of embedded interactive physician demo; replaced under D1. |
| `availability-probe` | `crates/availability-probe` | Secret-free Cloud Run uptime/readiness probe | `retain-as-tooling` | Ops monitoring binary with zero product logic. |
| `observability-relay` | `crates/observability-relay` | Secret-free OTLP/Grafana IAP relay | `retain-as-tooling` | Ops telemetry relay with zero product logic. |
| `mole-staging-events` | `crates/physician-sample` | Staging rehearsal synthetic event injector | `retain-as-tooling` | Operator staging rehearsal tool. |
| `curie-login` | `crates/local-product` | Local developer subscription login helper | `retain-as-tooling` | Local dev synthetic-only model login helper. |
