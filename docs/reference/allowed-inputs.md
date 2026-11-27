# Elysian Fields Allowed-Input Classification

Observer-side inventory of all existing reference inputs, specifications, verification artifacts, assets, and data sets at pinned revision `01b762838d639a51f82e9d7d15e788e02bac772f`. Every item is assigned exactly one clean-room admissibility class:

- **`allowed`**: May be copied directly into the Elysian Realm (`tyler274/ElysianFields`) as an approved input or asset after gatekeeper sign-off, with no rewrite required.
- **`allowed-after-gatekeeper-rewrite`**: Observer-side material that **must never** enter the Elysian Realm verbatim. `Aponia · spec` extracts implementation-neutral behavioral requirements, scenarios, or clean data schemas and strips all reference module layout, Rust signatures, DOM selectors, CSS classes, and internal identifiers before publishing a specification package.
- **`denied`**: Excluded from every role (observers, gatekeepers, builders, verifiers, integrators) or strictly barred from crossing the clean-room wall.

## Summary Classification Matrix

| Input / Content Set | Reference Location & Observed Inventory | Classification | Clean-Room Handling & Rationale |
| --- | --- | --- | --- |
| **Intent specifications** | `intent/` (6 files: `README.md`, `mobile.md`, `physician.md`, `connected-care.md`, `harness.md`, `record-mapping.md`) | `allowed-after-gatekeeper-rewrite` | Owns product truth and names no renderer (`intent/README.md`), but references crate paths (`crates/harness`, `crates/telemetry`, `clinical-library`) and verification file paths. Gatekeeper strips crate/path identifiers into clean spec packages. |
| **REA evidence bundles & clean-room reclamation specs** | `EXPORT-MANIFEST.json` records `696` historical REA v6.8.0 specification bundles across Waves 1–5B; **0** raw `ev_*.json` bundles are committed in `01b762838` (stored externally in local lab storage). Committed artifacts: `book/src/clean-room-reclamation.md`, `book/src/black-box-replacement.md` (cites 2 external IDs: `ev_a3d89be5…`, `ev_5eb06d37…`), and `verification/black-box-replacement/` (7 probe scripts + 7 candidate files). | `allowed-after-gatekeeper-rewrite` | Observer-side only. Historical REA specs specify old module layout, `pub(crate)` signatures, DOM selectors, and CSS classes that have no meaning on Bevy. Only gatekeeper-distilled behavioral requirements may cross the wall. |
| **TLA+ formal protocol models & configs** | `verification/` — **6 executable TLA+ protocol models** (+ `.cfg`) and **4 `Charts.tla` projection modules** (10 `.tla` files total; see table below) | `allowed-after-gatekeeper-rewrite` | State protocol invariants and state machines independent of renderer, making them high-value formal specifications. Require a light gatekeeper scrub of any crate/DOM references in adjacent READMEs and verification comments before crossing the wall; the pure TLA+ state/invariant clauses can then be approved as formal spec inputs. |
| **Chromium browser qualification suites** | 23 Chromium/CDP Rust integration suites across `physician-sample` (17), `mobile` (2), `public-site` (1), `extension-popup` (1), `dei-site` (1), `physician-demo-api` (1), plus `crates/physician-sample/tests/js/*.js` | `allowed-after-gatekeeper-rewrite` | Observer-side scenario sources only. They assert against Dioxus DOM selectors, CSS classes, and `data-*` hooks. Gatekeeper restates every user journey as implementation-neutral behavioral acceptance scenarios. |
| **Android UI Automator test suite** | `platform/android/ModeShellTest.kt`, `crates/android/src/main.rs` | `allowed-after-gatekeeper-rewrite` | Observer-side scenario source for mode-shell switching (`clientThenPhysician`, WebView/keyboard handling). Assertions tied to `dev.dioxus.main.MainActivity` / WebView nodes must be restated behaviorally for Bevy Android. |
| **Brand assets (logos, marks, favicons, social previews)** | 29 raster/SVG files across `crates/public-site/assets/` (21 files incl. partner institution logos & portraits), `crates/physician-app/assets/` (2), `crates/dei-site/assets/` (1), `extension/chrome/icons/` (4) | `allowed` | Non-code brand identity imagery (`dabney-moe-logo-transparent.*`, `wordmark.svg`, `mark.svg`, `favicon.*`, `dabney-{16,32,48,128}.png`). Note: partner institution logos and portraits in `crates/public-site/assets/homepage/assets/` are approved only for the public marketing site, not inside clinical apps. |
| **Typography & font files** | **Zero** bundled `.woff`/`.woff2`/`.ttf`/`.otf` font files and **zero** external font CDN URLs (`fonts.googleapis.com`, etc.) in `01b762838`. Both `crates/ui/assets/tokens.css` and `crates/public-site/assets/css/chrome.css` rely solely on OS system font stacks (`-apple-system, BlinkMacSystemFont, "Inter", "Segoe UI", Roboto, Helvetica, Arial, sans-serif`, `Georgia`, `ui-monospace`). | `allowed` (system-font policy; new bundled fonts require license gate) | Because Bevy renders text onto GPU textures and cannot rely on CSS system font fallback in browser Wasm, M1/M2 must bundle permissively licensed open-source fonts (e.g., SIL OFL 1.1 Noto Sans / Inter / Noto Sans Arabic / Noto Sans CJK / Noto Sans Mono) covering all 9 shipped UI scripts (Latin, CJK, Hangul, Arabic RTL) and pass `cargo-deny`/license review before admission. |
| **Legal & transparency documents** | `crates/public-site/data/legal/` (10 JSON files: `privacy.json`, `terms.json`, `clinical-transparency.json`, `mobile-app-consumer-health-data.json`, `mobile-app-privacy.json` + 3 dated archives `2026-09-09`, `2026-09-23`, `2026-09-24`, `mobile-app-terms.json` + 1 archive `2026-09-08`) | `allowed` | Authored legal policy text and immutable versioned compliance archives (including Washington MHMDA consumer health data notice). Must be preserved verbatim to uphold regulatory and historical archive obligations. |
| **Clinical skills (`skills/`)** | 6 markdown skill files (`mental-health-lingo-cgm`, `patient-communication`, `patient-follow-up`, `patient-memory-upkeep`, `patient-record-retrieval`, `visit-preparation`) | `allowed` | Domain clinical guidance consumed at runtime via `skill_view`. Zero reference Rust source code; content-addressed by SHA-256 in `pin.toml`. Gatekeeper verifies no tool/schema names drift once new harness tool contracts are finalized. |
| **UI i18n strings (9 shipped languages)** | `crates/ui/src/i18n.rs` — 9 `Locale` variants × 51 `CurieMsg` keys = 459 localized UI strings (see language list below) | `allowed-after-gatekeeper-rewrite` | The Rust source file `crates/ui/src/i18n.rs` contains Dioxus signals/hooks and must NOT cross the wall verbatim. Gatekeeper extracts the pure `(message_key, bcp47_locale, text_direction, localized_string)` translation table into a language-agnostic JSON/TOML asset file, which then enters the Elysian Realm as an allowed asset. |
| **Translated repository READMEs (`docs/i18n/`)** | 19 `README.<lang>.md` files (`ar`, `bn`, `de`, `es`, `fr`, `hi`, `id`, `it`, `ja`, `ko`, `mr`, `pt`, `ru`, `ta`, `te`, `tr`, `ur`, `vi`, `zh-CN`) + `repo-metadata.json` | `denied` (not a product UI input; docs-only) | Translated README docs describing the `dabney-moe` reference repo (note: includes Indic scripts `bn`, `hi`, `mr`, `ta`, `te` and `ur` referenced in multilingual shaping tests, which are distinct from the 9 shipped UI locales). Do not import reference repo READMEs into the Elysian Realm; use synthetic multilingual test strings for M1 shaping benchmarks. |
| **Synthetic primary-care patient charts** | `patients/units/` (468 unit ledgers: 60 light, 122 moderate, 286 complex; flagship units `unit-2013`, `unit-1672`, `unit-2188`, `unit-2797`, `unit-1670`) + realized FHIR R4 bundles (`patients/`, `calendar/`, `manifest.json`) | `allowed` (realized FHIR R4 bundles & unit ledgers after PHI-free audit) | Confirmed 100% synthetic (`patients/README.md`: zero real patient data or PHI). Tree SHA-256 pinned in `pin.toml`. Realized FHIR R4 bundles and unit ledgers may be seeded into builder/verifier fixtures once gatekeeper strips any reference crate paths from `patients/README.md`. |
| **SAMHSA recovery facility data shards** | `crates/dei-site/data/recovery/` (56 public-domain SAMHSA FindTreatment.gov state/territory JSON shards) | `allowed` | Public government directory dataset (`FindTreatment.gov`), no reference implementation code. |
| **Workspace Rust source code & crate manifests** | `crates/*` (43 workspace members), `platform/*` Kotlin/Swift bridges, `extension/chrome/*.js` | `denied` | Reference implementation code. Strictly observer-side only under Decision D1; never crosses the wall to builders. |
| **Production credentials & Doppler secrets** | Doppler `prd`/`stg` configs, `OPENAI_API_KEY`, OAuth client secrets, SMTP/Gmail credentials, GCP service account keys, signing certificates | `denied` | Strictly excluded from every role and every repository. |
| **Real patient PHI & live clinical records** | Any real patient charts, EHR exports, health-kit exports, or non-synthetic clinical identifiers | `denied` | Strictly excluded from every role, fixture, test, and model prompt. |
| **Real personal/clinic messages** | Any real SMS/WhatsApp/Gmail/Spruce message archives (including `workspace-tools messages` offline personal archive) | `denied` | Strictly excluded from every role. |
| **Sealed evaluation fixtures (`eval/`)** | Any `eval/` directory (not present in `01b762838`, enforced globally by policy) | `denied` | Sealed evaluation material; strictly unreadable and excluded from every role, prompt, fixture, and model-visible context. |

