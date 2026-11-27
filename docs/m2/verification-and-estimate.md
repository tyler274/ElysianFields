# M2 Deliverance Verification Report and M3 Infinity Sizing Estimate

This report records the independent verification of **Milestone M2 (Deliverance — One vertical slice)** at `main` revision **`4d69f5f`** (`4d69f5fcb7e925f1ad6ab364210d645caafe1ed2`), audits all seven M2 replacement crates against every requirement and executable scenario in [`specs/packages/example-session-shell/`](../../specs/packages/example-session-shell/spec.md) and [`specs/packages/m2-physician-vertical-slice/`](../../specs/packages/m2-physician-vertical-slice/spec.md), records remaining architectural and physical-hardware gaps honestly, and derives the effort and sizing estimate for **Milestone M3 (Infinity — Suite replacement, Increments 1–5)** directly from measured M2 delivery data.

---

## 1. Executive Verification Verdict

**Verdict: PASS WITH RECORDED INTEGRATION & HARDWARE GAPS** at revision **`4d69f5f`**.

- **M2 vertical-slice test suite**: **36 / 36 passed** (`0` failed, `0` ignored) across all seven M2 crates (`ef-core`, `ef-clinic-domain`, `ef-harness`, `ef-boundaries`, `ef-ui-catalog`, `ef-clinic-server`, `ef-clinic-client`) in `50 s` cold / `15 s` warm workspace run.
- **Full workspace verification (`./scripts/dev-check.sh --tier full`)**: **PASS in `1m22s`** (`cargo-fmt` `4 s`, `python-tests` `7 s` / 66 tests, `clippy-host` `-D warnings` `23 s`, `clippy-wasm` `wasm32-unknown-unknown` `-D warnings` `32 s`, `cargo-test` `15 s` / **87 / 87** Rust tests across all 14 workspace crates, `docs-and-wall` `1 s` with **127 / 127** files clean and `0` wall violations).
- **Supply-chain & license audit (`cargo deny --all-features check advisories bans licenses sources`)**: **PASS in `13 s`** (`advisories ok, bans ok, licenses ok, sources ok`).
- **Ledger schema check (`scripts/check_ledger.py`)**: **PASS** (`33 / 33` requirements valid; 13 M2/example requirements populated with `passed` on verified Linux/Wasm/hosted targets and `blocked` on unexercised physical hardware targets; 20 M3 requirements retained as `unknown`).
- **Honest gap accounting**: Automated domain, contract, real-TCP HTTP/SSE WAL-durability, headless Bevy `0.20.1`, and `wasm32-unknown-unknown` verification passed completely. Physical-hardware OS/mobile targets (`desktop-macos`, `desktop-windows`, `android-native`, `ios-native`, `browser-wasm-mobile`) remain explicitly **`blocked`** in [`specs/ledger.toml`](../../specs/ledger.toml). Section 4 records four concrete pre-production integration items identified during the independent code and contract audit (client/server route alignment, live PostgreSQL/Valkey multi-instance runtime wiring, per-IP `429` rate limiting, and `BootTab` outage screen handling).

---

## 2. Commands Executed & Observed Outputs (Revision `4d69f5f`)

| Gate | Exact Command | Observed Output | Duration | Status |
| --- | --- | --- | ---: | --- |
| M2 crates test suite | `cargo test --locked -p ef-core -p ef-clinic-domain -p ef-harness -p ef-boundaries -p ef-ui-catalog -p ef-clinic-server -p ef-clinic-client` | **36 passed, 0 failed, 0 ignored** (`ef-core` 5, `ef-clinic-domain` 4, `ef-harness` 3, `ef-boundaries` 3, `ef-ui-catalog` 7, `ef-clinic-server` 2, `ef-clinic-client` 12) | `50 s` (cold) | **PASS** |
| Dependency & license gate | `nix shell nixpkgs#cargo-deny -c cargo deny --all-features check advisories bans licenses sources` | `advisories ok, bans ok, licenses ok, sources ok` (non-blocking duplicate transitive warnings only) | `13 s` | **PASS** |
| Full local CI gate | `nix shell nixpkgs#python3 nixpkgs#mdbook -c ./scripts/dev-check.sh --tier full` | `cargo-fmt` `4 s`, `python-tests` `7 s` (66 tests), `clippy-host` `23 s`, `clippy-wasm` `32 s`, `cargo-test` `15 s` (**87 passed, 0 failed**), `docs-and-wall` `1 s` (127 files, `0` wall violations, `mdbook build` clean) | `1m22s` | **PASS** |
| Ledger & wall validators | `nix shell nixpkgs#python3 -c python3 scripts/check_ledger.py && nix shell nixpkgs#python3 -c python3 scripts/check_wall.py` | `Ledger validation OK: 33 requirement(s) verified in specs/ledger.toml.` and `Clean-room wall check OK: 127 file(s) scanned, 0 violations.` | `1 s` | **PASS** |

