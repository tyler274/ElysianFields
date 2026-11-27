# Specification Package: M3 Suite Replacement (Increments 1–5)

- **Package**: `specs/packages/m3-suite`
- **Status**: Gatekeeper (Aponia)-approved for Builder (Vill-V) implementation
- **Domains**: `CLINIC`, `MOBILE`, `CONN`, `HARNESS`, `EXT`, `PUBLIC`, `PLATFORM`
- **Products**: `curie-clinic`, `curie-mobile`, `connected-care`, `shared-platform`, `browser-extension`, `public-web`
- **Requirements**: `EF-CLINIC-0010`–`EF-CLINIC-0015`, `EF-MOBILE-0001`–`EF-MOBILE-0005`, `EF-CONN-0001`–`EF-CONN-0002`, `EF-HARNESS-0002`–`EF-HARNESS-0004`, `EF-EXT-0001`, `EF-PUBLIC-0001`–`EF-PUBLIC-0002`, `EF-PLATFORM-0004` (20 requirements across Increments 1–5)
- **Acceptance Scenarios**: `SCN-M3-0001` through `SCN-M3-0020` in [`scenarios.toml`](./scenarios.toml)
- **Machine-Readable Contracts**: [`contracts/m3-boundaries-and-states.schema.json`](./contracts/m3-boundaries-and-states.schema.json)

## Package Chapters

| Increment | Chapter | Scope |
| --- | --- | --- |
| **Increment 1** | [`01-clinic-remaining.md`](./01-clinic-remaining.md) | Remaining Curie Clinic workflows: Today triage, message matching (14 formal invariants), reply drafting/withholding (12 formal invariants), longitudinal patient memory & rounds, multi-format EHR/channel setup (16 closed buckets, SMART on FHIR R4, KipuEMR + 42 CFR Part 2, C-CDA R2.1, HL7 v2.x, SHL, Gmail/Spruce) |
| **Increment 2** | [`02-curie-mobile.md`](./02-curie-mobile.md) | Curie Mobile standalone account & sign-in (6 formal invariants), portal & document ingestion with per-record savepoints, HealthKit/Health Connect acquisition & account-fenced outbox, push notifications, **For my doctor** sharing (13 formal invariants + Part 2 consent), portable export, account deletion & independent privacy receipt journal replay |
| **Increment 3** | [`03-connected-care.md`](./03-connected-care.md) | Connected care bridge: single-owner state machine, invitations, DOB verification & 5-attempt permanent lockout, Share consent (11 formal invariants), cross-product isolation, ending & rejoining connections |
| **Increments 4–5** | [`04-shared-execution-and-surfaces.md`](./04-shared-execution-and-surfaces.md) | Shared bounded execution harness & content-free OTLP telemetry, clinical library nightly pipeline (9 core guarantees) & PubMed rate-limited refresh, offline semantic encoder, synthetic 468-patient cohort bank, Chrome MV3 read-only EHR extension, semantic HTML public site + legal archives + isolated demo, and Dei SAMHSA recovery finder |
| **All Increments** | [`scenarios.toml`](./scenarios.toml) | 20 executable behavioral acceptance scenarios (`SCN-M3-0001`–`SCN-M3-0020`) |

## Cross-Cutting Architectural Invariants

- **Strict product isolation**: Curie Clinic and Curie Mobile keep separate databases, accounts, clinical types, service identities, secret scopes, and cache brokers. Data crosses only through Connected Care with verified consent.
- **Server headless rule**: Client rendering engines are never dependencies of server-side or shared execution crates.
- **Honest failure and privacy boundaries**: Partial failures never read as quiet or empty; 42 CFR Part 2 SUD records and SUD counseling notes remain strictly gated with verbatim § 2.32 notices; operational telemetry carries strictly allowlisted non-clinical attributes.

---

## Increment 1 — Remaining Curie Clinic Workflows

### `EF-CLINIC-0010` — Today queue urgency ordering, incomplete-source honesty, and channel health banners

- **ID**: `EF-CLINIC-0010`
- **Product and Platforms**: `curie-clinic` — `browser-wasm-desktop`, `desktop-linux`, `hosted-services`
- **Provenance**: `observed-and-required`; Authority levels 2 (clinical triage honesty) and 4 (behavioral baseline). Normative details in [`01-clinic-remaining.md`](./01-clinic-remaining.md) §1.1.
- **Actors and Authority**: Authenticated clinician with panel access; Today aggregator and channel health monitors.
- **Preconditions**: Practice has urgent (`now`) and same-day (`today`) items, scheduled visits, and a connected messaging channel.
- **Actions**: Load Today; mark item seen/done; inject temporary channel error (`temp_error`); inject revoked credentials (`revoked`); inject a partial producer outage.
- **Visible Result**: Urgent items surface ahead of `today` items; `temp_error` displays trouble notice without asking to reconnect (`TodaySaysSo`); `revoked` prompts reconnect; partial failure displays `failed_parts` instead of a quiet day.
- **Persisted and External Effects**: Seen/done standing persists against item `version_id` so newer clinical changes resurface the item; partial outage sets `may_be_missing_someone = true`.
- **Failure and Restart Behavior**: Upstream failures surface explicitly with retry rather than collapsing into an empty quiet state; channel status re-evaluates cleanly across reloads.
- **Accessibility and Input Behavior**: Keyboard-navigable priority list with landmarks and polite live announcements for channel health and incomplete-source warnings.
- **Executable Acceptance Scenario**: `SCN-M3-0001`.