## Detailed Inventories

### 1. TLA+ Models Under `verification/`

| # | Domain | Specification Module | Config | Projection Module | Scope |
| --- | --- | --- | --- | --- | --- |
| 1 | Connected care invitations | `verification/connected-care/invitation/Invitation.tla` | `Invitation.cfg` | — | Single open invitation, resend/cancel, 5-attempt DOB lockout, explicit Share consent |
| 2 | Curie Mobile sharing | `verification/mobile-app/shared/Shared.tla` | `Shared.cfg` | — | `For my doctor` `To send` queue, immutable confirmed sends, idempotent retries, source removals |
| 3 | Curie Mobile sign-in | `verification/mobile-app/sign-in/MobileSignIn.tla` | `MobileSignIn.cfg` | `Charts.tla` (5 statecharts) | Session lifecycle, owed sign-out, account-switch draft/record clearing |
| 4 | Physician sign-in | `verification/physician/sign-in/SignIn.tla` | `SignIn.cfg` | `Charts.tla` (5 statecharts) | Single-use email links, multi-tab revocation propagation, empty sign-in clearing |
| 5 | Physician message matching | `verification/physician/message-matching/MessageMatching.tla` | `MessageMatching.cfg` | `Charts.tla` (6 statecharts) | Unambiguous auto-linking, manual suggestions, reversible moves & Undo, backlog consent |
| 6 | Physician message reply | `verification/physician/message-reply/MessageReply.tla` | `MessageReply.cfg` | `Charts.tla` (4 statecharts) | Linked message review, physician-owned drafts, revisions, withholding after patient move |

