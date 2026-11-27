# M4 Vicissitude: Data Migration Rehearsal, Observability, Packaging & Rollout Proposal

This document defines the operational rehearsal architecture, verified invariants, production observability specification, multi-platform packaging and signing checklist, and staged cutover/rollback proposal for Elysian Fields.

All automated migration rehearsals live in [`crates/ef-migration-rehearsal`](../../crates/ef-migration-rehearsal/Cargo.toml) and run strictly on 100% synthetic data from [`crates/ef-patient-bank`](../../crates/ef-patient-bank/Cargo.toml) (zero protected health information and zero real patient or clinician records).

## 1. Rehearsal Methodology and Verified Operational Invariants

Every operational cutover requirement is codified as an automated, repeatable Rust test in [`crates/ef-migration-rehearsal/tests/rehearsal_suite.rs`](../../crates/ef-migration-rehearsal/tests/rehearsal_suite.rs):

| Operational Invariant | Implementation Owner | Rehearsed Verification Scope |
| --- | --- | --- |
| **Logical export & import at full cohort scale** | `ef_migration_rehearsal::export_import` | Exports the complete deterministic **468-patient** synthetic FHIR R4 cohort (`60` Light, `122` Moderate, `286` Complex, `5` flagship charts, `47` Connected Care links) with per-patient and manifest-wide SHA-256 checksums, imports across `WorkspaceEvidenceStore` (`ef-clinic-domain`), `PatientRecordStore` (`ef-mobile-domain`), and `ConnectedCareBridge` (`ef-connected-care`), and asserts `468/468` patients, `0` lost rows, `468/468` preserved `Allowed` panel assignments, `47/47` verified Connected Care links passing all 11 bridge invariants, and exact manifest SHA-256 parity. |
| **Mid-flight interruption & idempotent resume** | `run_fenced_logical_import`, `ImportCheckpoint` | Injects a hard crash after committing `200/468` patients, resumes from the durable checkpoint to complete the remaining `268` patients while skipping the `200` already-committed keys, then re-executes the full bundle idempotently (`200 + 468 = 668` duplicates skipped, `0` duplicate rows inserted, identical final SHA-256). |
| **Account deletion & privacy receipt replay** | `ef_migration_rehearsal::privacy_replay` | Snapshots pre-deletion primary Mobile & Connected Care stores, writes immutable create-only receipts (`SessionsRevoke`, `PublicationRevoke`, `AccountDelete`) into `PrivacyReceiptJournal` (verifying duplicate `put_if_absent` returns `AlreadyPresent`), restores the pre-deletion backup, executes `run_restore_replay_gate` + `ConnectedCareBridge::terminate_account_links`, proves `0` resurrected records, session rejection, Connected Care link termination, idempotent second replay, and fail-closed rejection when the independent receipt journal is unavailable. |
| **Single-writer domain fencing** | `DomainWriterFence`, `WriterLeaseToken` | Maintains independent monotonic epoch leases per storage domain (`curie_clinic`, `curie_mobile`, `connected_care`). Acquiring cutover ownership increments the domain epoch and rejects any concurrent or stale split-brain write with `RehearsalError::SplitBrainWriteRejected` without disrupting independent domains. |
| **Zero-side-effect shadow mode** | `OutboundEffectSink`, `ExecutionMode::ShadowZeroOutbound`, `ShadowDiffAudit` | Intercepts every outbound effect (`EhrWrite`, `SmsDispatch`, `EmailDispatch`, `ApnsPush`, `FcmPush`) during shadow runs, asserting **`0` outbound dispatches** while logging content-free suppressed audit entries and computing status + SHA-256 differential parity between reference and replacement responses. |
| **Pending-job drain & clean rollback** | `rehearse_pending_job_drain_and_handoff`, `CutoverRollbackSnapshot`, `execute_clean_rollback` | Rehearses `60 s` `ChatEngine` lease handoff with zero split-brain execution, `MessageReplyEngine` `Running -> Woken` recovery across worker restart, idempotent `HealthTransactionalOutbox` deduplication on post-cutover retry, and atomic rollback revoking the aborted candidate's writer lease and verifying exact pre-cutover SHA-256 state parity. |

## 2. Production Observability Specification

### 2.1 Content-Free Telemetry Invariant