### `EF-CLINIC-0011` — Message matching auto-link safety, 15-minute undecided fallback, and access-scoped suggestions

- **ID**: `EF-CLINIC-0011`
- **Product and Platforms**: `curie-clinic` — `browser-wasm-desktop`, `hosted-services`
- **Provenance**: `observed-and-required`; Authority levels 2 (access isolation and no dropped messages) and 3 (formal message-matching invariants). Normative details in [`01-clinic-remaining.md`](./01-clinic-remaining.md) §1.2.
- **Actors and Authority**: Clinician D1 (access to Patient A only), Clinician D2 (access to Patient B only), inbound message matcher.
- **Preconditions**: Two patients A and B exist in separate physician panels.
- **Actions**: Ingest unambiguous message for A; ingest ambiguous/conflicting message; inspect **Messages to match** as D1 when suggestion points to B; advance an `undecided` message past the 15-minute fallback.
- **Visible Result**: Unambiguous message auto-links to A (`LinksOnItsOwn`); ambiguous routes to `to_match`; D1 sees zero Patient B suggestions or linked conversations (`SuggestionsWithAccess`, `OnlyWithAccess`); undecided fallback surfaces in `to_match` (`NeverInvisible`).
- **Persisted and External Effects**: Conversation states (`none`, `undecided`, `to_match`, `filtered`, `linked`, `not_patient`) persist durably; read cursor advances strictly once per kept/unreadable message (`ExactlyOnce`).
- **Failure and Restart Behavior**: Matching worker restarts never lose or silently skip kept messages; undecodable messages surface in the unreadable queue (`CantReadShows`).
- **Accessibility and Input Behavior**: Keyboard-operable matching queue with accessible status labels and focus management when items leave the queue.
- **Executable Acceptance Scenario**: `SCN-M3-0002`.

### `EF-CLINIC-0012` — Message move with Undo restoration, deferred record update, and channel disconnect/backlog consent

- **ID**: `EF-CLINIC-0012`
- **Product and Platforms**: `curie-clinic` — `browser-wasm-desktop`, `hosted-services`
- **Provenance**: `observed-and-required`; Authority levels 2 (reversible moves, consent for backlog) and 3 (formal message-matching invariants). Normative details in [`01-clinic-remaining.md`](./01-clinic-remaining.md) §1.2.
- **Actors and Authority**: Clinician authorized for both Patient A and Patient B; peer clinicians; channel administration.
- **Preconditions**: Conversation linked to Patient A; active messaging channel connected.
- **Actions**: Move conversation A -> B, ingest a second message during Undo, press **Undo**; move A -> B again and let Undo expire; inject unreadable message, disconnect channel, reconnect with pre-connection backlog.
- **Visible Result**: **Undo** restores all kept messages (including mid-Undo arrivals) to A (`UndoPutsBack`) with zero deferred record updates (`ActsAfterUndo`); standing move after Undo expiry updates B and clears A (`RightPatient`); disconnect clears unlinked/unreadable while keeping linked (`ClearedOnDisconnect`); backlog waits for explicit consent (`BacklogOnlyWhenAccepted`).
- **Persisted and External Effects**: Moves enforce `OnlyBetweenTheirPatients` and close unauthorized peer views immediately; downstream clinical record mutations defer strictly until Undo expires and skip no-op moves back to origin.
- **Failure and Restart Behavior**: Server restart preserves active Undo origin and pending deferred-update timers without executing premature writes.
- **Accessibility and Input Behavior**: Undo banner exposes assertive/polite live announcement and keyboard-accessible **Undo** action with clear focus return.
- **Executable Acceptance Scenario**: `SCN-M3-0003`.

### `EF-CLINIC-0013` — Message reply drafting, per-physician draft isolation, auto-reply non-answering, and move withholding

- **ID**: `EF-CLINIC-0013`
- **Product and Platforms**: `curie-clinic` — `browser-wasm-desktop`, `hosted-services`
- **Provenance**: `observed-and-required`; Authority levels 2 (clinician-owned outbound replies, zero auto-send) and 3 (all 12 formal message-reply invariants). Normative details in [`01-clinic-remaining.md`](./01-clinic-remaining.md) §1.3.
- **Actors and Authority**: Clinicians D1 and D2 with access to Patient A; automated reply review worker.
- **Preconditions**: Linked conversation on Patient A has an open patient message, flagged clinical concern, and ready system reply draft.
- **Actions**: Record practice auto-responder message; D1 edits personal draft while D2 opens and copies reply; close reply and press **Undo**; move conversation A -> B while a stale review for A completes.
- **Visible Result**: Auto-responder never marks message answered (`OnlyTheReviewAnswers`); D1's draft stays isolated from D2 (`YourDraft`); **Copy reply** copies exact visible text with zero internal citation markup (`CopiesWhatYouSee`); closing keeps concern visible (`ConcernStays`) and **Undo** restores reply (`UndoBringsBack`); moving immediately withholds prior ready reply (`WithheldAfterMove`).
- **Persisted and External Effects**: At most one active review runs per conversation (`OneReviewEach`); sent practice history records strictly actual sent messages (`HistoryTrue`); channel-disconnected notice shows iff disconnected (`NoteWhileDisconnected`).
- **Failure and Restart Behavior**: Worker restarts return interrupted reviews to `woken` without duplicating messages; stale reviews for prior patients are discarded on write.
- **Accessibility and Input Behavior**: Keyboard-accessible draft editor, **Copy reply**, **Restore system reply**, and **Undo** controls with clear screen-reader confirmation announcements.
- **Executable Acceptance Scenario**: `SCN-M3-0004`.