Additional prose-only verification specs under `verification/` (all `allowed-after-gatekeeper-rewrite` due to crate/tool references): `clinical-library/nightly/README.md`, `physician/chat/README.md`, `physician/citations/README.md`, `physician/memory/README.md`.

### 2. Nine Shipped UI Languages (`crates/ui/src/i18n.rs`)

`crates/ui/src/i18n.rs` defines **9** compile-time exhaustive UI locales (`51` `CurieMsg` keys each = `459` strings), persisted under `curie.locale.v1`:

| # | Enum Variant | BCP-47 Tag | Native Name | Short Code | Script Family | Direction |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | `En` | `en` | English | `EN` | Latin | `ltr` |
| 2 | `Es` | `es` | Español | `ES` | Latin | `ltr` |
| 3 | `Fr` | `fr` | Français | `FR` | Latin | `ltr` |
| 4 | `Pt` | `pt` | Português | `PT` | Latin | `ltr` |
| 5 | `De` | `de` | Deutsch | `DE` | Latin | `ltr` |
| 6 | `ZhHans` | `zh-Hans` | 简体中文 | `中文` | Han (Simplified CJK) | `ltr` |
| 7 | `Ja` | `ja` | 日本語 | `日本語` | Kanji / Kana | `ltr` |
| 8 | `Ko` | `ko` | 한국어 | `한국어` | Hangul | `ltr` |
| 9 | `Ar` | `ar` | العربية | `العربية` | Arabic | `rtl` |

*Observation note:* The program plan mentions "nine shipped languages, including Arabic and the Indic scripts." At pinned SHA `01b762838`, the **9 shipped UI locales** in `crates/ui/src/i18n.rs` are the nine listed above (including Arabic RTL, CJK, and Hangul, with **no** Indic UI locale yet); Indic scripts (`bn` Bengali, `hi` Hindi, `mr` Marathi, `ta` Tamil, `te` Telugu) and `ur` Urdu appear only in the 19 translated repository README files under `docs/i18n/`. M1 text-shaping qualification should still test Indic scripts (Devanagari/Bengali/Tamil) alongside Arabic RTL and CJK so complex script shaping (`cosmic-text` / `harfbuzz`) is proven upfront.

### 3. REA Evidence Bundle Count Reconciliation

- `EXPORT-MANIFEST.json` records `rea_specification_bundles: 696` from historical reclamation waves (Waves 1–5B on prior `dabney-moe` history).
- A full scan of the pinned tree `01b762838` confirms **0** `ev_*.json` REA evidence bundles are checked into Git (`book/src/black-box-replacement.md` explicitly states raw captures and evidence JSONs remain outside Git in local lab directories and cites 2 external `ev_` IDs).
- Committed black-box/REA artifacts in `01b762838` consist of `book/src/clean-room-reclamation.md`, `book/src/black-box-replacement.md`, `EXPORT-MANIFEST.json`, `verification/black-box-replacement/probes/` (7 scripts), and `verification/black-box-replacement/replacement/` (7 standalone roster-codec candidate files). All are observer-side only (`allowed-after-gatekeeper-rewrite`).