All production logs, metrics, traces, dashboards, and alert payloads MUST remain strictly content-free:
- **Permitted fields**: service/crate name, route template, HTTP status code, opaque entity IDs (`practice_id`, `workspace_id`, `work_item_id`, `connection_id`, `account_incarnation`), latency histograms, retry/attempt counters, lease/epoch counters, and SHA-256 digests.
- **Forbidden fields**: clinical prose, prompts, assistant answers, cited quotes, patient names, dates of birth, email addresses, bearer session tokens, one-time sign-in codes, and raw EHR payloads.

### 2.2 Dashboards, SLOs, and Concrete Alert Rules

| Surface | Primary Dashboard Panels | Target SLO | Alert Rule & Threshold | Severity / Action |
| --- | --- | --- | --- | --- |
| **Curie Clinic** (`ef-clinic-server`, `ef-clinic-workflows`, `ef-clinic-client`) | Auth verification & refusal rates (`no_session`/`session_ended`/`access_removed`), SSE active streams & `resync` rate, worker lease stall reclaims, 5-failure `CouldntFinish` exhaustion rate, citation `[1]`/`[12]` open p95/p99, Today `failed_parts` rate | Auth & workspace API availability `>= 99.9%`; chat command ACK p95 `<= 300 ms`; citation open p95 `<= 100 ms` (note) / `<= 150 ms` (5k table); idle client `0 fps` | `clinic_api_5xx_ratio > 1%` for `5 m`; `clinic_chat_couldnt_finish_ratio > 2%` for `10 m`; `clinic_worker_stall_reclaims > 3` in `15 m`; `clinic_today_failed_parts_ratio > 5%` for `10 m`; `clinic_cross_practice_access_anomaly > 0` (immediate) | **Sev-1 / Page** on cross-practice anomaly or `5xx > 1%`; **Sev-2** on worker stalls or `CouldntFinish` spike -> pause cohort ramp |
| **Curie Mobile** (`ef-mobile-server`, `ef-mobile-domain`, `ef-mobile-client`) | Offline owed sign-out flush lag, portal PKCE & per-record savepoint error rate, HealthKit/Health Connect outbox upload lag, `PrivacyReceiptJournal` write & restore-replay status, restriction delivery job fail-closed count | API availability `>= 99.9%`; health outbox p95 drain `<= 60 s`; `PrivacyReceiptJournal` write durability `100%` (fail-closed) | `mobile_privacy_journal_put_errors > 0` in `1 m`; `mobile_restore_replay_gate_failures > 0`; `mobile_owed_signout_flush_lag_p95 > 300 s`; `mobile_portal_savepoint_failure_ratio > 10%` for `15 m` | **Sev-1 / Page** on any privacy journal or restore-replay gate failure (blocks backup promotion); **Sev-2** on vendor failure -> toggle per-vendor kill-switch |
| **Connected Care** (`ef-connected-care`) | Active connections, invitation creation/resend/cancel rates, 5-wrong-DOB permanent lockout counts, Part 2 written consent rejections, `Uncertain` transfer age, disconnect-halted transfers | Transfer state consistency `100%` (zero invariant violations); Part 2 unconsented transfer rate `0` | `connected_care_invariant_violation > 0`; `connected_care_dob_lockouts > 20` in `10 m` (brute-force signal); `connected_care_uncertain_transfer_age_max > 300 s` | **Sev-1 / Page** on any invariant violation; **Sev-2** on DOB lockout spike or aged uncertain transfers |
| **Clinical Library Nightly** (`ef-clinical-lib`, `ef-encoder`) | Nightly run completion status, PubMed shared `100 ms` rate-limiter wait histogram, 9-rule failure taxonomy breakdown, corpus freshness age | Nightly run completes within `2 h` window; PubMed rate `<= 10 req/s` strictly enforced | `clinical_lib_nightly_failed > 0`; `clinical_lib_corpus_age_hours > 36`; `clinical_lib_pubmed_rate_limit_429 > 5` in `15 m` | **Sev-2** alert to platform owner; preserves last known good corpus snapshot automatically |
| **Chrome Extension MV3** (`ef-extension`) | Popup Brotli-11 size CI gate (`<= 1.0 MiB`), strict CSP violation counter, named EHR extractor success/redaction counts | `0` CSP violations; popup cold open `<= 250 ms`; bundle size `<= 1.0 MiB` | `extension_csp_violations > 0`; `extension_bundle_brotli_bytes > 1048576` (CI hard fail) | **Sev-1** CI block on size/CSP regression; **Sev-2** on extractor breakage |
| **Public Site & Dei Recovery Finder** (`ef-public-site`) | `/elysianfields/` `308`/`404` & `no-store` synthetic probe, Dei SAMHSA shard cache load state (`fresh`/`stale`/`failed`), crisis-line (`988`/`911`/SAMHSA) synthetic render check | Static availability `>= 99.95%`; crisis-line synthetic check `100%` pass | `dei_crisis_banner_probe_failed > 0`; `subpath_cache_header_drift > 0`; `dei_shard_fetch_failure_ratio > 2%` for `10 m` | **Sev-1 / Page** on crisis-line probe failure or `no-store` header drift |