### `EF-CLINIC-0014` — Patient memory rounds watermark, quiet no-op, disagreement preservation, and history-preserving retirement

- **ID**: `EF-CLINIC-0014`
- **Product and Platforms**: `curie-clinic` — `hosted-services`
- **Provenance**: `observed-and-required`; Authority levels 2 (clinical provenance and non-destructive history) and 4 (behavioral baseline). Normative details in [`01-clinic-remaining.md`](./01-clinic-remaining.md) §1.4.
- **Actors and Authority**: Background patient-memory rounds job and clinician authority records.
- **Preconditions**: Patient workspace holds dated memory items and an active physician instruction.
- **Actions**: Run rounds with zero new records; inject disagreeing external chart record and run rounds; abort a rounds run mid-execution; supersede an active clinical fact with a dated resolution.
- **Visible Result**: Quiet run performs zero edits; disagreement retains both dated chart record and physician instruction; superseded fact moves to retired history with resolution date and reason without altering unrelated entries.
- **Persisted and External Effects**: Rounds watermark advances monotonically solely upon clean completion; rendered model-facing prompts carry zero tracker IDs or unknown tool fields.
- **Failure and Restart Behavior**: Aborted or failed rounds runs never advance the watermark past unreviewed records.
- **Accessibility and Input Behavior**: N/A (headless background service; client presentation renders dated active and retired provenance clearly).
- **Executable Acceptance Scenario**: `SCN-M3-0005`.

### `EF-CLINIC-0015` — Multi-format EHR record mapping (FHIR R4, Kipu/Part 2, C-CDA, HL7 v2) with native byte retention and binding checks

- **ID**: `EF-CLINIC-0015`
- **Product and Platforms**: `curie-clinic` — `hosted-services`
- **Provenance**: `observed-and-required`; Authority levels 2 (42 CFR Part 2 statutory compliance and lossless clinical provenance) and 3 (closed 16-bucket mapping contract). Normative details in [`01-clinic-remaining.md`](./01-clinic-remaining.md) §1.5 and [`contracts/m3-boundaries-and-states.schema.json`](./contracts/m3-boundaries-and-states.schema.json).
- **Actors and Authority**: Practice EHR ingestion pipeline and source adapters (SMART on FHIR R4, KipuEMR v3/v4, C-CDA R2.1, HL7 v2.x, `shlink:/`, Gmail/Spruce).
- **Preconditions**: Synthetic multi-format batches including Kipu 42 CFR Part 2 SUD + counseling notes, C-CDA with one unreadable `nonXMLBody`, HL7 v2.x messages, and one record with a mismatched patient binding.
- **Actions**: Ingest each batch; attempt mismatched-binding ingestion; inspect native byte digests, 16-bucket classification, `FailedPart` entries, and Part 2 notices.
- **Visible Result**: Valid records classify into the closed 16 buckets with untouched native source bytes and SHA-256 digests; unreadable attachments emit `FailedPart` without dropping valid siblings; Part 2 records carry verbatim 42 CFR § 2.32(a) notices.
- **Persisted and External Effects**: Mismatched patient-binding records (`subject` / `casefile_id` / `recordTarget` / `PID-3`) are rejected before storage; Part 2 SUD and counseling notes are gated from general exports without verified written consent.
- **Failure and Restart Behavior**: Partial failures isolate cleanly per record/segment; EHRs lacking live FHIR surface explicit assisted/export onboarding rather than fake live checks.
- **Accessibility and Input Behavior**: N/A (headless ingestion pipeline; client displays `FailedPart` and Part 2 notices clearly).
- **Executable Acceptance Scenario**: `SCN-M3-0006`.

---

## Increment 2 — Curie Mobile Patient Application and Backend

### `EF-MOBILE-0001` — Curie Mobile sign-in, offline owed sign-out flush, and device handoff isolation

