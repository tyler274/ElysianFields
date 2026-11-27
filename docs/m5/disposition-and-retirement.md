# M5 Daybreak: 43-Item Replacement Disposition & Program Completion Sign-Off

This document closes **Milestone M5 (Daybreak — Retirement)** at `main` revision **`4c9ef67`** (`4c9ef67` head after M4 Vicissitude), records the complete 1-to-1 disposition of all **43** M0 inventory items from [`docs/reference/scope-inventory.json`](../reference/scope-inventory.json) in machine-validated [`docs/m5/disposition.json`](disposition.json), verifies all approved narrow Decision D2 adapters and zero transitional UI stacks, consolidates M0–M5 workspace proof and CI budget performance, and states the remaining owner sign-off checklist honestly.

---

## 1. Executive Completion Verdict

**Verdict: READY FOR OWNER-AUTHORIZED HARDWARE SIGN-OFF & STAGED CUTOVER** at revision **`4c9ef67`**.

- **43 / 43 M0 inventory items accounted for (`scripts/check_disposition.py`)**: **`34` `replace`** (`19` `verified-ready-for-staged-cutover`, `15` `verified-with-hardware-signoff-gate`), **`6` `retain-as-tooling`**, **`3` `out-of-scope`** — exact 1-to-1 bijection enforced permanently in both [`scripts/dev-check.sh`](../../scripts/dev-check.sh) and [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml).
- **33 / 33 requirements covered (`specs/ledger.toml`, `scripts/check_ledger.py`)**: **`51` `passed`** cells across verified Linux/Wasm/hosted/static/MV3-package targets, **`33` `blocked`** cells on unexercised physical OS/mobile hardware targets, and **`0` `unknown`** cells anywhere in the program ledger.
- **Zero transitional product paths**: `scripts/check_disposition.py` and [`crates/ef-boundaries`](../../crates/ef-boundaries/src/lib.rs) verify zero reference product crates in `[workspace].members` or `crates/`, zero `bevy` dependencies in all 15 headless crates, zero cross-product imports between `ef-clinic-*` and `ef-mobile-*`, and zero client or extension secret literals.
- **Clean-room wall & supply-chain integrity**: `scripts/check_wall.py` scans all builder-side `specs/`, `crates/`, and `book/` files with **`0` violations** (`docs/m5/` also passes every wall rule across all scopes); `cargo deny --all-features check advisories bans licenses sources` passes with zero advisories, bans, license, or source violations.

---

## 2. Complete 43-Item Replacement Disposition Table

The machine-readable source of truth is [`docs/m5/disposition.json`](disposition.json), validated 1-to-1 against [`docs/reference/scope-inventory.json`](../reference/scope-inventory.json) and [`specs/ledger.toml`](../../specs/ledger.toml) at revision `4c9ef67`:

