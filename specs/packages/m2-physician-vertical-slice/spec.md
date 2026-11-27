# Specification Package: M2 Physician Vertical Slice

- **Package**: `specs/packages/m2-physician-vertical-slice`
- **Status**: Gatekeeper (Aponia)-approved for Builder (Vill-V) implementation
- **Domains**: `CLINIC`, `HARNESS`, `PLATFORM`
- **Products**: Curie Clinic (`curie-clinic`), Shared Platform (`shared-platform`)
- **Requirements**: `EF-CLINIC-0002` through `EF-CLINIC-0009`, `EF-HARNESS-0001`, `EF-PLATFORM-0001` through `EF-PLATFORM-0003`
- **Acceptance Scenarios**: `SCN-M2-0001` through `SCN-M2-0012` in [`scenarios.toml`](./scenarios.toml)
- **Machine-Readable Contracts**: [`contracts/chat-command-and-answer.schema.json`](./contracts/chat-command-and-answer.schema.json)

## Package Chapters

This package specifies the end-to-end M2 physician vertical slice — sign-in, patient selection, evidence reading and inline citation inspection, streamed clinical chat with tool-step traces, mid-stream cancel and bounded retry recovery, reload and durable readback, `/elysianfields/` subpath mounting, strict CSP, keyboard and screen-reader accessibility, and nine-language localization with Arabic RTL:

| Chapter | Scope |
| --- | --- |
| [`01-auth-and-sessions.md`](./01-auth-and-sessions.md) | Invite-only sign-in, single-use expiring tokens, session lifetime and sliding renewal, structured refusals, multi-tab sync, and all 11 formal sign-in behavioral invariants |
| [`02-workspace-and-citations.md`](./02-workspace-and-citations.md) | Practice-scoped three-tier access control, patient list and roster search, non-modal citation reading panel state machine, exact-read provenance, and stale/withdrawn redaction |
| [`03-streaming-chat-and-durability.md`](./03-streaming-chat-and-durability.md) | Durable idempotent command envelope, `AnswerState` and `ExchangeSteps`, incremental SSE delta streaming, worker lease and bounded 5-failure retry, composer optimistic-concurrency autosave, reload readback, and deterministic provider fixture contract |
| [`04-subpath-csp-a11y-i18n.md`](./04-subpath-csp-a11y-i18n.md) | `/elysianfields/` subpath mounting and Nix cache hygiene, clinical `no-store` headers and strict CSP, keyboard shortcuts, global privacy screen cover, single-owner focus/accessibility, and nine-language + RTL i18n |
| [`scenarios.toml`](./scenarios.toml) | 12 executable behavioral acceptance scenarios (`SCN-M2-0001`–`SCN-M2-0012`) |

## Shared Architectural Rules

- **Product boundary invariant**: Curie Clinic accounts, clinical records, chat transcripts, and stored state stay strictly isolated from Curie Mobile patient accounts and storage.
- **Performance and platform contracts**: Interactive clients satisfy the budgets in [`specs/m1-feasibility-budgets.md`](../../m1-feasibility-budgets.md) across the target platforms in [`specs/m1-platforms.md`](../../m1-platforms.md).
- **Single state owner**: Domain state stays independent of rendering entities; text input and accessibility bridges maintain one authoritative state owner without duplicate editable inputs or duplicate screen-reader trees.
- **Failure looks like failure**: Failed auth checks, failed patient-list reads, failed citation loads, and failed chat generations display explicit error and recovery states and never render as blank documents or empty clinical lists.

---

## Requirement Register (11-Field Summary)

Each requirement below binds its ID, platforms, provenance, actors, preconditions, actions, visible result, persisted/external effects, failure/restart behavior, accessibility/input behavior, and executable acceptance scenario to the detailed normative rules in Chapters `01`–`04`.

### `EF-CLINIC-0002` — Invite-only sign-in, single-use link verification, and anti-enumeration