- **ID**: `EF-MOBILE-0001`
- **Product and Platforms**: `curie-mobile` — `browser-wasm-mobile`, `android-native`, `ios-native`
- **Provenance**: `observed-and-required`; Authority levels 2 (standalone patient identity and shared-device privacy) and 3 (all 6 formal mobile sign-in invariants). Normative details in [`02-curie-mobile.md`](./02-curie-mobile.md) §2.1.
- **Actors and Authority**: Patient Account A and Patient Account B sharing a mobile device; Curie Mobile auth service.
- **Preconditions**: Account A is signed in with an unsaved local note draft.
- **Actions**: Take device offline and sign out; verify sign-in screen holds nothing; restore network to flush owed sign-out; sign in as Account B.
- **Visible Result**: Sign-out immediately transitions to `Enter your email` with zero credentials or Account A drafts (`SignInHoldsNothing`); Account B starts completely clean (`DraftMatchesAccount`).
- **Persisted and External Effects**: Durable `owes` queue flushes on reconnect (`OwedSessionsEventuallyEnd`, `ServerEndsSignOut`) and invalidates Account A's session server-side; standalone account never couples to clinic accounts.
- **Failure and Restart Behavior**: App restart or process kill while offline preserves the owed sign-out queue until server confirmation (`NoDeadlock`).
- **Accessibility and Input Behavior**: Full TalkBack, VoiceOver, and mobile web screen-reader support with keyboard/IME/autofill compatibility on all four sign-in screens.
- **Executable Acceptance Scenario**: `SCN-M3-0007`.

### `EF-MOBILE-0002` — Curie Mobile portal/SHL ingestion with per-record savepoints and 42 CFR Part 2 boundaries

- **ID**: `EF-MOBILE-0002`
- **Product and Platforms**: `curie-mobile` — `browser-wasm-mobile`, `android-native`, `ios-native`, `hosted-services`
- **Provenance**: `observed-and-required`; Authority levels 2 (patient-owned record integrity and Part 2 boundaries) and 4 (behavioral baseline). Normative details in [`02-curie-mobile.md`](./02-curie-mobile.md) §2.2.
- **Actors and Authority**: Signed-in Curie Mobile patient; canonical portal/document ingestion service.
- **Preconditions**: Synthetic SMART on FHIR R4 portal and `shlink:/` payload containing one malformed sibling resource and one 42 CFR Part 2 record.
- **Actions**: Complete PKCE `S256` portal connection and ingest mixed batch; attempt insecure `http://` or invalid-key `shlink:/` URI; disconnect portal.
- **Visible Result**: Valid records commit and appear with original dates/provenance while malformed sibling surfaces an honest notice; insecure `shlink:/` is rejected before fetch; disconnecting portal preserves previously imported records.
- **Persisted and External Effects**: Per-record savepoints isolate failures; Part 2 records carry verbatim § 2.32 notices and remain excluded from general connected-care sharing without written consent; per-vendor kill-switches halt new syncs without breaking existing records.
- **Failure and Restart Behavior**: Interrupted syncs resume cleanly without duplicating committed records; daily background refreshes operate independently per connected portal.
- **Accessibility and Input Behavior**: Accessible portal search, clear error alerts, and readable provenance notices across mobile web, Android, and iOS.
- **Executable Acceptance Scenario**: `SCN-M3-0008`.

### `EF-MOBILE-0003` — On-device HealthKit / Health Connect acquisition with account-fenced transactional outbox

- **ID**: `EF-MOBILE-0003`
- **Product and Platforms**: `curie-mobile` — `android-native`, `ios-native`
- **Provenance**: `observed-and-required`; Authority levels 2 (account isolation and durable health sync) and 3 (platform adapter boundary). Normative details in [`02-curie-mobile.md`](./02-curie-mobile.md) §2.3 and §2.4.
- **Actors and Authority**: Patient Account A and Account B on mobile device; native HealthKit / Health Connect adapters; Curie Mobile client/API.
- **Preconditions**: Account A grants explicit granular health read permissions; device is temporarily offline.
- **Actions**: Acquire synthetic steps, heart-rate, and blood-pressure samples offline into the transactional outbox; sign out of Account A and sign in as Account B before network returns; restore network; sign back in as Account A.
- **Visible Result**: Account B sees zero Account A samples; UI reads synced health trends exclusively via the typed client layer (never calling native health bridges directly).
- **Persisted and External Effects**: Outbox rows fence strictly to `(account_id, account_incarnation)`; Account A uploads idempotently with zero dropped or duplicated samples; push tokens revoke on sign-out or privacy withdrawal.
- **Failure and Restart Behavior**: OS backgrounding, pause/resume, and process recreation preserve committed outbox entries and unsaved notes without cross-account leakage.
- **Accessibility and Input Behavior**: Native permission rationale flows and synced trend views expose full VoiceOver/TalkBack semantics and handle soft-keyboard resize cleanly.
- **Executable Acceptance Scenario**: `SCN-M3-0009`.

### `EF-MOBILE-0004` — For my doctor curated sharing: immutable sends, idempotency, source removal, and Part 2 consent