## 3. Multi-Platform Client Packaging and Signing Checklist

Status reflects current verified artifacts on Linux `x86_64` vs targets requiring physical hardware, OS code-signing certificates, or app-store credentials (marked **`blocked-no-hardware`** / **`credential-gated`**, never falsely reported as signed):

| # | Ledger Platform Key | Packaging Artifact | Signing & Verification Requirements | Current Readiness Status |
| --- | --- | --- | --- | --- |
| 1 | `browser-wasm-desktop` | Static `/elysianfields/` bundle (`index.html`, hashed `.wasm` + JS glue + SIL OFL fonts) | Subresource integrity / content-hashed immutable assets, strict CSP (`script-src 'self' 'wasm-unsafe-eval'`), `wasm-opt -O2 --strip-debug`, Brotli-11 `<= 3.5 MiB` (Clinic) / `<= 2.5 MiB` (Mobile Web) | **Build & Chromium headless CSP verified**; Safari/Firefox real-hardware check `blocked-no-hardware` |
| 2 | `browser-wasm-mobile` | Same responsive `/elysianfields/` static Wasm bundle | Mobile Safari >= 17.4 & Android Chrome >= 120 WebGL2 context-loss & soft-keyboard resize qualification | `blocked-no-hardware` (requires physical iOS & Android devices) |
| 3 | `desktop-linux` | Nix flake output / AppImage / `.deb` (`x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`) | Reproducible Nix build, SHA-256 release checksums, detached GPG/Cosign signature | **Native Linux `x86_64` build & tests verified**; release signing key `credential-gated`, low-tier Wayland/Vulkan GPU check `blocked-no-hardware` |
| 4 | `desktop-macos` | Signed & notarized universal/ARM64 `.app` / `.dmg` (`aarch64-apple-darwin`) | Apple Developer ID Application certificate (`codesign --options runtime`), `notarytool` notarization, stapled ticket | `blocked-no-hardware` + `credential-gated` |
| 5 | `desktop-windows` | Signed MSIX / `.msi` installer (`x86_64-pc-windows-msvc`) | Authenticode EV / Azure Trusted Signing certificate + timestamp server | `blocked-no-hardware` + `credential-gated` |
| 6 | `android-native` | Play Store signed `.aab` (`aarch64-linux-android`, API 34–35) | Play App Signing upload keystore (Doppler/HSM), Health Connect manifest declarations, TalkBack verification | `blocked-no-hardware` + `credential-gated` |
| 7 | `ios-native` | App Store / TestFlight signed `.ipa` (`aarch64-apple-ios`, iOS 15.0+) | Apple Distribution certificate, provisioning profile, HealthKit & Push entitlements, VoiceOver verification | `blocked-no-hardware` + `credential-gated` |
| 8 | `chrome-extension-popup` | Signed MV3 `.zip` / `.crx` package (`extension/`, Decision D2 semantic HTML popup) | Strict MV3 CSP (`0` inline scripts, `0` `unsafe-eval`), Brotli-11 `<= 1.0 MiB` hard cap (`~1.5 KiB` measured), Chrome Web Store review | **Artifact, CSP, and size cap verified**; Web Store publish `credential-gated`, headed toolbar check `blocked-no-hardware` |
| 9 | `hosted-services` | Non-root distroless Linux container images (`ef-clinic-server`, `ef-mobile-server`, workers) | Multi-arch OCI digest pinning, Cosign image attestation, Doppler runtime secret injection (`DOPPLER_TOKEN` only in Secret Manager) | **Headless Rust builds, SQL migrations & WAL durability verified**; production registry/cluster deploy `credential-gated` |
| 10 | `static-public-web` | Static semantic HTML/CSS + Dei SAMHSA shards (`ef-public-site`) | Indexable zero-JS HTML check, verbatim legal archive checksums, CDN cache header audit | **Static HTML generation, crisis-first Dei finder, and shard state machine verified**; production CDN deploy `credential-gated` |