---

## 3. Requirement-to-Test Traceability Matrix (`4d69f5f`)

Every M2 and session-shell requirement (`EF-CLINIC-0001`–`0009`, `EF-HARNESS-0001`, `EF-PLATFORM-0001`–`0003`) maps to its executable scenario, verified Rust tests, command, observed result, and audited revision SHA:

| Req ID | Scenario ID | Owning Crates | Verifying Test(s) | Command & Observed Output | Revision | Target Disposition |
| --- | --- | --- | --- | --- | --- | --- |
| `EF-CLINIC-0001` | `SCN-CLINIC-0001` | `ef-core`, `ef-clinic-domain`, `ef-clinic-server`, `ef-clinic-client` | `scn_m2_0001_invite_only_sign_in_single_use_verify_and_anti_enumeration`, `scn_m2_0002_two_tab_sign_out_expiry_clearing_and_keeps_work_race`, `scn_m2_0003_deep_link_patient_landing_across_unauthenticated_boot`, `subpath_and_loopback_override_satisfy_m2_platform_rules`, `full_vertical_slice_e2e_over_real_tcp_with_restart_durability` | `cargo test --locked -p ef-core -p ef-clinic-domain -p ef-clinic-server -p ef-clinic-client` -> `23 passed, 0 failed` | `4d69f5f` | `browser-wasm-desktop`, `desktop-linux`: **passed**; `browser-wasm-mobile`, `desktop-macos`, `desktop-windows`: **blocked** |
| `EF-CLINIC-0002` | `SCN-M2-0001` | `ef-clinic-domain`, `ef-clinic-server`, `ef-clinic-client` | `scn_m2_0001_invite_only_anti_enumeration_single_use_verify_and_session_lifetime`, `scn_m2_0001_invite_only_sign_in_single_use_verify_and_anti_enumeration`, `full_vertical_slice_e2e_over_real_tcp_with_restart_durability`, `deactivated_account_and_sse_last_event_id_filtering_over_tcp` | `cargo test --locked -p ef-clinic-domain -p ef-clinic-server -p ef-clinic-client` -> `18 passed, 0 failed` | `4d69f5f` | `browser-wasm-desktop`, `desktop-linux`: **passed**; `desktop-macos`, `desktop-windows`, `android-native`, `ios-native`: **blocked** |
| `EF-CLINIC-0003` | `SCN-M2-0002` | `ef-clinic-domain`, `ef-clinic-client` | `scn_m2_0002_and_0003_all_11_sign_in_invariants_multi_tab_and_deep_link`, `scn_m2_0002_two_tab_sign_out_expiry_clearing_and_keeps_work_race` | `cargo test --locked -p ef-clinic-domain -p ef-clinic-client` -> `16 passed, 0 failed` | `4d69f5f` | `browser-wasm-desktop`: **passed** |
| `EF-CLINIC-0004` | `SCN-M2-0003` | `ef-clinic-domain`, `ef-clinic-client` | `scn_m2_0002_and_0003_all_11_sign_in_invariants_multi_tab_and_deep_link`, `scn_m2_0003_deep_link_patient_landing_across_unauthenticated_boot` | `cargo test --locked -p ef-clinic-domain -p ef-clinic-client` -> `16 passed, 0 failed` | `4d69f5f` | `browser-wasm-desktop`: **passed** |
| `EF-CLINIC-0005` | `SCN-M2-0004` | `ef-clinic-domain`, `ef-clinic-server`, `ef-clinic-client` | `scn_m2_0004_and_0005_three_tier_access_citations_monotonic_fencing_and_redaction`, `full_vertical_slice_e2e_over_real_tcp_with_restart_durability`, `scn_m2_0004_practice_scoped_three_tier_access_and_honest_roster_states` | `cargo test --locked -p ef-clinic-domain -p ef-clinic-server -p ef-clinic-client` -> `18 passed, 0 failed` | `4d69f5f` | `hosted-services`, `browser-wasm-desktop`: **passed** |
| `EF-CLINIC-0006` | `SCN-M2-0005` | `ef-clinic-domain`, `ef-ui-catalog`, `ef-clinic-server`, `ef-clinic-client` | `scn_m2_0004_and_0005_three_tier_access_citations_monotonic_fencing_and_redaction`, `virtualized_list_and_5000_row_table_stay_o_viewport`, `full_vertical_slice_e2e_over_real_tcp_with_restart_durability`, `scn_m2_0005_evidence_reading_5000_row_virtualized_citations_and_redaction` | `cargo test --locked -p ef-clinic-domain -p ef-ui-catalog -p ef-clinic-server -p ef-clinic-client` -> `25 passed, 0 failed` | `4d69f5f` | `browser-wasm-desktop`, `desktop-linux`: **passed**; `android-native`, `ios-native`: **blocked** |
| `EF-CLINIC-0007` | `SCN-M2-0006` | `ef-clinic-domain`, `ef-harness`, `ef-ui-catalog`, `ef-clinic-server`, `ef-clinic-client` | `scn_m2_0006_to_0009_chat_wire_contract_idempotency_stop_resume_5_failure_retry_and_draft_conflict`, `stub_provider_executes_two_tool_rounds_streams_answer_and_emits_content_free_telemetry`, `reduced_motion_and_idle_redraw_gate_render_zero_frames_when_idle`, `full_vertical_slice_e2e_over_real_tcp_with_restart_durability`, `scn_m2_0006_streamed_clinical_chat_tool_trace_idempotency_and_idle_zero_frames` | `cargo test --locked -p ef-clinic-domain -p ef-harness -p ef-ui-catalog -p ef-clinic-server -p ef-clinic-client` -> `28 passed, 0 failed` | `4d69f5f` | `browser-wasm-desktop`, `desktop-linux`: **passed**; `android-native`, `ios-native`: **blocked** |
| `EF-CLINIC-0008` | `SCN-M2-0007` | `ef-clinic-domain`, `ef-harness`, `ef-clinic-server`, `ef-clinic-client` | `scn_m2_0006_to_0009_chat_wire_contract_idempotency_stop_resume_5_failure_retry_and_draft_conflict`, `stub_provider_supports_mid_stream_cancel_and_5_failure_exhaustion_with_try_again`, `full_vertical_slice_e2e_over_real_tcp_with_restart_durability`, `scn_m2_0007_mid_stream_cancellation_stop_and_fenced_resume` | `cargo test --locked -p ef-clinic-domain -p ef-harness -p ef-clinic-server -p ef-clinic-client` -> `21 passed, 0 failed` | `4d69f5f` | `browser-wasm-desktop`, `hosted-services`: **passed** |
| `EF-CLINIC-0009` | `SCN-M2-0008` | `ef-clinic-domain`, `ef-harness`, `ef-clinic-server`, `ef-clinic-client` | `scn_m2_0006_to_0009_chat_wire_contract_idempotency_stop_resume_5_failure_retry_and_draft_conflict`, `stub_provider_supports_mid_stream_cancel_and_5_failure_exhaustion_with_try_again`, `full_vertical_slice_e2e_over_real_tcp_with_restart_durability`, `scn_m2_0008_bounded_5_failure_retry_exhaustion_and_try_again_recovery` | `cargo test --locked -p ef-clinic-domain -p ef-harness -p ef-clinic-server -p ef-clinic-client` -> `21 passed, 0 failed` | `4d69f5f` | `browser-wasm-desktop`, `hosted-services`: **passed** |
| `EF-HARNESS-0001` | `SCN-M2-0009` | `ef-clinic-domain`, `ef-harness`, `ef-clinic-server`, `ef-clinic-client` | `scn_m2_0006_to_0009_chat_wire_contract_idempotency_stop_resume_5_failure_retry_and_draft_conflict`, `clinical_tool_enforces_cross_patient_isolation_fail_closed`, `full_vertical_slice_e2e_over_real_tcp_with_restart_durability`, `deactivated_account_and_sse_last_event_id_filtering_over_tcp`, `scn_m2_0009_reload_bus_resync_draft_409_conflict_and_durable_readback` | `cargo test --locked -p ef-clinic-domain -p ef-harness -p ef-clinic-server -p ef-clinic-client` -> `21 passed, 0 failed` | `4d69f5f` | `browser-wasm-desktop`, `hosted-services`: **passed** |
| `EF-PLATFORM-0001` | `SCN-M2-0010` | `ef-core`, `ef-boundaries`, `ef-clinic-server`, `ef-clinic-client` | `subpath_and_loopback_override_satisfy_m2_platform_rules`, `workspace_manifests_satisfy_all_architecture_boundaries`, `full_vertical_slice_e2e_over_real_tcp_with_restart_durability`, `scn_m2_0010_subpath_mounting_loopback_safety_cache_headers_and_strict_csp` | `cargo test --locked -p ef-core -p ef-boundaries -p ef-clinic-server -p ef-clinic-client` -> `22 passed, 0 failed` | `4d69f5f` | `browser-wasm-desktop`, `hosted-services`: **passed** |
| `EF-PLATFORM-0002` | `SCN-M2-0011` | `ef-ui-catalog`, `ef-clinic-client` | `shortcut_router_enforces_global_privacy_cover_scopes_and_escape_restoration`, `headless_bevy_plugin_projects_single_owner_a11y_bridge_and_throttles_live_stream`, `wcag_21_aa_themes_pass_light_and_dark_and_reject_bad_contrast`, `scn_m2_0011_keyboard_only_navigation_privacy_cover_and_zero_duplicate_a11y` | `cargo test --locked -p ef-ui-catalog -p ef-clinic-client` -> `19 passed, 0 failed` | `4d69f5f` | `browser-wasm-desktop`, `desktop-linux`: **passed**; `desktop-macos`, `desktop-windows`: **blocked** |
| `EF-PLATFORM-0003` | `SCN-M2-0012` | `ef-core`, `ef-ui-catalog`, `ef-clinic-client` | `nine_locales_and_startup_priority_match_spec`, `nine_locale_catalog_is_exhaustive_and_resolves_priority_with_rtl_mirroring`, `composer_supports_multiline_ime_rtl_indic_clusters_undo_redo_and_clipboard`, `scn_m2_0012_nine_locale_switching_arabic_rtl_mirroring_and_verbatim_source_language` | `cargo test --locked -p ef-core -p ef-ui-catalog -p ef-clinic-client` -> `24 passed, 0 failed` | `4d69f5f` | `browser-wasm-desktop`, `desktop-linux`: **passed**; `android-native`, `ios-native`: **blocked** |