- **ID**: `EF-MOBILE-0004`
- **Product and Platforms**: `curie-mobile` — `browser-wasm-mobile`, `android-native`, `ios-native`, `hosted-services`
- **Provenance**: `observed-and-required`; Authority levels 2 (patient-controlled sharing and Part 2 written consent) and 3 (all 13 formal `For my doctor` invariants). Normative details in [`02-curie-mobile.md`](./02-curie-mobile.md) §2.5.
- **Actors and Authority**: Connected patient curating `To send` records/notes; connected practice receiver.
- **Preconditions**: Patient connected to Practice 1 with marked Record R1, Note N1 linked to R2, and Part 2 Record R3.
- **Actions**: Verify pre-confirm disclosure; confirm send R1+N1 across an uncertain timeout + retry; mutate R1 upstream; remove newly marked R4 upstream; attempt and then explicitly consent to sending Part 2 R3.
- **Visible Result**: Disclosure states send is irreversible and not for urgent care (`DisclosureBeforeSend`); nothing transfers before confirm (`NothingReachesWithoutConfirm`); R4 removal clears its mark with a plain notice (`SourceRemovalClearsMark`); status moves `Sent` -> `Received by the care team` honestly (`SentAndReceivedTruth`).
- **Persisted and External Effects**: Confirmed send freezes exact versions (`SendHoldsExactImmutableContent`), delivers at most once (`SendReachesAtMostOnce`), clears carried items only (`ConfirmedSendClearsCarriedOnly`), and enforces explicit written consent + verbatim § 2.32 notice for Part 2 records.
- **Failure and Restart Behavior**: Uncertain sends reuse the same idempotent key; ending connection halts uncertain retries cleanly while preserving unsent marks (`ConnectionEndStopsUncertain`).
- **Accessibility and Input Behavior**: Accessible checkboxes/marks, clear confirmation modal with focus trapping and restoration, and polite status announcements for save/send outcomes.
- **Executable Acceptance Scenario**: `SCN-M3-0010`.

### `EF-MOBILE-0005` — Curie Mobile account deletion and isolated DB restore privacy receipt journal replay

- **ID**: `EF-MOBILE-0005`
- **Product and Platforms**: `curie-mobile` — `hosted-services`, `android-native`, `ios-native`
- **Provenance**: `observed-and-required`; Authority levels 2 (irreversible account deletion and non-resurrection privacy guarantee) and 3 (independent create-only privacy receipt journal contract). Normative details in [`02-curie-mobile.md`](./02-curie-mobile.md) §2.6 and [`contracts/m3-boundaries-and-states.schema.json`](./contracts/m3-boundaries-and-states.schema.json).
- **Actors and Authority**: Patient requesting account deletion; independent create-only journal; privacy delivery and restore-replay jobs; unprivileged runtime API.
- **Preconditions**: Account has records, a confirmed send, and an active connected-care link; pre-deletion PostgreSQL backup exists.
- **Actions**: Delete account (writing immutable `account_delete` receipt); restore pre-deletion backup into an isolated DB; execute mandatory privacy receipt journal replay gate before opening traffic.
- **Visible Result**: Deleted account cannot sign in or resurrect after restore; portable export / PDF summary remains available prior to deletion.
- **Persisted and External Effects**: Independent journal stores metadata-only receipts (13 restriction ops, zero PHI/prose/credentials); replay enforces `account_incarnation` fencing, idempotently terminates connected-care links, and scrubs staging transfer records; runtime API runs unprivileged with zero DDL authority.
- **Failure and Restart Behavior**: Delivery and replay jobs fail closed (non-zero exit) if any receipt fails, budget exhausts, pending items remain, or journal is unavailable; `/readyz` checks migration state before serving.
- **Accessibility and Input Behavior**: Account deletion confirmation clearly communicates permanence with accessible confirmation controls and focus safety.
- **Executable Acceptance Scenario**: `SCN-M3-0011`.

---

## Increment 3 — Connected Care Bridge and Product Isolation

### `EF-CONN-0001` — Practice invitation lifecycle: single open invite, 5-wrong-DOB lockout, resend, cancel, and Share consent

- **ID**: `EF-CONN-0001`
- **Product and Platforms**: `connected-care` — `browser-wasm-desktop`, `browser-wasm-mobile`, `android-native`, `ios-native`, `hosted-services`
- **Provenance**: `observed-and-required`; Authority levels 2 (explicit Share consent and identity verification) and 3 (all 11 formal `Invitation` invariants). Normative details in [`03-connected-care.md`](./03-connected-care.md) §§3.1–3.2 and [`contracts/m3-boundaries-and-states.schema.json`](./contracts/m3-boundaries-and-states.schema.json).
- **Actors and Authority**: Authorized practice member in Curie Clinic; standalone signed-in patient in Curie Mobile; single-owner Connected Care bridge service.
- **Preconditions**: Practice 1 invites Patient P who owns a standalone Curie Mobile account.
- **Actions**: Issue and resend open invite; submit 4 wrong DOB guesses then a 5th wrong DOB; verify resend cannot unlock; cancel locked invite and issue a fresh invite; enter matching DOB, leave preview without tapping Share, then return and tap **Share**.
- **Visible Result**: Zero practice/physician preview before DOB match (`PreviewOnlyAfterDob`); 5th wrong DOB locks permanently (`LockedAfterFiveWrongDob`); leaving without tapping Share grants zero consent (`OnlyShareGrantsConsent`); tapping Share connects the existing standalone account (`ShareConnectsSignedInAccount`, `SameConnectionState`).
- **Persisted and External Effects**: Single authoritative bridge owns `Not invited` / `Invited` / `Connected` / `Ended`; at most one open invite per pair (`AtMostOneOpen`); cancelling invalidates every prior link/code (`CancelledNeverConnects`); zero cross-product clinical tables shared.
- **Failure and Restart Behavior**: Failed outbound mail leaves invite open (`FailedMailLeavesOpen`); reloads agree across both products without drift.
- **Accessibility and Input Behavior**: Accessible DOB input, clear non-leaking lockout messaging, keyboard-operable preview and **Share** / **Decline** controls across web and mobile.
- **Executable Acceptance Scenario**: `SCN-M3-0012`.