| ID | Inventory Key | Zone | Class | Replacement / Owning Target(s) | Requirement ID(s) | Primary Evidence | Retirement Status |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `INV-01` | `agent-skills` | Shared | `replace` | `crates/ef-harness`, `crates/ef-clinical-lib` | `EF-HARNESS-0001`, `EF-HARNESS-0002` | [`docs/m3/verification-report.md`](../m3/verification-report.md) | `verified-ready-for-staged-cutover` |
| `INV-02` | `ancestry` | Ancestry | `out-of-scope` | — | — | [`docs/reference/scope-inventory.json`](../reference/scope-inventory.json) | `out-of-scope` |
| `INV-03` | `ancestry-app` | Ancestry | `out-of-scope` | — | — | [`docs/reference/scope-inventory.json`](../reference/scope-inventory.json) | `out-of-scope` |
| `INV-04` | `ancestry-fit` | Ancestry | `out-of-scope` | — | — | [`docs/reference/scope-inventory.json`](../reference/scope-inventory.json) | `out-of-scope` |
| `INV-05` | `android` | Tooling | `retain-as-tooling` | `crates/m1-mobile-lifecycle/platform/android/README.md` | — | [`docs/m1/mobile-lifecycle-report.md`](../m1/mobile-lifecycle-report.md) | `retained-as-tooling` |
| `INV-06` | `availability-probe` | Tooling | `retain-as-tooling` | `docs/m4/rehearsal-and-rollout.md` | — | [`docs/m4/rehearsal-and-rollout.md`](../m4/rehearsal-and-rollout.md) | `retained-as-tooling` |
| `INV-07` | `boundaries` | Tooling | `retain-as-tooling` | `crates/ef-boundaries` | `EF-PLATFORM-0004` | [`docs/m3/verification-report.md`](../m3/verification-report.md) | `retained-as-tooling` |
| `INV-08` | `clinical-library` | Shared | `replace` | `crates/ef-clinical-lib` | `EF-HARNESS-0003` | [`docs/m3/verification-report.md`](../m3/verification-report.md) | `verified-ready-for-staged-cutover` |
| `INV-09` | `dei-site` | Tooling | `replace` | `crates/ef-public-site` | `EF-PUBLIC-0002` | [`docs/m3/verification-report.md`](../m3/verification-report.md) | `verified-with-hardware-signoff-gate` |
| `INV-10` | `encoder` | Shared | `replace` | `crates/ef-encoder` | `EF-HARNESS-0004` | [`docs/m3/verification-report.md`](../m3/verification-report.md) | `verified-ready-for-staged-cutover` |
| `INV-11` | `extension-popup` | Shared | `replace` | `crates/ef-extension`, `extension/`, `web/mv3-popup/` | `EF-EXT-0001` | [`docs/m3/verification-report.md`](../m3/verification-report.md) | `verified-with-hardware-signoff-gate` |
| `INV-12` | `front-door` | Bridge | `replace` | `crates/ef-connected-care`, `crates/ef-migration-rehearsal` | `EF-CONN-0001`, `EF-CONN-0002` | [`docs/m3/verification-report.md`](../m3/verification-report.md), [`docs/m4/rehearsal-and-rollout.md`](../m4/rehearsal-and-rollout.md) | `verified-with-hardware-signoff-gate` |
| `INV-13` | `gmail` | Shared | `replace` | `crates/ef-clinic-workflows`, `crates/ef-clinic-server` | `EF-CLINIC-0002`, `EF-CLINIC-0010`, `EF-CLINIC-0015` | [`docs/m3/verification-report.md`](../m3/verification-report.md) | `verified-ready-for-staged-cutover` |
| `INV-14` | `harness` | Shared | `replace` | `crates/ef-harness`, `crates/ef-clinical-lib` | `EF-HARNESS-0001`, `EF-HARNESS-0002` | [`docs/m2/verification-and-estimate.md`](../m2/verification-and-estimate.md), [`docs/m3/verification-report.md`](../m3/verification-report.md) | `verified-ready-for-staged-cutover` |
| `INV-15` | `curie-http` | Shared | `replace` | `crates/ef-core`, `crates/ef-clinic-server`, `crates/ef-mobile-server` | `EF-PLATFORM-0001` | [`docs/m2/verification-and-estimate.md`](../m2/verification-and-estimate.md) | `verified-ready-for-staged-cutover` |
| `INV-16` | `local-product` | Physician | `replace` | `crates/ef-clinic-domain`, `crates/ef-clinic-workflows`, `crates/ef-clinic-server` | `EF-CLINIC-0005`–`0009`, `0014`–`0015` | [`docs/m2/verification-and-estimate.md`](../m2/verification-and-estimate.md), [`docs/m3/verification-report.md`](../m3/verification-report.md) | `verified-ready-for-staged-cutover` |
| `INV-17` | `mobile` | Bridge | `replace` | `crates/ef-mobile-client`, `crates/ef-clinic-client`, `crates/m1-mobile-lifecycle` | `EF-MOBILE-0001`, `EF-CONN-0001`, `EF-CONN-0002` | [`docs/m1/mobile-lifecycle-report.md`](../m1/mobile-lifecycle-report.md), [`docs/m3/verification-report.md`](../m3/verification-report.md) | `verified-with-hardware-signoff-gate` |
| `INV-18` | `mobile-api` | Curie Mobile | `replace` | `crates/ef-mobile-server`, `crates/ef-mobile-domain`, `crates/ef-migration-rehearsal` | `EF-MOBILE-0001`, `0002`, `0004`, `0005` | [`docs/m3/verification-report.md`](../m3/verification-report.md), [`docs/m4/rehearsal-and-rollout.md`](../m4/rehearsal-and-rollout.md) | `verified-with-hardware-signoff-gate` |
| `INV-19` | `mobile-app` | Curie Mobile | `replace` | `crates/ef-mobile-client`, `crates/ef-ui-catalog`, `web/elysianfields/` | `EF-MOBILE-0001`, `0002`, `0004`, `0005` | [`docs/m3/verification-report.md`](../m3/verification-report.md) | `verified-with-hardware-signoff-gate` |
| `INV-20` | `mobile-client` | Curie Mobile | `replace` | `crates/ef-mobile-client`, `crates/ef-mobile-domain` | `EF-MOBILE-0001`, `EF-MOBILE-0004` | [`docs/m3/verification-report.md`](../m3/verification-report.md) | `verified-with-hardware-signoff-gate` |
| `INV-21` | `mobile-health` | Curie Mobile | `replace` | `crates/ef-mobile-domain`, `crates/ef-mobile-client`, `crates/m1-mobile-lifecycle` | `EF-MOBILE-0003` | [`docs/m3/verification-report.md`](../m3/verification-report.md) | `verified-with-hardware-signoff-gate` |
| `INV-22` | `mobile-notify` | Curie Mobile | `replace` | `crates/ef-mobile-domain`, `crates/ef-mobile-client`, `crates/m1-mobile-lifecycle` | `EF-MOBILE-0003`, `EF-MOBILE-0005` | [`docs/m3/verification-report.md`](../m3/verification-report.md) | `verified-with-hardware-signoff-gate` |
| `INV-23` | `mobile-stub` | Curie Mobile | `replace` | `crates/ef-mobile-client`, `crates/ef-mobile-server` | `EF-MOBILE-0001`, `0002`, `0004` | [`docs/m3/verification-report.md`](../m3/verification-report.md) | `verified-with-hardware-signoff-gate` |
| `INV-24` | `mobile_wire` | Curie Mobile | `replace` | `crates/ef-mobile-domain`, `crates/ef-mobile-server` | `EF-MOBILE-0001`, `0002`, `0004`, `0005` | [`docs/m3/verification-report.md`](../m3/verification-report.md) | `verified-ready-for-staged-cutover` |
| `INV-25` | `observability-relay` | Tooling | `retain-as-tooling` | `docs/m4/rehearsal-and-rollout.md` | — | [`docs/m4/rehearsal-and-rollout.md`](../m4/rehearsal-and-rollout.md) | `retained-as-tooling` |
| `INV-26` | `patient-bank` | Tooling | `replace` | `crates/ef-patient-bank`, `crates/ef-migration-rehearsal` | `EF-HARNESS-0004` | [`docs/m3/verification-report.md`](../m3/verification-report.md), [`docs/m4/rehearsal-and-rollout.md`](../m4/rehearsal-and-rollout.md) | `verified-ready-for-staged-cutover` |
| `INV-27` | `patient-work` | Physician | `replace` | `crates/ef-clinic-domain`, `crates/ef-clinic-workflows` | `EF-CLINIC-0007`–`0013` | [`docs/m2/verification-and-estimate.md`](../m2/verification-and-estimate.md), [`docs/m3/verification-report.md`](../m3/verification-report.md) | `verified-ready-for-staged-cutover` |
| `INV-28` | `physician-app` | Physician | `replace` | `crates/ef-clinic-client`, `crates/ef-ui-catalog`, `web/elysianfields/` | `EF-CLINIC-0001`–`0013` | [`docs/m2/verification-and-estimate.md`](../m2/verification-and-estimate.md), [`docs/m3/verification-report.md`](../m3/verification-report.md) | `verified-with-hardware-signoff-gate` |
| `INV-29` | `physician-auth` | Physician | `replace` | `crates/ef-clinic-domain`, `crates/ef-clinic-server` | `EF-CLINIC-0001`–`0005` | [`docs/m2/verification-and-estimate.md`](../m2/verification-and-estimate.md) | `verified-ready-for-staged-cutover` |
| `INV-30` | `physician-bus` | Physician | `replace` | `crates/ef-clinic-server`, `crates/ef-clinic-domain` | `EF-HARNESS-0001`, `EF-CLINIC-0007` | [`docs/m2/verification-and-estimate.md`](../m2/verification-and-estimate.md) | `verified-ready-for-staged-cutover` |
| `INV-31` | `physician-client` | Physician | `replace` | `crates/ef-clinic-client`, `crates/ef-clinic-domain` | `EF-CLINIC-0001`, `0007`, `EF-HARNESS-0001` | [`docs/m2/verification-and-estimate.md`](../m2/verification-and-estimate.md) | `verified-with-hardware-signoff-gate` |
| `INV-32` | `physician-demo-api` | Physician | `replace` | `crates/ef-public-site` | `EF-PUBLIC-0001` | [`docs/m3/verification-report.md`](../m3/verification-report.md) | `verified-ready-for-staged-cutover` |
| `INV-33` | `physician-demo-db` | Physician | `replace` | `crates/ef-public-site`, `crates/ef-patient-bank` | `EF-PUBLIC-0001`, `EF-HARNESS-0004` | [`docs/m3/verification-report.md`](../m3/verification-report.md) | `verified-ready-for-staged-cutover` |
| `INV-34` | `physician-sample` | Physician | `replace` | `crates/ef-clinic-server`, `crates/ef-clinic-workflows`, `crates/ef-migration-rehearsal` | `EF-CLINIC-0002`, `0005`–`0009`, `EF-HARNESS-0001`, `EF-PLATFORM-0001` | [`docs/m2/verification-and-estimate.md`](../m2/verification-and-estimate.md), [`docs/m4/rehearsal-and-rollout.md`](../m4/rehearsal-and-rollout.md) | `verified-ready-for-staged-cutover` |
| `INV-35` | `physician-wire` | Physician | `replace` | `crates/ef-clinic-domain`, `crates/ef-clinic-server` | `EF-CLINIC-0005`–`0007` | [`docs/m2/verification-and-estimate.md`](../m2/verification-and-estimate.md) | `verified-ready-for-staged-cutover` |
| `INV-36` | `public-site` | Tooling | `replace` | `crates/ef-public-site` | `EF-PUBLIC-0001`, `EF-PUBLIC-0002` | [`docs/m3/verification-report.md`](../m3/verification-report.md) | `verified-ready-for-staged-cutover` |
| `INV-37` | `recovery-directory` | Shared | `replace` | `crates/ef-public-site` | `EF-PUBLIC-0002` | [`docs/m3/verification-report.md`](../m3/verification-report.md) | `verified-with-hardware-signoff-gate` |
| `INV-38` | `spruce` | Shared | `replace` | `crates/ef-clinic-workflows` | `EF-CLINIC-0010`, `EF-CLINIC-0015` | [`docs/m3/verification-report.md`](../m3/verification-report.md) | `verified-ready-for-staged-cutover` |
| `INV-39` | `telemetry` | Shared | `replace` | `crates/ef-harness`, `crates/ef-clinical-lib`, `crates/ef-migration-rehearsal` | `EF-HARNESS-0001`, `EF-HARNESS-0002` | [`docs/m3/verification-report.md`](../m3/verification-report.md), [`docs/m4/rehearsal-and-rollout.md`](../m4/rehearsal-and-rollout.md) | `verified-ready-for-staged-cutover` |
| `INV-40` | `ui` | Shared | `replace` | `crates/ef-ui-catalog`, `crates/m1-accessibility/web/a11y-bridge.js`, `crates/m1-text-i18n`, `crates/m1-streaming-perf` | `EF-PLATFORM-0002`, `EF-PLATFORM-0003` | [`docs/m1/decision-review.md`](../m1/decision-review.md), [`docs/m2/verification-and-estimate.md`](../m2/verification-and-estimate.md) | `verified-with-hardware-signoff-gate` |
| `INV-41` | `ui-gallery` | Tooling | `replace` | `crates/ef-ui-catalog`, `crates/m1-accessibility`, `crates/m1-streaming-perf` | `EF-PLATFORM-0002`, `EF-PLATFORM-0003` | [`docs/m1/decision-review.md`](../m1/decision-review.md), [`docs/m2/verification-and-estimate.md`](../m2/verification-and-estimate.md) | `verified-with-hardware-signoff-gate` |
| `INV-42` | `workspace-tools` | Tooling | `retain-as-tooling` | `scripts/dev-check.sh`, `scripts/ci_scope.py`, `scripts/ci_timings.py` | — | [`docs/ci/budgets.md`](../ci/budgets.md) | `retained-as-tooling` |
| `INV-43` | `mole-sandbox` | Tooling | `retain-as-tooling` | `scripts/board_split.py`, `scripts/check_wall.py` | — | [`docs/wall/isolation-and-canary.md`](../wall/isolation-and-canary.md) | `retained-as-tooling` |