- **ID**: `EF-CLINIC-0002`
- **Product and Platforms**: `curie-clinic` — `browser-wasm-desktop`, `desktop-linux`, `desktop-macos`, `desktop-windows`, `android-native`, `ios-native`
- **Provenance**: `observed-and-required`; Authority levels 2 (access and anti-enumeration intent), 3 (auth wire contract), and 4 (behavioral baseline). Normative details in [`01-auth-and-sessions.md`](./01-auth-and-sessions.md) §§1.1–1.3.
- **Actors and Authority**: Unauthenticated visitor, invited active clinician, deactivated clinician, authentication service.
- **Preconditions**: Backend runs with required authentication; Practice A has an active invited clinician and a deactivated clinician; an unknown email does not exist.
- **Actions**: Submit `POST /auth/sign-in` for unknown, deactivated, and active emails; submit `POST /auth/verify` with the issued single-use token; replay the same `POST /auth/verify` request.
- **Visible Result**: Hosted sign-in initiation returns identical anti-enumeration confirmation for all three accounts; first verification signs in and lands on `Today`; replayed verification is rejected cleanly; deactivated verification displays access-removed refusal.
- **Persisted and External Effects**: Outbound mail is sent only for active accounts; single-use 24-hour token consumption and session minting (`12h` idle / `7d` absolute cap / `15m` renewal quantum) occur atomically; all auth responses carry `Cache-Control: no-store`.
- **Failure and Restart Behavior**: Mail delivery failure returns `502` (`mail_delivery_failed`); blank/expired/replayed tokens return `401` (`session_ended`); deactivated accounts return `403` (`access_removed`); deactivation and session fences survive restarts.
- **Accessibility and Input Behavior**: Keyboard-operable sign-in form with accessible labels, IME/autofill support, visible focus indicators, and live-region announcements for verification errors.
- **Executable Acceptance Scenario**: `SCN-M2-0001`.

### `EF-CLINIC-0003` — Two-tab sign-out, expiry clearing, and concurrent-session race protection

- **ID**: `EF-CLINIC-0003`
- **Product and Platforms**: `curie-clinic` — `browser-wasm-desktop`
- **Provenance**: `observed-and-required`; Authority levels 2 (session safety), 3 (11-invariant formal sign-in protocol), and 4 (behavioral baseline). Normative details in [`01-auth-and-sessions.md`](./01-auth-and-sessions.md) §§1.4–1.5.
- **Actors and Authority**: Authenticated clinician operating two concurrent browser tabs sharing origin-scoped storage and cross-tab sync, plus session authority service.
- **Preconditions**: Tab A and Tab B are both open in the same browser profile, authenticated to Practice A, and viewing a patient workspace.
- **Actions**: Expire Session 1 server-side during an in-flight read in Tab A; mint Session 2 in Tab B while a stale Session 1 `401` resolves in Tab A; trigger explicit sign-out in Tab A.
- **Visible Result**: Expiry transitions Tab A immediately to `SessionEndedPrompt` with zero patient content (`PromptIsEmpty`, `Expiry`); late `401` never ejects Tab A once Session 2 is stored (`KeepsWork`, `TodayFirst`); sign-out clears patient views in both tabs promptly (`OnceOut`, `CatchesUp`).
- **Persisted and External Effects**: Sign-out terminates both stored and held sessions server-side (`ServerRefusesSignedOut`) including fallback sign-out when storage is already empty, and broadcasts authority revocation to close open SSE streams immediately across instances.
- **Failure and Restart Behavior**: All 11 formal invariants (`OnceOut`, `Expiry`, `KeepsWork`, `SignedIn`, `TodayFirst`, `Lands`, `ActsAsShown`, `PromptIsEmpty`, `ServerRefusesSignedOut`, `CatchesUp`, `NoDeadlock`) hold without deadlocking `Opening` or `SignInLink`.
- **Accessibility and Input Behavior**: Focus shifts predictably on transition to `SessionEndedPrompt` or `SignInPage` with polite/assertive status announcement explaining why the session ended.
- **Executable Acceptance Scenario**: `SCN-M2-0002`.

### `EF-CLINIC-0004` — Deep-link patient landing across unauthenticated boot