## 4. Staged Rollout Proposal and Rollback Runbook (For Owner Approval)

External production cutover, DNS/routing changes, and reference retirement require explicit owner authorization before each stage transition.

### 4.1 Environment and Cohort Progression

| Stage | Target Scope | Writer Ownership | Outbound Side Effects | Entry & Exit Gate |
| --- | --- | --- | --- | --- |
| **Stage 0: Synthetic Rehearsal** | `ef-migration-rehearsal` + staging DB/bus | Rehearsal-only isolated cluster | Disabled (`ShadowZeroOutbound`) | **COMPLETE** (468-patient crash/resume parity, privacy replay, single-writer fencing, job drain, and rollback all green). |
| **Stage 1: Staging Shadow** | 100% mirrored read traffic under `/elysianfields/` in staging | Reference holds sole writer lease | **Strictly disabled (`0` outbound dispatches)** | `>= 72 h` continuous shadow with `0` outbound side effects, `0` cross-practice/Part 2 diff mismatches, p95 latency within M1 budgets, and all observability alerts wired & verified. |
| **Stage 2: Internal Pilot / Canary (5%)** | Explicit opt-in internal practices & test mobile accounts | Exclusive writer lease transferred per migrated practice/account (single writer per domain) | Enabled strictly for opted-in canary cohort | Owner sign-off + pre-cutover `CutoverRollbackSnapshot` verified + `>= 7 days` zero Sev-1 incidents, `0` privacy journal errors, `0` data parity regressions, and physical-hardware sign-off on required desktop/mobile targets. |
| **Stage 3: Controlled Expansion (25% -> 50%)** | Opt-in clinician practices + Connected Care cohorts | Exclusive Elysian Fields writer lease per migrated domain | Enabled for migrated cohorts | `>= 14 days` meeting all Section 2.2 SLOs, zero uninvestigated `CouldntFinish` spikes, verified backup-restore replay drill in staging. |
| **Stage 4: General Availability (100%) & Reference Retirement (M5)** | All authorized practices, Curie Mobile accounts, Extension, and Public/Dei surfaces | Exclusive Elysian Fields writer leases across all domains | Full production | Every scope item reaches verified replacement disposition; transitional shadow adapters removed; reference read/write paths decommissioned only after retention and recovery duties are confirmed. |

### 4.2 Automatic Abort Triggers

Cutover halts and triggers immediate rollback if **any** of the following conditions occurs:
1. Any cross-practice, cross-patient, or unconsented 42 CFR Part 2 data disclosure (`count > 0`).
2. Any `PrivacyReceiptJournal` write failure, restore-replay gate failure, or deleted-account resurrection (`count > 0`).
3. Any split-brain writer fence violation (`SplitBrainWriteRejected`) indicating concurrent writes across legacy and replacement systems.
4. Any shadow-mode outbound side effect (`dispatched_count > 0` while `ShadowZeroOutbound` is active).
5. Sustained API `5xx > 1%` for `5 minutes`, `CouldntFinish > 2%` for `10 minutes`, or Dei crisis-line synthetic probe failure.

### 4.3 Zero-Data-Loss Rollback Runbook

1. **Freeze ingress & pause cohort progression**: Shift `/elysianfields/` canary routing back to the prior verified target and stop new command admissions on the aborted candidate (`503` maintenance fence during handoff, `Cache-Control: no-store`).
2. **Revoke candidate writer fence**: Execute `DomainWriterFence::acquire_exclusive(domain, prior_holder_id)` (advancing the monotonic domain epoch) so no in-flight candidate worker can commit post-abort writes.
3. **Drain pending jobs**: Allow active `60 s` worker leases to expire or reclaim cleanly; recover interrupted reply reviews to `Woken`; preserve client mobile health outbox items (idempotently deduplicated by `(account_id, idempotency_key)`).
4. **Restore verified snapshot & replay privacy receipts**: Restore the pre-cutover `CutoverRollbackSnapshot` (verifying exact SHA-256 parity) **and unconditionally run `run_restore_replay_gate` + `ConnectedCareBridge::terminate_account_links`** against the independent `PrivacyReceiptJournal` before serving any traffic so no account deletion or privacy restriction committed during the canary window can ever resurrect.
5. **Resume prior traffic & audit**: Confirm health checks, `no-store` headers, and zero active Sev-1 alerts before re-opening traffic, then file a root-cause report before scheduling a replacement rehearsal.