---

## 3. Approved Narrow Adapters & Transitional Stack Audit

Every interactive application surface (`Curie Clinic` and `Curie Mobile`) runs on the single Bevy `0.20.1` architecture (`ef-ui-catalog`, `ef-clinic-client`, `ef-mobile-client`). Only four strictly bounded, single-owner adapters approved under Decision D2 / Architecture Section 5 exist outside Bevy:

| Approved Adapter | Owning Path(s) | Footprint & Boundary | Second-UI-Stack Check |
| --- | --- | --- | --- |
| **Browser Single-Owner ARIA Mirror** | [`crates/m1-accessibility/web/a11y-bridge.js`](../../crates/m1-accessibility/web/a11y-bridge.js) | `253 LOC` (`7.5 KiB` raw), `0` mirrored CSS rules, `1:1` `accesskit::NodeId` bijection | **PASS** — forwards interactions straight to Rust; holds zero business state |
| **Chrome MV3 Semantic-HTML Popup** | [`extension/popup.html`](../../extension/popup.html), [`extension/popup.css`](../../extension/popup.css), [`extension/popup.js`](../../extension/popup.js) | `4,347 B` raw (`0.41%` of `1.0 MiB` cap), `0` inline scripts, memory-only session tokens | **PASS** — Decision D2 read-only EHR connector surface |
| **Native Android & iOS OS Shims** | [`crates/m1-mobile-lifecycle/platform/android/README.md`](../../crates/m1-mobile-lifecycle/platform/android/README.md), [`crates/m1-mobile-lifecycle/platform/ios/README.md`](../../crates/m1-mobile-lifecycle/platform/ios/README.md) | Stateless OS lifecycle, Keystore/Keychain, Health Connect/HealthKit, and TalkBack/VoiceOver shims | **PASS** — Rust owns lifecycle, `EFSS` v1 state restore, outbox, and AccessKit tree |
| **Public Site & Dei Recovery Finder** | [`crates/ef-public-site/src/lib.rs`](../../crates/ef-public-site/src/lib.rs) | Semantic zero-JS HTML for marketing, verbatim legal archives, isolated demo, and crisis-first Dei finder | **PASS** — indexable semantic web per Architecture Section 5 |