- **ID**: `EF-CLINIC-0004`
- **Product and Platforms**: `curie-clinic` — `browser-wasm-desktop`
- **Provenance**: `observed-and-required`; Authority levels 2 (fail-closed routing) and 3 (`Lands` and `TodayFirst` formal invariants). Normative details in [`01-auth-and-sessions.md`](./01-auth-and-sessions.md) §§1.4–1.5.
- **Actors and Authority**: Unauthenticated clinician opening a direct patient deep link (`?patient=<workspace_id>`), plus sibling tab performing sign-in.
- **Preconditions**: Browser has no stored session; synthetic patient workspace belongs to the clinician's authorized panel.
- **Actions**: Open `/elysianfields/?patient=<workspace_id>` in Tab A; complete direct sign-in in Tab B (or Tab A); separately verify opening an emailed sign-in link resets target to `Today`.
- **Visible Result**: Unauthenticated Tab A displays `SignInPage` with zero protected content; once signed in via direct sign-in, Tab A lands on the deep-linked patient (`Lands`); opening an emailed sign-in link always lands on `Today` (`TodayFirst`).
- **Persisted and External Effects**: Deep-link target stays scoped strictly to in-app routes under `/elysianfields/`; no clinical data fetches succeed before live session establishment.
- **Failure and Restart Behavior**: Reloading during `Opening` preserves safe deep-link intent without stalling; unauthorized deep-link patient IDs resolve fail-closed (`404`) without disclosing patient existence.
- **Accessibility and Input Behavior**: Post-sign-in navigation announces the loaded workspace heading and moves focus to the main landmark cleanly.
- **Executable Acceptance Scenario**: `SCN-M2-0003`.

### `EF-CLINIC-0005` — Practice-scoped three-tier patient access control and cross-practice non-disclosure

- **ID**: `EF-CLINIC-0005`
- **Product and Platforms**: `curie-clinic` — `hosted-services`, `browser-wasm-desktop`
- **Provenance**: `observed-and-required`; Authority levels 2 (practice and panel isolation safety rules) and 4 (behavioral baseline). Normative details in [`02-workspace-and-citations.md`](./02-workspace-and-citations.md) §§2.1–2.2.
- **Actors and Authority**: Authenticated clinician in Practice A with `Allowed` access to panel patient A1, `ClinicDemographic` access to practice patient A2, and `Denied` standing for Practice B patient B1.
- **Preconditions**: Both practices exist in persistence with distinct synthetic patient panels.
- **Actions**: Query `GET /patient-workspaces`; request workspace detail, chat snapshot, and `POST /commands` for Practice B patient B1; attempt clinical chart/citation inspection for `ClinicDemographic` patient A2.
- **Visible Result**: Roster shows only authorized Practice A entries; `ClinicDemographic` surfaces demographic coordination only without clinical chart or transcript disclosure; failed backend reads render an explicit error + **Try again**, never an empty roster.
- **Persisted and External Effects**: Every cross-practice request returns `404 Not Found` with `Cache-Control: no-store` so cross-practice existence is never leaked; partial availability sets `incomplete` / `may_be_missing_someone` honestly.
- **Failure and Restart Behavior**: Store or authorization outages return explicit errors (`503`) rather than falling back to empty lists or permissive defaults.
- **Accessibility and Input Behavior**: Patient finder supports keyboard navigation, `Escape` clear-then-close semantics, and live status announcements for empty vs error states.
- **Executable Acceptance Scenario**: `SCN-M2-0004`.

### `EF-CLINIC-0006` — Evidence reading, inline citation inspection panel, and stale/withdrawn redaction