---

## 4. Independent Code & Contract Audit Findings

### 4.1 Verified Invariants (PASS)

1. **All 11 formal sign-in invariants (`EF-CLINIC-0003`, `EF-CLINIC-0004`)**:
   - Audited directly in [`crates/ef-clinic-domain/src/auth.rs`](../../crates/ef-clinic-domain/src/auth.rs) (`MultiTabBrowser`, `TabViewState`, `check_safety_invariants`) and [`crates/ef-clinic-client/src/state.rs`](../../crates/ef-clinic-client/src/state.rs) (`SharedBrowserSessionHub`).
   - Structural type guarantees enforce `PromptIsEmpty` (`TabViewState::SessionEndedPrompt` holds zero fields) and `SignedIn` (`TabViewState::Workspace` requires a concrete `SessionId`).
   - Behavioral checks verify `OnceOut`, `Expiry`, `KeepsWork` (stale `401` adopts newer live session in shared storage instead of ejecting), `TodayFirst`, `Lands` (deep-linked `?patient=` opens after sibling-tab sign-in), `ActsAsShown`, `ServerRefusesSignedOut` (revoking both shared storage and held token including fallback sign-out), `CatchesUp`, and `NoDeadlock`.
   - Session lifetime enforces `12 h` idle timeout, `7 d` absolute cap, `15 m` sliding renewal quantum, fixed-end clamping, and strict `now < idle_deadline` inequality.