No transitional web UI stack, no dual-client bridge, and no reference product code exist on any accepted execution path in the repository.

---

## 4. Complete M0–M5 Evidence, Test & CI Budget Summary (`4c9ef67`)

### 4.1 Program Milestone Evidence & Workspace Scale

| Milestone | Key Evidence Artifact | Delivered Crates / Scope | Verified Automated Coverage |
| --- | --- | --- | --- |
| **M0 Foundation & Wall** | [`docs/wall/isolation-and-canary.md`](../wall/isolation-and-canary.md), [`docs/reference/scope-inventory.json`](../reference/scope-inventory.json) | `elysian-smoke`, `elysian-bevy-probe`, wall/ledger/board-split tooling | `5` Rust tests; `43`-item reference pin & scope inventory; `0` wall breaches |
| **M1 Helix Feasibility** | [`docs/m1/decision-review.md`](../m1/decision-review.md) | `m1-accessibility`, `m1-mobile-lifecycle`, `m1-streaming-perf`, `m1-text-i18n`, `m1-web-delivery` | `46` Rust tests; `0.024 ms` p95 frame time (`0` idle frames); `1.42 MiB` Brotli-11 Bevy Wasm; Decision D2 recorded |
| **M2 Deliverance Vertical Slice** | [`docs/m2/verification-and-estimate.md`](../m2/verification-and-estimate.md) | `ef-core`, `ef-clinic-domain`, `ef-harness`, `ef-boundaries`, `ef-ui-catalog`, `ef-clinic-server`, `ef-clinic-client` | `36` Rust tests; all `11` sign-in invariants; 3-tier access; real TCP/SSE WAL restart durability; `13/13` M2 reqs in ledger |
| **M3 Infinity Suite Replacement** | [`docs/m3/verification-report.md`](../m3/verification-report.md) | `ef-clinic-workflows`, `ef-mobile-{domain,server,client}`, `ef-connected-care`, `ef-clinical-lib`, `ef-encoder`, `ef-patient-bank`, `ef-extension`, `ef-public-site` | `31` Rust tests; all `78` formal M3 invariants across 7 families; `4,347 B` MV3 popup; `20/20` M3 reqs in ledger |
| **M4 Vicissitude Cutover Rehearsal** | [`docs/m4/rehearsal-and-rollout.md`](../m4/rehearsal-and-rollout.md) | `ef-migration-rehearsal` | `5` E2E rehearsal tests: `468/468` synthetic patient export/import & crash/resume (`668` dedup skips), privacy receipt replay (`0` resurrections), `0` shadow outbound effects, single-writer fencing, clean rollback |
| **M5 Daybreak Retirement Gate** | [`docs/m5/disposition.json`](disposition.json), [`scripts/check_disposition.py`](../../scripts/check_disposition.py) | 43-item disposition bijection + CI/local gate | `43/43` inventory items mapped; `33/33` ledger reqs covered (`0` `unknown`); permanent CI gate |
| **Total Workspace (M0–M5)** | [`specs/ledger.toml`](../../specs/ledger.toml) | **25 Rust crates** (`56,205` Rust/SQL LOC: `45,660` src, `10,336` test, `209` SQL) + `extension/` + `web/` | **`123 / 123` Rust tests** (`0` failed, `0` ignored), **Python CI/wall/ledger/disposition unit tests passing**, **`cargo deny` clean**, **`0` wall violations** |