- **ID**: `EF-CLINIC-0006`
- **Product and Platforms**: `curie-clinic` — `browser-wasm-desktop`, `desktop-linux`, `android-native`, `ios-native`
- **Provenance**: `observed-and-required`; Authority levels 2 (clinical citation provenance and redaction safety) and 3 (M1 citation latency and 5,000-row virtualization budget). Normative details in [`02-workspace-and-citations.md`](./02-workspace-and-citations.md) §2.3.
- **Actors and Authority**: Authenticated clinician with `Allowed` access inspecting cited clinical notes and structured lab records.
- **Preconditions**: Authorized workspace contains a completed answer citing a multi-section progress note (`[1]`) and a 5,000-row structured lab record (`[12]`).
- **Actions**: Activate citation `[1]` to open the non-modal cited-source panel; activate citation `[12]` and scroll through the virtualized 5,000-row lab table; withdraw source `[1]` server-side and re-open it; attempt cross-patient `source_key` resolution.
- **Visible Result**: Non-modal reading panel displays verbatim highlighted quote, clinical date, settled vs flagged (`preliminary`, `inactive`, `refuted`, `entered-in-error`) status badge, and original-language provenance; 5,000-row table opens within budget (`<= 150 ms` browser / `<= 100 ms` desktop / `<= 120 ms` mobile) using virtualized rows; withdrawn source transitions immediately to `Unavailable` and clears prior clinical text.
- **Persisted and External Effects**: Citations trace strictly to retained source revisions actually read during the run; cross-patient source lookup returns `404 Not Found` (`Cache-Control: no-store`).
- **Failure and Restart Behavior**: Monotonic `request_id` fencing discards stale responses when clicking rapidly across chips; transient fetch failures transition to `Failed` with explicit **Try again**; unreadable attachment pages stay explicit without hiding valid sibling pages.
- **Accessibility and Input Behavior**: Citation chips are keyboard-focusable and activatable; `Escape` dismisses the reading panel and restores focus to the activating citation chip; live announcements report `Loaded`, `Unavailable`, and `Failed`.
- **Executable Acceptance Scenario**: `SCN-M2-0005`.

### `EF-CLINIC-0007` — Streamed clinical chat with incremental delta delivery, tool trace, and concurrent interaction

- **ID**: `EF-CLINIC-0007`
- **Product and Platforms**: `curie-clinic` — `browser-wasm-desktop`, `desktop-linux`, `android-native`, `ios-native`
- **Provenance**: `accepted-difference`; Authority levels 1 (M1 stutter elimination mandate), 2 (clinical provenance), and 3 (M1 performance budgets). Intentional architectural divergence: uses incremental SSE token/step deltas into the active turn's typed buffer alongside authoritative snapshot readback on open, reload, and `resync`. Normative details in [`03-streaming-chat-and-durability.md`](./03-streaming-chat-and-durability.md) §§3.1–3.3 and [`contracts/chat-command-and-answer.schema.json`](./contracts/chat-command-and-answer.schema.json).
- **Actors and Authority**: Authenticated clinician with `Allowed` workspace access, durable chat API/worker, and deterministic provider fixture (`stub`).
- **Preconditions**: Workspace preloaded with 80 synthetic turns (~40,000 chars); deterministic stub configured for 2 tool rounds (`search_chart`, `read_records`) followed by 40 tokens/sec streamed markdown citing verified read records.
- **Actions**: Submit prompt via `POST /commands` (`message.submit` with `idempotency_key`); observe ordered `ExchangeSteps` and incremental streamed chunks while simultaneously typing in the composer at 6 keys/sec, scrolling the transcript, and opening a citation chip; repeat the same `idempotency_key`.
- **Visible Result**: Live steps and formatted markdown stream smoothly while meeting frame-time (`p95 <= 16.7 ms`), typing latency (`p95 <= 35 ms` browser / `<= 16.7 ms` desktop), and zero long-task (`> 50 ms`) budgets; settles to `Finished` with interactive citations; idle phase drops to `0 frames/sec` and `< 1%` CPU.
- **Persisted and External Effects**: Turn and pending work item persist durably in PostgreSQL before `200 OK` ACK; duplicate `idempotency_key` creates zero duplicate turns or side effects; idle SSE streams perform zero idle DB polling.
- **Failure and Restart Behavior**: Unreadable steps serialize as `ExchangeSteps::Unavailable` rather than blanking the transcript; SSE drop >30 s past keep-alive reconnects with bounded backoff from the last event `id`.
- **Accessibility and Input Behavior**: Multiline composer supports IME, clipboard, `Enter` send / `Shift+Enter` newline, and polite non-spamming live announcements for start/completion transitions.
- **Executable Acceptance Scenario**: `SCN-M2-0006`.

### `EF-CLINIC-0008` — Mid-stream cancellation (`work.stop`) and explicit `work.resume` with `expected_run_id` fencing