### `EF-CONN-0002` — Multi-practice independence, ending connection during uncertain send, and rejoining

- **ID**: `EF-CONN-0002`
- **Product and Platforms**: `connected-care` — `browser-wasm-mobile`, `browser-wasm-desktop`, `hosted-services`
- **Provenance**: `observed-and-required`; Authority levels 2 (patient autonomy and multi-practice isolation) and 3 (connected-care and `For my doctor` formal invariants). Normative details in [`03-connected-care.md`](./03-connected-care.md) §3.3.
- **Actors and Authority**: Patient connected to Practice 1 and Practice 2; practice members in Practice 1.
- **Preconditions**: Patient P is `Connected` to both Practice 1 and Practice 2 with an uncertain send pending to Practice 1.
- **Actions**: End connection with Practice 1; inspect Practice 2 connection and Patient P's standalone account/history; re-invite from Practice 1 and complete DOB + Share to rejoin.
- **Visible Result**: Ending Practice 1 halts uncertain send and removes **Try again** while preserving unsent marks (`ConnectionEndStopsUncertain`) and prior `Sent` history; Practice 2 stays `Connected` (`PracticesStayIndependent`); rejoining requires fresh explicit Share consent.
- **Persisted and External Effects**: Ending stops all future transfers immediately; historical `To send` items never auto-transfer upon rejoining; account deletion idempotently terminates all links.
- **Failure and Restart Behavior**: Concurrent disconnect by patient and practice resolves idempotently to `Ended`.
- **Accessibility and Input Behavior**: Clear accessible confirmation and status announcements for ending and rejoining practice connections.
- **Executable Acceptance Scenario**: `SCN-M3-0013`.

---

## Increment 4 — Shared Execution, Clinical Library, and Synthetic Patient Bank

### `EF-HARNESS-0002` — Shared harness tool validation, frozen `skill_view`, budget `CutOff`, and content-free telemetry

- **ID**: `EF-HARNESS-0002`
- **Product and Platforms**: `shared-platform` — `hosted-services`
- **Provenance**: `observed-and-required`; Authority levels 2 (HIPAA-safe content-free operational telemetry and bounded execution) and 3 (harness ownership split). Normative details in [`04-shared-execution-and-surfaces.md`](./04-shared-execution-and-surfaces.md) §4.1.
- **Actors and Authority**: Product-neutral bounded execution harness, deterministic provider stub, operational OTLP telemetry collector.
- **Preconditions**: Harness configured with frozen in-memory SHA-256-verified skill package, typed JSON Schema tools, and ops telemetry pipeline.
- **Actions**: Invoke `skill_view` on a valid skill and on a path-traversal name; invoke a tool with malformed JSON arguments; exceed configured turn/tool budget to trigger `CutOff`; inspect exported ops spans and logs.
- **Visible Result**: Valid `skill_view` reads solely from frozen memory; invalid paths and malformed tool arguments return structured typed errors; budget cap settles cleanly to `CutOff` preserving kept work.
- **Persisted and External Effects**: Operational traces/logs emit strictly allowlisted attributes with `status.message` cleared and log bodies rewritten to `"operational.event"` — zero prompts, completions, or clinical text; non-blocking telemetry queue (`queue_size = 1000`, `block_on_overflow = false`).
- **Failure and Restart Behavior**: Crash/restart reconstructs exact working state strictly from the consuming product's durable records; telemetry outages never stall clinical requests.
- **Accessibility and Input Behavior**: N/A (headless shared execution library).
- **Executable Acceptance Scenario**: `SCN-M3-0014`.

### `EF-HARNESS-0003` — Clinical library nightly stop rules, lost-slot isolation, single active version, and PubMed pacing

- **ID**: `EF-HARNESS-0003`
- **Product and Platforms**: `shared-platform` — `hosted-services`
- **Provenance**: `observed-and-required`; Authority levels 2 (de-identified clinical literature safety) and 3 (all 9 clinical library nightly guarantees + NCBI pacing contract). Normative details in [`04-shared-execution-and-surfaces.md`](./04-shared-execution-and-surfaces.md) §4.2.
- **Actors and Authority**: Nightly clinical literature worker, PubMed status refresher, operator resolution controls, PostgreSQL cluster.
- **Preconditions**: Disposable PostgreSQL with clinical library schema and deterministic local NCBI/PubMed stub.
- **Actions**: Trigger identical failure code across 2 nights; trigger 3 consecutive failed/idle nights; simulate lost slot lease mid-run and attempt candidate write/DB mutation/activation; run concurrent PubMed status refreshes.
- **Visible Result**: Automatic retries halt on 2nd identical failure code and on 3rd consecutive failed/idle night until explicit operator resolution; lost-slot run writes zero DB rows, zero active versions, and zero candidate overwrites; concurrent PubMed starts pace `>= 100 ms` apart.
- **Persisted and External Effects**: At most one checked candidate version is active (non-copyable handle + partial unique index); queries carry strictly de-identified clinical concepts; PubMed records lacking publication types classify as unreadable (never empty).
- **Failure and Restart Behavior**: Slot takeover occurs solely after confirming prior holder ended; 2nd consecutive unreadable holder alerts operator; auth refusal fails the night immediately without retrying or leaking keys.
- **Accessibility and Input Behavior**: N/A (headless background pipeline).
- **Executable Acceptance Scenario**: `SCN-M3-0015`.