### 4.2 CI & Deploy Wall-Clock Budgets vs. `DabneyMoeWebsite` Baseline

Enforced automatically on every PR and `main` run by `scripts/ci_timings.py --check-budgets` against [`docs/ci/budgets.json`](../ci/budgets.json):

| Workflow Metric | `DabneyMoeWebsite` Baseline Median (p90) | Elysian Fields Enforced Cap | Measured Speedup Target | Local Parity (`dev-check.sh --tier full`, 25 crates) |
| --- | --- | ---: | ---: | --- |
| PR feedback (`pr_feedback_secs`) | `1,368 s` / `22.8 min` (`1,679 s` p90) | **`360 s` (`6 min`)** | **3.8x faster** | Cold `~2m18s`, warm `~29 s` (fmt + Python + host/wasm32 Clippy `-D warnings` + 123 Rust tests + wall/ledger/disposition + `mdbook`) |
| Merge to live (`merge_to_live_secs`) | `1,340 s` / `22.3 min` (`1,640 s` p90) | **`480 s` (`8 min`)** | **2.8x faster** | Release binaries built once and reused across artifacts |
| Client-only merge (`client_only_merge_secs`) | `~1,340 s` / `22.3 min` | **`240 s` (`4 min`)** | **5.6x faster** | Static `/elysianfields/` bundle packaged without server OCI rebuild |