- **ID**: `EF-CLINIC-0008`
- **Product and Platforms**: `curie-clinic` — `browser-wasm-desktop`, `hosted-services`
- **Provenance**: `observed-and-required`; Authority levels 2 (clinician control and honest partial work) and 3 (command envelope contract). Normative details in [`03-streaming-chat-and-durability.md`](./03-streaming-chat-and-durability.md) §§3.1–3.4.
- **Actors and Authority**: Authenticated clinician, API server, background worker, and slow-paced deterministic provider fixture (`delay_ms = 200`).
- **Preconditions**: An exchange is actively streaming (`AnswerState::Working`).
- **Actions**: Activate **Stop** (`POST /commands` `work.stop`); observe `Stopped` state; activate **Resume** (`POST /commands` `work.resume` with `expected_run_id`); replay the stale `work.resume` with the old `expected_run_id`.
- **Visible Result**: Generation halts promptly while preserving all completed steps and kept partial text; **Resume** continues execution to `Finished` with zero lost steps.
- **Persisted and External Effects**: `work.stop` persists cancellation durably and returns any unconsumed queued messages; stale or superseded `work.resume` returns `409 Conflict` without creating a duplicate run; submitting a newer message supersedes earlier `resume` handles.
- **Failure and Restart Behavior**: `work.stop` on an already settled turn returns `409 Conflict`; reloading while `Stopped` preserves partial steps, text, and the valid `Resume` handle.
- **Accessibility and Input Behavior**: **Stop** and **Resume** controls are keyboard-focusable with clear accessible labels and live-region state transition announcements.
- **Executable Acceptance Scenario**: `SCN-M2-0007`.

### `EF-CLINIC-0009` — Bounded 5-failure retry exhaustion to `CouldntFinish` and explicit Try again recovery

- **ID**: `EF-CLINIC-0009`
- **Product and Platforms**: `curie-clinic` — `browser-wasm-desktop`, `hosted-services`
- **Provenance**: `observed-and-required`; Authority levels 2 (failure honesty, no silent infinite retry) and 3 (worker retry contract). Normative details in [`03-streaming-chat-and-durability.md`](./03-streaming-chat-and-durability.md) §3.4 and §3.6.
- **Actors and Authority**: Authenticated clinician, background worker, and fault-injected deterministic provider fixture (`failures = 5`).
- **Preconditions**: Deterministic stub configured to fail 5 consecutive attempts with test-mode immediate backoff (hosted mode enforces `10 s` doubling up to `600 s`).
- **Actions**: Submit a clinical prompt; observe 5 automatic retry attempts exhaust; activate **Try again** (`work.resume` with `expected_run_id`) once provider faults clear.
- **Visible Result**: After exactly 5 failed attempts, UI displays `CouldntFinish` with an honest failure notice and an explicit **Try again** control (zero background retry churn); activating **Try again** runs attempt 6 and settles to `Finished`.
- **Persisted and External Effects**: Attempt count, failure history, and frozen skill/recipe snapshot persist in PostgreSQL across retries; `work.resume` fences on `expected_run_id`.
- **Failure and Restart Behavior**: Worker restarts between retries resume from the exact persisted attempt count without resetting the 5-failure ceiling; non-retryable rejections surface honestly without spinning.
- **Accessibility and Input Behavior**: Failure notice emits an assistive alert announcement and **Try again** receives accessible labeling and keyboard operability.
- **Executable Acceptance Scenario**: `SCN-M2-0008`.

### `EF-HARNESS-0001` — Reload, worker lease reclaim, lossy bus `resync`, durable draft/transcript readback, and deterministic provider fixture