### `EF-HARNESS-0004` — Synthetic patient bank deterministic build, FHIR R4 validation, and authoring/runtime isolation

- **ID**: `EF-HARNESS-0004`
- **Product and Platforms**: `shared-platform` — `hosted-services`
- **Provenance**: `observed-and-required`; Authority levels 2 (zero PHI synthetic testing and runtime separation) and 3 (deterministic cohort contract). Normative details in [`04-shared-execution-and-surfaces.md`](./04-shared-execution-and-surfaces.md) §4.2.
- **Actors and Authority**: Synthetic cohort generator and physician runtime chart loader.
- **Preconditions**: Clean-room generator configured for 468 primary-care synthetic charts (60 light, 122 moderate, 286 complex; 5 flagship units) spanning 11 FHIR R4 resource families + future `Appointment` calendar.
- **Actions**: Build FHIR R4 bank twice into separate directories and diff digests; validate physiological consistency, pending post-`as_of` withholding, and isolated corrupted PDF pages; load bank twice idempotently into runtime DB.
- **Visible Result**: Builds are byte-deterministic with zero real PHI and zero repeated `>= 40`-char narrative boilerplate; second runtime import reports 0 changed records and preserves panel assignments.
- **Persisted and External Effects**: Runtime servers and container images consume solely rendered FHIR bundles (`patients/`, `calendar/`, `manifest.json`), never authoring ledgers; offline semantic encoder computes chart embeddings locally without external API calls.
- **Failure and Restart Behavior**: Partial imports remain transactional and idempotent; corrupted single PDF attachment pages fail in isolation without hiding valid sibling pages.
- **Accessibility and Input Behavior**: N/A (offline build and headless ingestion tool).
- **Executable Acceptance Scenario**: `SCN-M3-0016`.

---

## Increment 5 — Extension, Public Site, Dei Recovery Finder, and Workspace Boundaries

### `EF-EXT-0001` — Chrome MV3 extension pairing, memory-only credentials, declared named reads, and billing redaction

- **ID**: `EF-EXT-0001`
- **Product and Platforms**: `browser-extension` — `chrome-extension-popup`
- **Provenance**: `observed-and-required`; Authority levels 2 (least-privilege permissions, memory-only tokens, billing redaction) and 3 (M1 MV3 popup CSP & size/cold-start budget; D2 HTML fallback if exceeded). Normative details in [`04-shared-execution-and-surfaces.md`](./04-shared-execution-and-surfaces.md) §4.3.
- **Actors and Authority**: Clinician browser running Chrome MV3 extension (ordinary JS/TS worker and content readers + Wasm or approved HTML popup) and local pairing bridge.
- **Preconditions**: MV3 extension loaded in Chromium with local pairing sandbox and synthetic EHR response containing billing/medication price fields.
- **Actions**: Inspect manifest and core connector files; pair extension; lease declared named read job and reject raw-URL smuggling job; inspect `chrome.storage.local` and `chrome.storage.sync`; verify delivered payload and SHA-256 receipt.
- **Visible Result**: Popup opens `<= 250 ms` and `<= 1.0 MiB` Brotli-11 (or approved D2 narrow HTML adapter) with clear pairing/read status; zero CSP violations under `script-src 'self' 'wasm-unsafe-eval'; object-src 'self'`.
- **Persisted and External Effects**: Manifest uses `optional_host_permissions` only; core worker/connector/popup carry zero EHR vendor/patient/chart names; bearer/refresh tokens reside exclusively in memory-only `chrome.storage.session`; billing/price/invoice fields strip prior to SHA-256 receipt computation and durable outbox delivery.
- **Failure and Restart Behavior**: Raw-URL jobs fail closed; worker suspension clears expired tokens cleanly without leaking credentials to disk.
- **Accessibility and Input Behavior**: Keyboard-operable popup controls with visible focus rings and screen-reader status announcements.
- **Executable Acceptance Scenario**: `SCN-M3-0017`.

### `EF-PUBLIC-0001` — Public site semantic HTML, verbatim legal archives, and isolated interactive physician demo