---

## 5. Honest Remaining Owner Sign-Off Checklist

Automated verification across Linux `x86_64`, `wasm32-unknown-unknown`, headless Bevy `0.20.1`, synthetic TCP/WAL servers, and the 468-patient rehearsal suite is complete. Before transitioning from Stage 0 (Synthetic Rehearsal) to live user traffic or decommissioning any existing production service, the owner must authorize and complete the following remaining gates:

1. **Physical Hardware & Interactive Browser / AT Sign-Off (`33` `blocked` cells in [`specs/ledger.toml`](../../specs/ledger.toml))**:
   - **macOS (`desktop-macos`)**: Apple Silicon `macos-15` runner + signed/notarized `.app`/`.dmg` + Metal frame pacing + VoiceOver & macOS IME sweep.
   - **Windows (`desktop-windows`)**: `windows-latest` runner + Authenticode-signed installer + DX12 frame pacing + NVDA/Narrator & TSF IME sweep.
   - **Android (`android-native`)**: Physical ARM64 Android 14/15 device + signed `.aab` + Health Connect, FCM cold tap, Keystore persistence, TalkBack, `0` background GPU frames.
   - **iOS (`ios-native`)**: Physical ARM64 iOS 17/18 device + signed `.ipa` + HealthKit, APNs cold tap, Keychain persistence, VoiceOver, `0` background GPU frames.
   - **Mobile & Desktop Interactive Browsers (`browser-wasm-mobile`, headed `browser-wasm-desktop`, headed `chrome-extension-popup`)**: Real iOS Safari / Android Chrome / Firefox / macOS Safari rendering & assistive-technology sweeps, plus headed Chrome MV3 toolbar open timing (`<= 250 ms`).
2. **Code-Signing, Store & Cloud Production Credentials (`credential-gated`)**:
   - Provision Apple Developer ID / Distribution, Windows Authenticode, Google Play App Signing, Chrome Web Store publish, Cosign OCI attestation, and Doppler runtime tokens for production registries and clusters.
3. **Hosted Infrastructure & Staged Production Cutover Authorization ([`docs/m4/rehearsal-and-rollout.md`](../m4/rehearsal-and-rollout.md) Section 4)**:
   - Complete hosted PostgreSQL/Valkey/OAuth/SMTP/PubMed deployment wiring and live clinic client/server HTTP/SSE transport parity.
   - Authorize Stage 1 (Staging Shadow `>= 72 h` with `0` outbound side effects) -> Stage 2 (5% Canary `>= 7 d`) -> Stage 3 (25%–50% Expansion `>= 14 d`) -> Stage 4 (100% GA & controlled reference decommissioning after retention and recovery duties are confirmed).