2. **Practice-scoped three-tier access & cross-patient isolation (`EF-CLINIC-0005`, `EF-CLINIC-0006`)**:
   - `MembershipTier::{Allowed, ClinicDemographic, Denied}` is owned in [`crates/ef-clinic-domain/src/workspace.rs`](../../crates/ef-clinic-domain/src/workspace.rs) and enforced uniformly across `ef-clinic-server`, `ef-harness` (`SearchChartTool`, `ReadRecordsTool`), and `ef-clinic-client`.
   - `Denied` (cross-practice or unknown workspace) returns `404 Not Found` with `Cache-Control: private, no-store, max-age=0`, `CDN-Cache-Control: no-store`, and `Surrogate-Control: no-store`.
   - `ClinicDemographic` returns demographic summary only (`chats: None`) and refuses clinical citation bodies (`403`).
   - Cross-patient `source_key` lookups return `404 Not Found`. Withdrawn citations return `404` (`status: "unavailable"`) with zero clinical text and clear `CitationPanelController` state immediately.
3. **Real TCP/SSE server restart durability & `/elysianfields/` subpath security (`EF-HARNESS-0001`, `EF-PLATFORM-0001`)**:
   - [`crates/ef-clinic-server/src/persistence.rs`](../../crates/ef-clinic-server/src/persistence.rs) appends typed `WalEntry` JSONL frames with `flush()` + `sync_all()` (`fsync`) before returning HTTP `200`, and replays them on restart through the exact same live domain owners (`AuthAuthority`, `WorkspaceEvidenceStore`, `ChatEngine`, `DraftStore`) so live and recovery paths cannot drift apart.
   - Real TCP tests ([`crates/ef-clinic-server/tests/vertical_slice_e2e.rs`](../../crates/ef-clinic-server/tests/vertical_slice_e2e.rs)) verify shutdown + cold restart preserving sessions, 3 chat exchanges (`finished`, `stopped` with kept partial text, and 5-failure -> `Try again` -> `finished`), idempotency receipts, and revision-2 drafts; plus `Last-Event-ID` SSE cursor filtering and `?resync=true`.
   - Subpath enforcement over real TCP verifies `/` -> `404` (`no-store`), `/elysianfields-evil/` -> `404`, `/elysianfields?patient=ws-a1` -> `308 Permanent Redirect` to `/elysianfields/?patient=ws-a1`, `/elysianfields/` -> `200` with `Cache-Control: no-cache`, zero `Last-Modified`, incoming `If-Modified-Since` stripped, strict Wasm CSP (`script-src 'self' 'wasm-unsafe-eval'; frame-ancestors 'none'`), `X-Frame-Options: DENY`, `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, and `413 Payload Too Large` over `64 KiB` on `/auth/*` (`2 MiB` on `/commands`).
4. **Architecture boundaries, zero secrets & clean-room wall (`EF-PLATFORM-0001`, `EF-PLATFORM-0002`, `EF-PLATFORM-0003`)**:
   - [`crates/ef-boundaries/src/lib.rs`](../../crates/ef-boundaries/src/lib.rs) enforces zero `bevy`/`bevy_*` dependencies in all headless crates (`ef-core`, `ef-clinic-domain`, `ef-harness`, `ef-boundaries`, `ef-clinic-server`), zero cross-product imports between `ef-clinic-*` and `ef-mobile-*`, and zero forbidden secret patterns across all client crates and `web/`.
   - [`crates/ef-harness/src/lib.rs`](../../crates/ef-harness/src/lib.rs) emits strictly allowlisted content-free `TelemetrySpan` metadata verified to contain zero prompt or answer prose.
   - [`crates/ef-ui-catalog`](../../crates/ef-ui-catalog/src/lib.rs) and [`crates/ef-clinic-client`](../../crates/ef-clinic-client/src/plugin.rs) enforce compile-time exhaustive 9-locale translations (`56` keys x `9` locales = `504` entries), Arabic RTL mirroring, single-owner `A11yProjection` (`duplicated_nodes == 0`, `mirrored_css_rules == 0`), throttled polite live-region queue, global `Cmd/Ctrl+Shift+L` privacy cover with exact focus restoration, `O(viewport)` virtualization for 5,000-row lab tables, and `0` redraws during idle frames.
   - `scripts/check_wall.py` scans all **127** files in `specs/`, `crates/`, and `book/` with **`0` violations**.

### 4.2 Honest Gaps & Pre-Production Integration Items Identified

The audit identified four concrete gaps between the M2 vertical-slice deliverables and full hosted production parity. None invalidate M2 domain or contract verification, and all are scoped into M3 Increment 1 / M4 rehearsal:

1. **Client-to-server HTTP route & SSE framing alignment (`ef-clinic-client` <-> `ef-clinic-server`)**:
   - `ef-clinic-client` currently exercises its full Bevy plugin through `LocalVerticalSliceTransport` (in-process bridge over shared domain/harness owners) using spec paths (`/elysianfields/api/patient-workspaces`, `/commands`, `/patient-workspaces/events`, `/physician-state/drafts/...`) and SSE `event: message`, whereas `ef-clinic-server` currently serves `/elysianfields/api/patients`, `/workspaces/{ws}/chats/{chat}/commands`, `.../stream`, `.../draft`, returns `200 OK` JSON on `POST /auth/sign-out` (spec specifies `204 No Content`), and emits per-variant SSE event names (`delta_chunk`, `step_recorded`, `completed`).
   - **Action**: Unify route paths, `204` sign-out status, and SSE framing in a shared contract module and add the live HTTP/SSE `ClinicTransport` implementation against `ef-clinic-server` in M3 Increment 1.
2. **Hosted multi-instance runtime infrastructure (PostgreSQL + Valkey/Redis + `429` rate limiting)**:
   - `ef-clinic-server` proves crash durability via an fsynced JSONL WAL and ships the 11-table PostgreSQL 16/17 DDL migration ([`crates/ef-clinic-server/migrations/0001_m2_clinic_slice.sql`](../../crates/ef-clinic-server/migrations/0001_m2_clinic_slice.sql)), but runs single-process synchronous `TcpListener` with in-process worker execution rather than live PostgreSQL + Valkey/Redis pub/sub across 2 API instances + separate worker process, and does not yet enforce per-IP `429 Too Many Requests` + `Retry-After`.
   - **Action**: Wire PostgreSQL persistence, separate worker lease loop, lossy Valkey/Redis pub/sub fan-out, and ingress `429` rate limiting in M3 Increment 1 / M4 rehearsal.
3. **Minor error-handling edges in `ef-clinic-client` & `/auth/whoami` refusal status**:
   - In [`crates/ef-clinic-client/src/plugin.rs`](../../crates/ef-clinic-client/src/plugin.rs), initial workspace entry ignores a failed `fetch_patient_roster` (`if let Ok(...)`) and falls back to an empty transcript on failed `fetch_chat_snapshot` (`.unwrap_or_default()`), and `resolve_opening` transitions a network/service outage during `BootTab` to `SignInPage` rather than a dedicated retryable `ServiceUnavailable` screen (`EF-CLINIC-0001`).
   - In [`crates/ef-clinic-server/src/service.rs`](../../crates/ef-clinic-server/src/service.rs), `GET /auth/whoami` returns `403` for `access_removed` (matching `POST /auth/verify`) rather than `401` with `reason: "access_removed"` as listed in Chapter 1 §1.2.
   - **Action**: Tighten all three in M3 Increment 1 so every failed read surfaces an explicit error + **Try again** state.
4. **Physical hardware & interactive browser/AT sign-off (`blocked`)**:
   - Headless Linux verification cannot establish physical GPU frame pacing, OS IME candidate placement, or live screen-reader traversal on macOS (Metal/VoiceOver), Windows (DX12/NVDA), Android ARM64 (TalkBack), iOS ARM64 (VoiceOver), or mobile web browsers. Those targets remain explicitly **`blocked`** in [`specs/ledger.toml`](../../specs/ledger.toml) until executed on representative hardware per [`docs/m1/decision-review.md`](../m1/decision-review.md) Section 5.

---

## 5. Measured M2 Deliverance Effort Breakdown

All figures below are measured directly from committed files and GitHub Actions workflow runs at revision `4d69f5f`:

### 5.1 Crate, LOC & Test Inventory

| Crate | Kind | Source LOC (`src/*.rs`) | Integration Test LOC (`tests/*.rs`) | SQL DDL LOC | Total LOC | Passing Tests | Merged PR |
| --- | --- | ---: | ---: | ---: | ---: | ---: | --- |
| `ef-core` | Headless foundation | 844 | 151 | 0 | 995 | 5 | PR #15 |
| `ef-clinic-domain` | Headless domain | 2,693 | 784 | 0 | 3,477 | 4 | PR #15 |
| `ef-harness` | Headless execution | 608 | 330 | 0 | 938 | 3 | PR #15 |
| `ef-boundaries` | Boundary gate | 271 | 68 | 0 | 339 | 3 | PR #15 |
| `ef-ui-catalog` | Shared Bevy `0.20.1` UI | 4,295 | 0 *(unit tests in `src/lib.rs`)* | 0 | 4,295 | 7 | PR #14 |
| `ef-clinic-server` | Headless HTTP/SSE + WAL | 2,037 | 993 | 117 | 3,147 | 2 | PR #17 |
| `ef-clinic-client` | Bevy `0.20.1` clinic client | 3,523 | 1,744 | 0 | 5,267 | 12 | PR #18 |
| **M2 Total (7 crates)** | **4 headless + 2 Bevy + 1 gate** | **14,271** | **4,070** | **117** | **18,458** | **36** | **PRs #14, #15, #17, #18** |

- **Total M2 PR count**: **6 PRs** — 4 product crate PRs (#14, #15, #17, #18: `+18,704` lines across 56 changed files), 1 CI Wasm cache & per-deployable crate scoping PR (#16: `+253 / -54` lines across 3 files), and 1 verification & ledger closeout PR (#19).
- **M2 delivery unit ratios (calibration baseline for M3)**:
  - **Requirements delivered**: `13` (`EF-CLINIC-0001`–`0009`, `EF-HARNESS-0001`, `EF-PLATFORM-0001`–`0003`).
  - **Average code footprint per requirement**: `~1,420 LOC` total (`~1,098` src + `~322` test/DDL), or **`~1,090 LOC` per requirement** excluding one-time shared foundation (`ef-core`, `ef-boundaries`, `ef-ui-catalog`).
  - **Shared reusable foundation already sunk in M2**: `ef-core` (`995` LOC) + `ef-boundaries` (`339` LOC) + `ef-ui-catalog` (`4,295` LOC) = **`5,629` LOC (30.5% of M2)** that M3 increments reuse directly without rewriting.

### 5.2 Measured CI & Local Verification Durations

| Workflow / Gate | Measured Duration | Configured Budget (`docs/ci/budgets.json`) | Headroom |
| --- | ---: | ---: | ---: |
| Local `dev-check.sh --tier full` (warm, 14 crates, host + wasm32 Clippy + 87 Rust + 66 Python tests) | **`82 s` (`1m22s`)** | N/A (local parity gate) | Fast local loop |
| PR #15 CI (`ef-core`, `ef-clinic-domain`, `ef-harness`, `ef-boundaries`; Run `46021081266`) | **`115 s`** active (`rust-native` `84 s`, `rust-wasm` `46 s`, `cargo-deny` `57 s`, `lightweight` `11 s`) | `360 s` PR / `240 s` job | **3.1x faster** than PR cap |
| PR #17 CI (`ef-clinic-server`; Run `46024183410`) | **`234 s`** (`rust-native` `148 s`, `rust-wasm` `206 s`, `cargo-deny` `53 s`, `lightweight` `27 s`) | `360 s` PR / `240 s` job | Within cap (prompted PR #16) |
| PR #18 CI (`ef-clinic-client`, post-PR #16 Wasm scoping; Run `46025559983`) | **`208 s`** (`rust-native` `182 s`, `rust-wasm` **`72 s`**, `cargo-deny` `53 s`, `lightweight` `15 s`) | `360 s` PR / `240 s` job | **1.7x faster** than PR cap; Wasm **2.9x faster** |

---

## 6. Derived Effort & Sizing Estimate for M3 Infinity (Increments 1–5)

Per [`docs/program-plan.md`](../program-plan.md) Section 10, M3 sizing is calibrated strictly from measured M2 effort (`18,458` LOC across 7 crates / 6 PRs covering 13 requirements, with `5,629` LOC of shared UI/core foundation already landed) and scaled by each increment's formal requirement count, state-machine invariant count, external protocol/format breadth, and UI surface count in [`specs/packages/m3-suite/spec.md`](../../specs/packages/m3-suite/spec.md).

**Calibration unit**: `1.0 M2-slice` = `~12.8k` incremental product LOC (`18,458 - 5,629` sunk foundation), `~4–5` implementation PRs + `1` verification PR. Under the Program Stop Rule (`> 2.0x` estimate triggers review), each increment lists both its expected estimate and its `2.0x` stop-rule ceiling.

| Increment | M3 Scope & Requirements (`specs/packages/m3-suite`) | Primary Complexity Drivers | Est. New / Extended Crates | Est. LOC (Src + Tests + DDL) | Est. PRs | Relative Effort (`M2 = 1.0x`) | Stop-Rule Review Ceiling (`2.0x`) |
| --- | --- | --- | --- | ---: | ---: | ---: | ---: |
| **Increment 1** | **Remaining Curie Clinic** (`EF-CLINIC-0010`–`0015`, 6 reqs) + M2 HTTP/PG/Valkey unification | Today urgency triage, message matching (14 formal invariants), reply drafting/withholding (12 formal invariants), memory rounds watermark, 16-bucket multi-format EHR mapping (FHIR R4, Kipu/Part 2, C-CDA, HL7 v2, SHL, Gmail/Spruce), plus M2 route/PG/Valkey wiring | Extend `ef-clinic-domain`, `ef-clinic-server`, `ef-clinic-client`; add `ef-ehr-ingest`, `ef-clinic-messaging` (`+2` crates) | `14,000–18,000` | `5–7` | **`1.25x`** | `2.5x` (`~36k` LOC / `14` PRs) |
| **Increment 2** | **Curie Mobile** (`EF-MOBILE-0001`–`0005`, 5 reqs) | Standalone mobile auth (6 invariants, offline owed sign-out), portal/SHL per-record savepoints + Part 2, HealthKit/Health Connect account-fenced outbox, **For my doctor** sharing (13 invariants), account deletion & independent privacy receipt journal replay, Bevy mobile client + narrow Android/iOS shims | Add `ef-mobile-domain`, `ef-mobile-server`, `ef-mobile-client`, `ef-privacy-journal` (`+4` crates; reuses `ef-core` & `ef-ui-catalog`) | `13,000–17,000` | `5–6` | **`1.15x`** | `2.3x` (`~34k` LOC / `12` PRs) |
| **Increment 3** | **Connected Care Bridge** (`EF-CONN-0001`–`0002`, 2 reqs) | Single-owner cross-product bridge, invitations, 5-wrong-DOB permanent lockout, Share consent (11 invariants), multi-practice independence, end-during-uncertain-send & rejoin, strict zero-shared-clinical-table isolation | Add `ef-connected-care` + wire into clinic/mobile APIs & clients (`+1` crate) | `4,500–6,500` | `2–3` | **`0.45x`** | `0.9x` (`~13k` LOC / `6` PRs) |
| **Increment 4** | **Shared Execution, Clinical Library & Patient Bank** (`EF-HARNESS-0002`–`0004`, 3 reqs) | Frozen `skill_view`, JSON Schema tool validation, budget `CutOff`, content-free OTLP telemetry, clinical library nightly pipeline (9 core guarantees + `>= 100 ms` PubMed pacing + single active version), offline semantic encoder, deterministic 468-patient FHIR R4 cohort bank | Extend `ef-harness`; add `ef-clinical-library`, `ef-patient-bank`, `ef-semantic-encoder` (`+3` crates) | `9,500–12,500` | `4–5` | **`0.85x`** | `1.7x` (`~25k` LOC / `10` PRs) |
| **Increment 5** | **Extension, Public Site, Dei Finder & Full Boundary Gate** (`EF-EXT-0001`, `EF-PUBLIC-0001`–`0002`, `EF-PLATFORM-0004`, 4 reqs) | Chrome MV3 extension (memory-only `chrome.storage.session`, named reads, billing redaction, Decision D2 semantic HTML popup), semantic HTML public site + verbatim legal archives + isolated demo, Dei SAMHSA recovery finder (crisis lines first, zero-JS fallback, versioned shard cache), workspace-wide `ef-boundaries` enforcement | Extend `ef-boundaries`; add `ef-extension-bridge` (+ MV3 JS/HTML), `ef-public-site`, `ef-dei-finder` (`+3` crates) | `7,500–10,000` | `4–5` | **`0.70x`** | `1.4x` (`~20k` LOC / `10` PRs) |
| **M3 Total (Increments 1–5)** | **20 requirements (`SCN-M3-0001`–`SCN-M3-0020`)** | Complete clean-room suite replacement across all 6 products | **`+13` new crates** (`27` workspace crates total) | **`48,500–64,000`** | **`20–26`** | **`4.40x` M2** | **`8.8x` M2** *(per-increment gate applies individually)* |

### Key Sizing & Pipeline Risks to Watch in M3

1. **Workspace `rust-native` compile growth**: PR #18 (`14` crates) reached `182 s` on `rust-native` against the `240 s` job budget (`58 s` headroom). Adding `+13` crates in M3 will exceed `240 s` if every PR compiles and tests the full workspace unconditionally. Per-deployable crate scoping (started in PR #16 for Wasm) must be extended to `rust-native` on PRs (running touched + dependent crates on PRs while keeping full workspace compilation on `main` and shared-crate edits) before Increment 2 lands.
2. **Physical-hardware qualification cadence**: Increments 2 and 3 add native Android/iOS health-API and push-notification surfaces (`EF-MOBILE-0001`–`0005`, `EF-CONN-0001`). Hardware sign-off runners (`macos-15`, `windows-latest`, physical Android/iOS test devices) should be provisioned in parallel with Increment 1 so `blocked` cells in [`specs/ledger.toml`](../../specs/ledger.toml) burn down before M4 rehearsal.
3. **EHR format & Part 2 compliance depth in Increment 1**: Multi-format parsers (`FHIR R4`, `Kipu/Part 2`, `C-CDA R2.1`, `HL7 v2.x`) carry the highest specification density in M3; splitting Increment 1 into dedicated PRs (triage/matching/reply first, then EHR ingestion + live PG/Valkey wiring) keeps PR review and CI budgets bounded.
