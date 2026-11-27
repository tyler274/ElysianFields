# M3 Infinity Verification Report and Delivery Audit

This report records the independent verification of **Milestone M3 (Infinity — Suite replacement, Increments 1–5)** at `main` revision **`5153302`** (`5153302ab8462aac32a37a6eda32d89e7f8ced7e`), audits all ten new M3 crates, the Chrome MV3 extension package under [`extension/`](../../extension/manifest.json), and the workspace-wide boundary gate [`ef-boundaries`](../../crates/ef-boundaries/src/lib.rs) against every requirement (`EF-CLINIC-0010`–`0015`, `EF-MOBILE-0001`–`0005`, `EF-CONN-0001`–`0002`, `EF-HARNESS-0002`–`0004`, `EF-EXT-0001`, `EF-PUBLIC-0001`–`0002`, `EF-PLATFORM-0004`) and executable scenario (`SCN-M3-0001`–`0020`) in [`specs/packages/m3-suite/`](../../specs/packages/m3-suite/spec.md), and records remaining hardware and live-service integration gaps honestly.

---

## 1. Executive Verification Verdict

**Verdict: PASS WITH RECORDED INTEGRATION & HARDWARE GAPS** at revision **`5153302`**.

- **Full workspace verification (`./scripts/dev-check.sh --tier full`)**: **PASS in `2m17s`** across all **24 workspace crates** (`cargo-fmt` `6 s`, `python-tests` `8 s` / `70` tests, `clippy-host` `-D warnings` `24 s`, `clippy-wasm` `wasm32-unknown-unknown` `-D warnings` `32 s`, `cargo-test` `1m06s` / **`118 / 118`** Rust tests with `0` failed and `0` ignored, `docs-and-wall` `1 s` with **`196 / 196`** files clean, `0` wall violations, and `mdbook build book` clean).
- **Supply-chain & license audit (`cargo deny --all-features check advisories bans licenses sources`)**: **PASS in `10 s`** (`advisories ok, bans ok, licenses ok, sources ok`; zero new third-party dependencies added across M3).
- **Ledger schema & clean-room wall validators**: **PASS** (`Ledger validation OK: 33 requirement(s) verified in specs/ledger.toml`, `Clean-room wall check OK: 196 file(s) scanned, 0 violations`). Across the **20 M3 requirements** (`45` platform coverage cells), **`27` cells** are `passed` on verified Linux/Wasm/hosted/static/MV3-package surfaces, **`18` cells** on unexercised physical mobile/OS targets (`browser-wasm-mobile`, `android-native`, `ios-native`) remain explicitly **`blocked`**, and **`0` cells** remain `unknown` anywhere in [`specs/ledger.toml`](../../specs/ledger.toml).
- **Formal invariant coverage**: All **78** named formal invariants and safety guarantees across all seven M3 state-machine families (**14** `MessageMatching`, **12** `MessageReply`, **6** `MobileSignIn`, **13** `Shared` (`For my doctor`), **11** `Invitation`, **9** clinical-library nightly, and **13** privacy `RestrictionOperation` operations) are implemented in typed domain owners and exercised directly by automated scenario tests.
- **Decision D2 MV3 popup proof**: Uncompressed [`extension/`](../../extension/manifest.json) popup assets (`popup.html` `1,212 B` + `popup.css` `1,363 B` + `popup.js` `1,772 B`) total **`4,347 bytes`** (**`0.41%`** of the `1.0 MiB` / `1,048,576 B` cap — **241x smaller** than the ceiling); the entire 6-file extension package totals **`11,510 bytes`** (`1.10%` of cap).
- **Delivery vs. M2 sizing estimate**: M3 landed in **`19,951` Rust/SQL LOC** across **10 new crates** (**`20,198` Rust/SQL LOC** including the `ef-boundaries` expansion, plus the `11,510 B` `extension/` MV3 bundle) across **5 product PRs (#21–#25)** (plus PR #20 CI native scoping), well below the M2 calibration estimate (`48,500–64,000` LOC / `20–26` PRs) and far inside the `2.0x` Program Stop Rule ceiling.

---

## 2. Commands Executed & Observed Outputs (Revision `5153302`)

| Gate | Exact Command | Observed Output | Duration | Status |
| --- | --- | --- | ---: | --- |
| Full local CI gate | `nix shell nixpkgs#python3 nixpkgs#mdbook -c ./scripts/dev-check.sh --tier full` | `cargo-fmt` `6 s`, `python-tests` `8 s` (`70` tests), `clippy-host` `24 s`, `clippy-wasm` `32 s`, `cargo-test` `1m06s` (**`118 passed, 0 failed, 0 ignored`** across 24 crates), `docs-and-wall` `1 s` (`196` files, `0` wall violations, `mdbook build` clean) | `2m17s` | **PASS** |
| Dependency & license gate | `nix shell nixpkgs#cargo-deny -c cargo deny --all-features check advisories bans licenses sources` | `advisories ok, bans ok, licenses ok, sources ok` (non-blocking duplicate transitive warnings only) | `10 s` | **PASS** |
| Ledger & wall validators | `nix shell nixpkgs#python3 -c python3 scripts/check_ledger.py && nix shell nixpkgs#python3 -c python3 scripts/check_wall.py` | `Ledger validation OK: 33 requirement(s) verified in specs/ledger.toml.` and `Clean-room wall check OK: 196 file(s) scanned, 0 violations.` | `1 s` | **PASS** |

---

## 3. M3 Increment 1–5 Requirement-to-Test Traceability Matrix (`5153302`)

Every M3 requirement (`EF-CLINIC-0010`–`0015`, `EF-MOBILE-0001`–`0005`, `EF-CONN-0001`–`0002`, `EF-HARNESS-0002`–`0004`, `EF-EXT-0001`, `EF-PUBLIC-0001`–`0002`, `EF-PLATFORM-0004`) maps to its executable scenario, owning crate(s), verified test(s), command & observed result, revision SHA, and honest platform disposition in [`specs/ledger.toml`](../../specs/ledger.toml):

| Req ID | Scenario ID | Inc | Owning Crate(s) | Verifying Test(s) | Command & Observed Output | Revision | Target Disposition |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `EF-CLINIC-0010` | `SCN-M3-0001` | 1 | `ef-clinic-workflows` | `scn_m3_0001_today_queue_urgency_incomplete_honesty_and_channel_banners` | `cargo test --locked -p ef-clinic-workflows` -> `5 passed, 0 failed` | `5153302` | `browser-wasm-desktop`, `desktop-linux`, `hosted-services`: **passed** |
| `EF-CLINIC-0011` | `SCN-M3-0002` | 1 | `ef-clinic-workflows` | `scn_m3_0002_and_0003_all_14_message_matching_invariants` | `cargo test --locked -p ef-clinic-workflows` -> `5 passed, 0 failed` | `5153302` | `browser-wasm-desktop`, `hosted-services`: **passed** |
| `EF-CLINIC-0012` | `SCN-M3-0003` | 1 | `ef-clinic-workflows` | `scn_m3_0002_and_0003_all_14_message_matching_invariants` | `cargo test --locked -p ef-clinic-workflows` -> `5 passed, 0 failed` | `5153302` | `browser-wasm-desktop`, `hosted-services`: **passed** |
| `EF-CLINIC-0013` | `SCN-M3-0004` | 1 | `ef-clinic-workflows` | `scn_m3_0004_all_12_message_reply_invariants` | `cargo test --locked -p ef-clinic-workflows` -> `5 passed, 0 failed` | `5153302` | `browser-wasm-desktop`, `hosted-services`: **passed** |
| `EF-CLINIC-0014` | `SCN-M3-0005` | 1 | `ef-clinic-workflows` | `scn_m3_0005_patient_memory_rounds_watermark_quiet_noop_disagreement_and_retirement` | `cargo test --locked -p ef-clinic-workflows` -> `5 passed, 0 failed` | `5153302` | `hosted-services`: **passed** |
| `EF-CLINIC-0015` | `SCN-M3-0006` | 1 | `ef-clinic-workflows` | `scn_m3_0006_multi_format_ehr_mapping_native_bytes_part2_and_binding_checks` | `cargo test --locked -p ef-clinic-workflows` -> `5 passed, 0 failed` | `5153302` | `hosted-services`: **passed** |
| `EF-MOBILE-0001` | `SCN-M3-0007` | 2 | `ef-mobile-domain`, `ef-mobile-server`, `ef-mobile-client` | `scn_m3_0007_mobile_sign_in_offline_owed_sign_out_flush_and_handoff_isolation`, `mobile_server_tcp_wal_independent_journal_and_restore_replay_readyz_gate` | `cargo test --locked -p ef-mobile-domain -p ef-mobile-server -p ef-mobile-client` -> `6 passed, 0 failed` | `5153302` | `browser-wasm-mobile`, `android-native`, `ios-native`: **blocked** |
| `EF-MOBILE-0002` | `SCN-M3-0008` | 2 | `ef-mobile-domain`, `ef-mobile-server`, `ef-mobile-client` | `scn_m3_0008_portal_pkce_shl_per_record_savepoints_and_part2_boundaries`, `mobile_server_tcp_wal_independent_journal_and_restore_replay_readyz_gate` | `cargo test --locked -p ef-mobile-domain -p ef-mobile-server -p ef-mobile-client` -> `6 passed, 0 failed` | `5153302` | `hosted-services`: **passed**; `browser-wasm-mobile`, `android-native`, `ios-native`: **blocked** |
| `EF-MOBILE-0003` | `SCN-M3-0009` | 2 | `ef-mobile-domain`, `ef-mobile-client` | `scn_m3_0009_health_outbox_account_fencing_bevy_ui_isolation_and_push_revocation` | `cargo test --locked -p ef-mobile-domain -p ef-mobile-client` -> `5 passed, 0 failed` | `5153302` | `android-native`, `ios-native`: **blocked** |
| `EF-MOBILE-0004` | `SCN-M3-0010` | 2 | `ef-mobile-domain`, `ef-mobile-server`, `ef-mobile-client` | `scn_m3_0010_for_my_doctor_13_invariants_and_part2_written_consent_gate`, `mobile_server_tcp_wal_independent_journal_and_restore_replay_readyz_gate` | `cargo test --locked -p ef-mobile-domain -p ef-mobile-server -p ef-mobile-client` -> `6 passed, 0 failed` | `5153302` | `hosted-services`: **passed**; `browser-wasm-mobile`, `android-native`, `ios-native`: **blocked** |
| `EF-MOBILE-0005` | `SCN-M3-0011` | 2 | `ef-mobile-domain`, `ef-mobile-server`, `ef-mobile-client` | `scn_m3_0011_portable_export_account_deletion_and_isolated_restore_replay_gate`, `mobile_server_tcp_wal_independent_journal_and_restore_replay_readyz_gate` | `cargo test --locked -p ef-mobile-domain -p ef-mobile-server -p ef-mobile-client` -> `6 passed, 0 failed` | `5153302` | `hosted-services`: **passed**; `android-native`, `ios-native`: **blocked** |
| `EF-CONN-0001` | `SCN-M3-0012` | 3 | `ef-connected-care` | `scn_m3_0012_invitation_lifecycle_all_11_invariants`, `part2_written_consent_and_verbatim_notice_gate_with_audit_trail`, `closed_schema_json_serialization_matches_m3_contract`, `bridge_crate_manifest_stays_headless_and_product_isolated` | `cargo test --locked -p ef-connected-care` -> `6 passed, 0 failed` | `5153302` | `browser-wasm-desktop`, `hosted-services`: **passed**; `browser-wasm-mobile`, `android-native`, `ios-native`: **blocked** |
| `EF-CONN-0002` | `SCN-M3-0013` | 3 | `ef-connected-care` | `scn_m3_0013_multi_practice_independence_disconnect_during_uncertain_send_and_rejoin`, `account_deletion_and_restore_replay_terminates_links_idempotently` | `cargo test --locked -p ef-connected-care` -> `6 passed, 0 failed` | `5153302` | `browser-wasm-desktop`, `hosted-services`: **passed**; `browser-wasm-mobile`: **blocked** |
| `EF-HARNESS-0002` | `SCN-M3-0014` | 4 | `ef-harness`, `ef-clinical-lib` | `scn_m3_0014_shared_harness_skill_view_tool_validation_cutoff_and_content_free_telemetry`, `stub_provider_executes_two_tool_rounds_streams_answer_and_emits_content_free_telemetry`, `clinical_tool_enforces_cross_patient_isolation_fail_closed` | `cargo test --locked -p ef-harness -p ef-clinical-lib` -> `6 passed, 0 failed` | `5153302` | `hosted-services`: **passed** |
| `EF-HARNESS-0003` | `SCN-M3-0015` | 4 | `ef-clinical-lib` | `scn_m3_0015_all_9_nightly_guarantees_and_deidentified_concept_queries`, `scn_m3_0015_pubmed_100ms_concurrent_pacing_verbatim_hash_and_auth_fail_fast` | `cargo test --locked -p ef-clinical-lib` -> `3 passed, 0 failed` | `5153302` | `hosted-services`: **passed** |
| `EF-HARNESS-0004` | `SCN-M3-0016` | 4 | `ef-encoder`, `ef-patient-bank` | `offline_encoder_produces_deterministic_l2_normalized_clinical_embeddings`, `scoped_batch_index_ranks_relevant_notes_with_zero_cross_patient_leakage`, `scn_m3_0016_byte_deterministic_468_patient_bank_validation_and_idempotent_import` | `cargo test --locked -p ef-encoder -p ef-patient-bank` -> `3 passed, 0 failed` | `5153302` | `hosted-services`: **passed** |
| `EF-EXT-0001` | `SCN-M3-0017` | 5 | `ef-extension`, `extension/` | `matches_nist_sha256_vectors`, `scn_m3_0017_mv3_package_passes_manifest_csp_neutrality_and_d2_size_audit`, `scn_m3_0017_credentials_live_strictly_in_memory_only_session_storage`, `scn_m3_0017_declared_named_read_jobs_accept_valid_and_reject_raw_url_smuggling`, `scn_m3_0017_billing_and_price_redaction_produces_verified_sha256_receipt_in_outbox` | `cargo test --locked -p ef-extension` -> `5 passed, 0 failed` | `5153302` | `chrome-extension-popup`: **passed** *(headed Chrome toolbar timing recorded in Section 5.2)* |
| `EF-PUBLIC-0001` | `SCN-M3-0018` | 5 | `ef-public-site` | `scn_m3_0018_semantic_public_and_legal_routes_render_verbatim_without_js_or_canvas`, `scn_m3_0018_synthetic_demo_isolation_citations_turn_budget_and_ingress_limits` | `cargo test --locked -p ef-public-site` -> `3 passed, 0 failed` | `5153302` | `static-public-web`, `hosted-services`: **passed** |
| `EF-PUBLIC-0002` | `SCN-M3-0019` | 5 | `ef-public-site` | `scn_m3_0019_dei_crisis_lines_first_zero_js_fallback_and_versioned_shard_offline_lifecycle` | `cargo test --locked -p ef-public-site` -> `3 passed, 0 failed` | `5153302` | `static-public-web`, `browser-wasm-desktop`: **passed**; `browser-wasm-mobile`: **blocked** |
| `EF-PLATFORM-0004` | `SCN-M3-0020` | 5 | `ef-boundaries` | `scn_m3_0020_workspace_manifests_satisfy_all_architecture_boundaries`, `scn_m3_0020_headless_classification_covers_all_m2_and_m3_headless_crates`, `scn_m3_0020_negative_injections_fail_closed_with_explicit_diagnostics` | `cargo test --locked -p ef-boundaries` -> `3 passed, 0 failed` | `5153302` | `hosted-services`: **passed** |

---

## 4. Independent Code, Invariant, Boundary & Decision D2 Audit

### 4.1 Formal State-Machine & Safety Invariant Coverage (78 / 78 Verified)

| Invariant Family | Count | Owning Module | Key Audited Guarantees | Verifying Test(s) |
| --- | ---: | --- | --- | --- |
| `MessageMatching` | **14** | [`crates/ef-clinic-workflows/src/matching.rs`](../../crates/ef-clinic-workflows/src/matching.rs) | `ExactlyOnce` cursor, `LinksOnItsOwn` auto-link, `OnlyWithAccess`, `SuggestionsWithAccess`, `NeverInvisible` (`15`-minute undecided fallback), `OnlyBetweenTheirPatients`, `ActsAfterUndo`, `UndoPutsBack`, `RightPatient`, `CantReadShows`, `ClearedOnDisconnect`, `BacklogOnlyWhenAccepted` | `scn_m3_0002_and_0003_all_14_message_matching_invariants` |
| `MessageReply` | **12** | [`crates/ef-clinic-workflows/src/replies.rs`](../../crates/ef-clinic-workflows/src/replies.rs) | `OnlyTheReviewAnswers` (auto-reply never answers), `HistoryTrue`, `OneReviewEach`, internal citation markup rejection/sanitization, `OnlyWithAccess`, `YourDraft`, `CopiesWhatYouSee`, `NoteWhileDisconnected`, `ConcernStays`, `UndoBringsBack`, `WithheldAfterMove`, crash/restart recovery to `Woken` | `scn_m3_0004_all_12_message_reply_invariants` |
| `MobileSignIn` | **6** | [`crates/ef-mobile-domain/src/account.rs`](../../crates/ef-mobile-domain/src/account.rs) | `check_all_6_invariants`: offline owed sign-out queue (`owes`), online flush (`OwedSessionsEventuallyEnd`, `ServerEndsSignOut`), `SignInHoldsNothing`, `DraftMatchesAccount`, `NoDeadlock`, device handoff isolation | `scn_m3_0007_mobile_sign_in_offline_owed_sign_out_flush_and_handoff_isolation` |
| `Shared` (`For my doctor`) | **13** | [`crates/ef-mobile-domain/src/for_my_doctor.rs`](../../crates/ef-mobile-domain/src/for_my_doctor.rs) | `verify_invariants`: `JoinAndLeaveSafe`, `SaveHonesty`, `NothingReachesWithoutConfirm`, `DisclosureBeforeSend`, `SendReachesAtMostOnce`, `ConfirmedSendClearsCarriedOnly`, `SendHoldsExactImmutableContent`, `SourceCorrectionSafe`, `SentAndReceivedTruth`, `SourceRemovalClearsMark`, `ConnectionEndStopsUncertain`, plus 42 CFR Part 2 written consent, separate SUD counseling notes consent, and verbatim § 2.32 notice | `scn_m3_0010_for_my_doctor_13_invariants_and_part2_written_consent_gate` |
| `Invitation` | **11** | [`crates/ef-connected-care/src/bridge.rs`](../../crates/ef-connected-care/src/bridge.rs) | `audit_all_invariants`: `SameConnectionState`, `AtMostOneOpen`, `FailedMailLeavesOpen`, `ResendNeverSplitsOrUnlocks`, `PreviewOnlyAfterDob`, `LockedAfterFiveWrongDob`, `CancelledNeverConnects`, `DobVerifiedOnlyAfterMatch`, `OnlyShareGrantsConsent`, `ConnectsOnlyOnOpenShare`, `ShareConnectsSignedInAccount`, plus `PracticesStayIndependent`, metadata-only audit log (zero DOB leakage), and idempotent deletion link termination | `scn_m3_0012_invitation_lifecycle_all_11_invariants`, `scn_m3_0013_multi_practice_independence_disconnect_during_uncertain_send_and_rejoin` |
| Clinical library nightly | **9** | [`crates/ef-clinical-lib/src/nightly.rs`](../../crates/ef-clinical-lib/src/nightly.rs) | De-identified `ConceptQuery` guard, `WaitingToRetry`-only retry scope, `HumanReview` recovery only on explicit/newer-reader trigger, 2-night identical failure code halt, 3-consecutive failed/idle night halt, running/unreadable holder refusal + 2nd unreadable alert, lost-slot isolation (`0` DB writes / `0` artifacts / `0` activations), write-once candidate folders, single checked active version (`active_version_count == 1`), `>= 100 ms` concurrent PubMed start pacing, auth fail-fast | `scn_m3_0015_all_9_nightly_guarantees_and_deidentified_concept_queries`, `scn_m3_0015_pubmed_100ms_concurrent_pacing_verbatim_hash_and_auth_fail_fast` |
| Privacy restriction ledger | **13** | [`crates/ef-mobile-domain/src/privacy.rs`](../../crates/ef-mobile-domain/src/privacy.rs) | `RestrictionOperation::ALL.len() == 13`, create-only independent `FileReceiptJournal`, zero PHI/prose/credentials in receipts, fail-closed `503` on `/readyz` until restore replay completes, non-resurrection fencing via `account_incarnation` | `scn_m3_0011_portable_export_account_deletion_and_isolated_restore_replay_gate`, `mobile_server_tcp_wal_independent_journal_and_restore_replay_readyz_gate` |

### 4.2 Workspace Boundary Gate Audit (`ef-boundaries`, `EF-PLATFORM-0004`)

Audited in [`crates/ef-boundaries/src/lib.rs`](../../crates/ef-boundaries/src/lib.rs) and [`crates/ef-boundaries/tests/workspace_boundaries.rs`](../../crates/ef-boundaries/tests/workspace_boundaries.rs) across all **24 workspace crates**:

1. **Headless enforcement**: All 14 headless crates (`ef-core`, `ef-clinic-domain`, `ef-clinic-workflows`, `ef-harness`, `ef-boundaries`, `ef-clinic-server`, `ef-mobile-domain`, `ef-mobile-server`, `ef-connected-care`, `ef-clinical-lib`, `ef-encoder`, `ef-patient-bank`, `ef-extension`, `ef-public-site`) carry zero `bevy` / `bevy_*` dependencies in `[dependencies]` or `[dev-dependencies]`.
2. **Product isolation**: No `ef-clinic-*` crate imports `ef-mobile-*`, and no `ef-mobile-*` crate imports `ef-clinic-*`. Shared neutral crates (`ef-core`, `ef-connected-care`, `ef-clinical-lib`, `ef-encoder`, `ef-patient-bank`, `ef-extension`, `ef-public-site`) import neither `ef-clinic-*` nor `ef-mobile-*`.
3. **Persistence & secret boundaries**: `ef-harness` has zero persistence/DB dependencies (`sqlx`, `postgres`, `tokio-postgres`, `rusqlite`, `diesel`, `sea-orm`, `redis`, `valkey`, `fred`). All client crates, `web/`, and `extension/` carry zero forbidden secret literals (`sk-`, `sk-ant-`, `AIza`, `ghp_`, `DOPPLER_TOKEN=`, `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `PRIVATE KEY`, etc.).
4. **Negative injection proof**: `scn_m3_0020_negative_injections_fail_closed_with_explicit_diagnostics` verifies that injected `bevy` dependencies in headless crates, cross-product `ef-clinic-*` <-> `ef-mobile-*` imports, `ef-harness` persistence imports, `ef-connected-care` product imports, and client secret strings all fail closed with exact diagnostic error variants.
5. **Audit note on `ef-harness`**: `ef-harness` retains its M2 compile-time dependency on `ef-clinic-domain` for M2 clinical tool types while blocking `ef-mobile-*` and persistence crates; if `ef-harness` is later shared directly with `ef-mobile-*` runtime loops, extracting the clinical tool trait adapter out of `ef-harness` into `ef-clinic-domain` will make `ef-harness` fully product-neutral as well.

### 4.3 Decision D2 MV3 Popup Byte Proof (`EF-EXT-0001`)

Measured directly from disk on revision `5153302` and cross-checked by [`crates/ef-extension/tests/scn_m3_0017_extension.rs`](../../crates/ef-extension/tests/scn_m3_0017_extension.rs) (`audit_extension_package`):

| Surface / File | Uncompressed Size (Bytes) | Share of `1.0 MiB` (`1,048,576 B`) Cap | Notes |
| --- | ---: | ---: | --- |
| [`extension/popup.html`](../../extension/popup.html) | `1,212 B` | `0.12%` | Semantic HTML popup, zero Wasm, zero `<canvas>` |
| [`extension/popup.css`](../../extension/popup.css) | `1,363 B` | `0.13%` | Static CSS, zero inline script/eval |
| [`extension/popup.js`](../../extension/popup.js) | `1,772 B` | `0.17%` | Vanilla JS messaging to background service worker |
| **Popup UI Subtotal (`popup.{html,css,js}`)** | **`4,347 B`** | **`0.41%`** | **241x below `1.0 MiB` Decision D2 cap** |
| [`extension/manifest.json`](../../extension/manifest.json) | `702 B` | `0.07%` | MV3, minimal permissions (`storage`, `activeTab`), strict CSP |
| [`extension/worker.js`](../../extension/worker.js) | `3,197 B` | `0.30%` | Memory-only `chrome.storage.session` credentials, named reads |
| [`extension/connector.js`](../../extension/connector.js) | `3,264 B` | `0.31%` | Content-script declared read dispatch + billing redaction |
| **Full `extension/` Package (6 files)** | **`11,510 B`** | **`1.10%`** | **91x below `1.0 MiB` cap** |
| [`web/mv3-popup/`](../../web/mv3-popup/manifest.json) *(M1 Decision D2 shell)* | `3,930 B` (`3,466 B` popup) | `0.37%` | Reference semantic HTML MV3 harness |

---

## 5. Honest Remaining Gaps & Pre-M4 Integration Items

### 5.1 Live-Service Integration Scope (M4 Rehearsal Focus)

The M3 crates prove every state-machine invariant, wire contract, binary/XML/HL7/FHIR parser, WAL/journal durability rule, and architectural boundary with zero external network dependencies. The following live-infrastructure wirings belong to M4 Vicissitude cutover rehearsal and are explicitly distinguished from unit/domain proof:

1. **Hosted database, pub/sub & network connectors**: `ef-mobile-server` runs real TCP HTTP/1.1 + fsynced JSONL WAL + independent create-only `FileReceiptJournal` + PostgreSQL migration DDL ([`crates/ef-mobile-server/migrations/0001_m3_mobile.sql`](../../crates/ef-mobile-server/migrations/0001_m3_mobile.sql)). `ef-clinic-workflows`, `ef-connected-care`, `ef-clinical-lib`, `ef-patient-bank`, and `ef-public-site` are verified via headless domain, router, and integration harnesses rather than live multi-service PostgreSQL/Valkey/SMTP/OAuth/PubMed network deployments.
2. **M2 clinic client/server route unification**: The pre-production integration items recorded in [`docs/m2/verification-and-estimate.md`](../m2/verification-and-estimate.md) Section 4.2 (live HTTP/SSE transport between `ef-clinic-client` and `ef-clinic-server`, multi-instance PostgreSQL + Valkey fan-out, ingress `429` rate limiting) remain tracked for hosted rehearsal.

### 5.2 Physical Hardware & Headed Browser Proof Gaps (`blocked`)

Headless Linux verification cannot execute native OS rendering, GPU frame pacing, mobile health SDK bridges, or interactive assistive-technology sweeps. The following remain explicitly **`blocked`** in [`specs/ledger.toml`](../../specs/ledger.toml) until executed on representative hardware per [`docs/m1/decision-review.md`](../m1/decision-review.md) Section 5:

- **`android-native` & `ios-native`** (`EF-MOBILE-0001`–`0005`, `EF-CONN-0001`): Physical Android ARM64 (Health Connect, FCM, TalkBack) and iOS ARM64 (HealthKit, APNs, VoiceOver) builds and on-device lifecycle sweeps.
- **`browser-wasm-mobile`** (`EF-MOBILE-0001`, `0002`, `0004`, `EF-CONN-0001`–`0002`, `EF-PUBLIC-0002`): Mobile Safari / Android Chrome touch viewport, service-worker offline reload, and mobile Wasm budgets.
- **Headed desktop browsers & Chrome extension toolbar**: Live Chromium/Firefox/Safari rendering sweeps, real NVDA/VoiceOver screen-reader traversal, and headed Chrome MV3 toolbar open timing (`<= 250 ms` on `chrome-extension-popup`) remain on the pre-release interactive QA checklist.

---

## 6. Measured Workspace Metrics & M3 Delivery vs. Estimate

### 6.1 Complete 24-Crate Workspace Inventory (`5153302`)

| Crate | Milestone | Source LOC | Test LOC | SQL LOC | Total LOC | Passing Tests | Merged PR |
| --- | --- | ---: | ---: | ---: | ---: | ---: | --- |
| `elysian-smoke` | M0 | 118 | 0 | 0 | 118 | 3 | M0 |
| `elysian-bevy-probe` | M0 | 212 | 0 | 0 | 212 | 2 | M0 |
| `m1-accessibility` | M1 | 3,839 | 0 | 0 | 3,839 | 8 | M1 |
| `m1-mobile-lifecycle` | M1 | 2,522 | 939 | 0 | 3,461 | 12 | M1 |
| `m1-streaming-perf` | M1 | 2,967 | 200 | 0 | 3,167 | 5 | M1 |
| `m1-text-i18n` | M1 | 2,905 | 618 | 0 | 3,523 | 12 | M1 |
| `m1-web-delivery` | M1 | 1,681 | 95 | 0 | 1,776 | 9 | M1 |
| `ef-core` | M2 | 844 | 151 | 0 | 995 | 5 | PR #15 |
| `ef-clinic-domain` | M2 | 2,693 | 784 | 0 | 3,477 | 4 | PR #15 |
| `ef-harness` | M2 | 608 | 330 | 0 | 938 | 3 | PR #15 |
| `ef-boundaries` | M2 + M3 | 426 | 160 | 0 | 586 | 3 | PR #15, PR #25 |
| `ef-ui-catalog` | M2 | 4,295 | 0 | 0 | 4,295 | 7 | PR #14 |
| `ef-clinic-server` | M2 | 2,037 | 993 | 117 | 3,147 | 2 | PR #17 |
| `ef-clinic-client` | M2 | 3,523 | 1,744 | 0 | 5,267 | 12 | PR #18 |
| `ef-clinic-workflows` | M3 Inc 1 | 3,097 | 898 | 0 | 3,995 | 5 | PR #22 |
| `ef-mobile-domain` | M3 Inc 2 | 2,665 | 0 | 0 | 2,665 | 0 *(tested via `ef-mobile-client`/`server`)* | PR #24 |
| `ef-mobile-server` | M3 Inc 2 | 1,069 | 286 | 92 | 1,447 | 1 | PR #24 |
| `ef-mobile-client` | M3 Inc 2 | 532 | 864 | 0 | 1,396 | 5 | PR #24 |
| `ef-connected-care` | M3 Inc 3 | 1,890 | 708 | 0 | 2,598 | 6 | PR #21 |
| `ef-clinical-lib` | M3 Inc 4 | 1,854 | 572 | 0 | 2,426 | 3 | PR #23 |
| `ef-encoder` | M3 Inc 4 | 361 | 79 | 0 | 440 | 2 | PR #23 |
| `ef-patient-bank` | M3 Inc 4 | 1,408 | 107 | 0 | 1,515 | 1 | PR #23 |
| `ef-extension` | M3 Inc 5 | 1,016 | 208 | 0 | 1,224 | 5 | PR #25 |
| `ef-public-site` | M3 Inc 5 | 1,851 | 394 | 0 | 2,245 | 3 | PR #25 |
| **M3 New Crates Subtotal (10 crates)** | **M3 Inc 1–5** | **15,743** | **4,116** | **92** | **19,951** | **31** | **PRs #21–#25** |
| **Workspace Total (24 crates)** | **M0–M3** | **44,413** | **10,130** | **209** | **54,752** | **118** | **All merged PRs** |

*(Plus `ef-boundaries` M3 expansion of `+247` LOC across `src/` + `tests/` and `extension/` static MV3 bundle of `11,510` bytes across 6 files; total Git diff additions across M3 product PRs #21–#25 = `+20,976 / -43` lines across `87` files, plus PR #20 CI native scoping `+221 / -31` lines across `3` files.)*

### 6.2 Actual M3 Delivery vs. M2 Sizing Estimate

| Increment | M2 Estimate (`docs/m2/verification-and-estimate.md`) | Actual M3 Delivery (`5153302`) | Stop-Rule Status (`<= 2.0x`) |
| --- | --- | --- | --- |
| **Increment 1** (Remaining Curie Clinic) | `14,000–18,000` LOC / `5–7` PRs | `3,995` LOC (`ef-clinic-workflows`) / `1` PR (#22) | **Well under estimate** |
| **Increment 2** (Curie Mobile) | `13,000–17,000` LOC / `5–6` PRs | `5,508` LOC (`ef-mobile-{domain,server,client}`) / `1` PR (#24) | **Well under estimate** |
| **Increment 3** (Connected Care Bridge) | `4,500–6,500` LOC / `2–3` PRs | `2,598` LOC (`ef-connected-care`) / `1` PR (#21) | **Well under estimate** |
| **Increment 4** (Harness, Clinical Lib, Patient Bank) | `9,500–12,500` LOC / `4–5` PRs | `4,381` LOC (`ef-clinical-lib`, `ef-encoder`, `ef-patient-bank`) / `1` PR (#23) | **Well under estimate** |
| **Increment 5** (Extension, Public/Dei, Boundaries) | `7,500–10,000` LOC / `4–5` PRs | `3,716` LOC (`ef-extension`, `ef-public-site`, `ef-boundaries` delta) + `extension/` / `1` PR (#25) | **Well under estimate** |
| **M3 Total (Increments 1–5)** | **`48,500–64,000` LOC / `20–26` PRs** | **`20,198` Rust/SQL LOC + `extension/` / `5` product PRs** (+ PR #20 CI, PR #26 verification) | **PASS (`0.36x` of midpoint; zero stop-rule triggers)** |