- **ID**: `EF-HARNESS-0001`
- **Product and Platforms**: `curie-clinic` — `browser-wasm-desktop`, `hosted-services`
- **Provenance**: `observed-and-required`; Authority levels 2 (durability and single-owner recovery) and 3 (hosted-shape scale stack and optimistic-concurrency draft contract). Normative details in [`03-streaming-chat-and-durability.md`](./03-streaming-chat-and-durability.md) §§3.3–3.6.
- **Actors and Authority**: Authenticated clinician, two API instances, background worker, PostgreSQL store, lossy pub/sub invalidation bus, and deterministic provider fixture.
- **Preconditions**: Hosted-shape stack running with 2 API instances, worker, PostgreSQL, and lossy Valkey/Redis bus.
- **Actions**: Type and autosave a composer draft; submit a chat turn on Instance A; restart the pub/sub bus and worker mid-run (or expire lease); reload client against Instance B; trigger a concurrent draft `409 Conflict` from a second tab.
- **Visible Result**: Bus restart triggers `resync` and reloads authoritative transcript with zero lost turns or steps; reload against Instance B displays exact persisted transcript, citations, and saved draft; draft `409 Conflict` pauses autosave and presents explicit **Retry (keep mine)** and **Use newer text** choices without silently overwriting local typing.
- **Persisted and External Effects**: Worker leases (`60 s` lease / `300 s` stall reclaim) guarantee single-claim execution and reconstruct live state solely from durable PostgreSQL records; draft dirty state derives solely from `local_text != saved_baseline_text` with generation fencing; deterministic stub captures request JSONL transcripts for verification.
- **Failure and Restart Behavior**: Transient draft save failures show `"Not saved"` with slow backoff and never authorize blind overwrites; submitting clears only the submitted prefix while preserving keystrokes typed after Send.
- **Accessibility and Input Behavior**: Draft save status (`Saved`, `Not saved`, conflict notice) announces politely without stealing composer focus; conflict choices are keyboard-operable.
- **Executable Acceptance Scenario**: `SCN-M2-0009`.

### `EF-PLATFORM-0001` — Subpath mounting under `/elysianfields/`, static cache hygiene, clinical `no-store` headers, and strict Wasm CSP

- **ID**: `EF-PLATFORM-0001`
- **Product and Platforms**: `shared-platform` — `browser-wasm-desktop`, `hosted-services`
- **Provenance**: `observed-and-required`; Authority levels 1 (`/elysianfields/` subpath mandate), 2 (clinical cache isolation and zero client secrets), and 3 (CSP and deployment contract). Normative details in [`04-subpath-csp-a11y-i18n.md`](./04-subpath-csp-a11y-i18n.md) §§4.1–4.2.
- **Actors and Authority**: Browser client, HTTP reverse proxy/application server, and security policy verifier.
- **Preconditions**: Web client bundle and backend APIs are mounted at `/elysianfields/` alongside sibling routes on the same host domain.
- **Actions**: Request bare `GET /elysianfields?patient=<id>`; inspect cache and security headers across HTML, hashed Wasm/JS/font assets, and clinical/auth APIs; boot with a non-loopback `?api=https://attacker.example` parameter; run Wasm boot, WebGL2 init, and SSE streaming under strict CSP.
- **Visible Result**: Bare `/elysianfields` redirects `308 Permanent Redirect` to `/elysianfields/?patient=<id>`; application boots cleanly with zero CSP violations and zero root-path asset 404s.
- **Persisted and External Effects**: HTML carries `Cache-Control: no-cache` with `Last-Modified` and incoming `If-Modified-Since` stripped; hashed assets carry `public, max-age=31536000, immutable`; all clinical and auth endpoints carry `Cache-Control: no-store` (`CDN-Cache-Control: no-store`); non-loopback `?api=` overrides are ignored; zero secrets appear in client bundles.
- **Failure and Restart Behavior**: Rate-limited (`429` + `Retry-After`) and oversized (`413`: `64 KiB` auth, `2 MiB` commands) responses fail closed with `Cache-Control: no-store`; independent deploys under `/elysianfields/` never overwrite sibling routes.
- **Accessibility and Input Behavior**: Strict CSP (`script-src 'self' 'wasm-unsafe-eval'; font-src 'self'; frame-ancestors 'none'`) permits bundled local fonts and assistive-technology bridges without inline script exceptions.
- **Executable Acceptance Scenario**: `SCN-M2-0010`.

### `EF-PLATFORM-0002` — Keyboard-only navigation, global privacy screen cover, single-owner accessibility, and focus restoration