- **ID**: `EF-PUBLIC-0001`
- **Product and Platforms**: `public-web` — `static-public-web`, `hosted-services`
- **Provenance**: `observed-and-required`; Authority levels 1 (semantic indexable HTML constraint, zero canvas on legal docs) and 2 (verbatim legal archive fidelity and demo isolation). Normative details in [`04-shared-execution-and-surfaces.md`](./04-shared-execution-and-surfaces.md) §4.4.
- **Actors and Authority**: Public visitor, search crawler, zero-JS user agent, and public web server.
- **Preconditions**: Replacement public site running with zero production clinical DB credentials or secrets.
- **Actions**: Fetch `/`, `/mobile`, `/privacy`, `/terms`, `/clinical-transparency`, `/mobile/privacy`, archived dated legal routes, `/invite`, `/robots.txt`, `/sitemap.xml` with JS disabled; exercise interactive synthetic demo up to its turn budget; probe `/grafana/` in production config.
- **Visible Result**: Every marketing and legal page renders complete semantic, indexable HTML without JS or canvas rendering; `/invite` emits `noindex, nofollow`; legal pages match approved JSON archives verbatim; `/grafana/` returns `404`/`403` in production.
- **Persisted and External Effects**: Interactive demo runs strictly on isolated per-visitor in-memory synthetic charts with verbatim citations and enforces per-visitor turn budgets; public ingress enforces rate limiting, body caps (`64 KiB`), and `no-store` on errors/probes.
- **Failure and Restart Behavior**: Demo turn exhaustion surfaces clear completion notice; server restarts drop ephemeral demo sessions cleanly without touching any clinical persistence.
- **Accessibility and Input Behavior**: Valid semantic HTML landmarks, heading hierarchy, keyboard navigation, and zero-JS readability across all public and legal routes.
- **Executable Acceptance Scenario**: `SCN-M3-0018`.

### `EF-PUBLIC-0002` — Dei recovery finder: crisis lines first, zero-JS fallback, versioned shard cache, and offline reload

- **ID**: `EF-PUBLIC-0002`
- **Product and Platforms**: `public-web` — `static-public-web`, `browser-wasm-desktop`, `browser-wasm-mobile`
- **Provenance**: `observed-and-required`; Authority levels 2 (crisis-line safety ordering and honest error fallback) and 3 (versioned Cache Storage shard state machine). Normative details in [`04-shared-execution-and-surfaces.md`](./04-shared-execution-and-surfaces.md) §4.4 and [`contracts/m3-boundaries-and-states.schema.json`](./contracts/m3-boundaries-and-states.schema.json).
- **Actors and Authority**: Public visitor seeking substance-use or mental-health recovery resources (online, offline, or without JS).
- **Preconditions**: Static Dei site served locally with SHA-256-versioned SAMHSA state JSON shards.
- **Actions**: Load state page with JS disabled; load with JS enabled, cache shard, and filter; stop server, reload offline, and filter; restart server with updated shard version and reload; simulate fetch failure with no cached shard.
- **Visible Result**: Crisis lines (**988**, **911**, **SAMHSA National Helpline**) always render **before** any facility listing; zero-JS page shows initial facilities and city directory; offline reload filters cached shard seamlessly; fetch failure shows error alert + **Try again** over prerendered listing and never displays zero results.
- **Persisted and External Effects**: Cache Storage caches SHA-256-versioned shards (`Loading`, `Fresh`, `Stale`, `Failed`), prunes superseded shard versions on upgrade, and service worker retains matching JS+Wasm pairs for cached pages.
- **Failure and Restart Behavior**: Stale cached shards remain usable under a clear warning while fresh shards download; offline recovery never breaks JS/Wasm asset pairing.
- **Accessibility and Input Behavior**: Crisis phone links and filter controls are keyboard-operable and screen-reader accessible with live status announcements for `Loading`, `Stale`, and `Failed`.
- **Executable Acceptance Scenario**: `SCN-M3-0019`.

### `EF-PLATFORM-0004` — Cross-product architectural boundary gate across all replacement crates

- **ID**: `EF-PLATFORM-0004`
- **Product and Platforms**: `shared-platform` — `hosted-services`
- **Provenance**: `observed-and-required`; Authority levels 1 (product boundary and headless server mandates) and 2 (compile-time isolation guarantee). Normative details in [`03-connected-care.md`](./03-connected-care.md) §3.1 and [`04-shared-execution-and-surfaces.md`](./04-shared-execution-and-surfaces.md) §4.1.
- **Actors and Authority**: Automated workspace boundary verifier running in CI and local checks.
- **Preconditions**: Elysian Realm workspace contains replacement Curie Clinic, Curie Mobile, Connected Care bridge, and shared crates.
- **Actions**: Run automated boundary check across all workspace manifests and sources; test negative injection of a cross-product or server-rendering dependency.
- **Visible Result**: Clean workspace passes with zero boundary violations; any injected violation fails closed with explicit crate/edge diagnostics.
- **Persisted and External Effects**: Enforces permanently that server crates never depend on client rendering engines, Curie Clinic and Curie Mobile never import each other's clinical crates, shared harness/channels/clinical library import zero product or forbidden DB crates, and mobile UI never calls native health bridges directly.
- **Failure and Restart Behavior**: Missing or unparseable workspace manifests fail closed (exit non-zero).
- **Accessibility and Input Behavior**: N/A (compile-time and CI architectural gate).
- **Executable Acceptance Scenario**: `SCN-M3-0020`.