- **ID**: `EF-PLATFORM-0002`
- **Product and Platforms**: `shared-platform` — `browser-wasm-desktop`, `desktop-linux`, `desktop-macos`, `desktop-windows`
- **Provenance**: `observed-and-required`; Authority levels 1 (browser accessibility stop-rule under Decision D2) and 2 (clinical privacy veil and keyboard accessibility). Normative details in [`04-subpath-csp-a11y-i18n.md`](./04-subpath-csp-a11y-i18n.md) §4.3.
- **Actors and Authority**: Authenticated clinician operating strictly by keyboard and/or screen reader.
- **Preconditions**: Clinician is signed in on `Today` with keyboard focus inside the multiline composer.
- **Actions**: Type `/` and `j` inside composer; trigger `Cmd+Shift+L` / `Ctrl+Shift+L` twice to toggle Privacy Screen Cover; blur composer, press `/` for Patient Finder, select patient with `ArrowDown`/`Enter`, open a citation chip via keyboard, and dismiss with `Escape`.
- **Visible Result**: Editing pauses single-key shortcuts (`/`, `j`, `k`, `?`) while `Cmd/Ctrl+Shift+L` always immediately hides all clinical content behind an opaque privacy veil and restores exact focus on uncover; every workflow control shows a `>= 2 px` high-contrast focus ring and completes without a mouse.
- **Persisted and External Effects**: Privacy cover toggles purely in client presentation state without mutating clinical data or dropping unsaved drafts.
- **Failure and Restart Behavior**: Modal focus traps never strand keyboard focus after dismissal or unexpected session expiry; dismissing overlays always returns focus to the triggering control (or main landmark if removed).
- **Accessibility and Input Behavior**: Single authoritative accessibility/input state owner with zero duplicate editable inputs or duplicate screen-reader nodes; polite/assertive live announcements for workspace state changes.
- **Executable Acceptance Scenario**: `SCN-M2-0011`.

### `EF-PLATFORM-0003` — Nine-language UI switching, Arabic RTL layout mirroring, bundled fonts, and verbatim source language

- **ID**: `EF-PLATFORM-0003`
- **Product and Platforms**: `shared-platform` — `browser-wasm-desktop`, `desktop-linux`, `android-native`, `ios-native`
- **Provenance**: `observed-and-required`; Authority levels 2 (clinical fidelity and multilingual access) and 3 (9-locale catalog and bundled SIL OFL font contract). Normative details in [`04-subpath-csp-a11y-i18n.md`](./04-subpath-csp-a11y-i18n.md) §4.4.
- **Actors and Authority**: Clinician viewing an authorized patient workspace containing English clinical records while switching UI locales.
- **Preconditions**: All 9 shipped locales (`en`, `es`, `fr`, `pt`, `de`, `zh-Hans`, `ja`, `ko`, `ar`) and bundled open-source fonts covering Latin, Arabic RTL, CJK, Hangul, and monospace are present.
- **Actions**: Cycle through all 9 locales via the language switcher and `?lang=` parameter; inspect Arabic (`ar`) RTL mirroring and mixed Arabic UI + LTR numeric lab values (`54 mL/min/1.73m²`); reload to verify persistence; open a cited clinical record under `ar`.
- **Visible Result**: All core UI keys translate cleanly with zero missing/tofu glyphs; `ar` mirrors horizontal layout chrome, caret direction, and accessibility direction (`rtl`) while preserving BiDi legibility for mixed numeric units; clinical source text remains verbatim in its original language accompanied by the localized original-language provenance notice.
- **Persisted and External Effects**: Locale selection resolves in priority order `?lang=` -> persisted `curie.locale.v1` -> OS/browser preferred languages -> `en`, and persists across reloads.
- **Failure and Restart Behavior**: Unknown or malformed `?lang=` tags fall back gracefully to persisted/OS/`en` locale without crashing or rendering untranslated placeholder keys.
- **Accessibility and Input Behavior**: Document and assistive-technology language/direction metadata updates synchronously with locale switches; Arabic/CJK/Hangul OS IME composition works cleanly in the composer and search inputs.
- **Executable Acceptance Scenario**: `SCN-M2-0012`.
