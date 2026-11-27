# 2. Curie Mobile Patient Application & Backend (M3 Increment 2)

Curie Mobile is the standalone patient-owned personal health record application (Android native, iOS native, and browser web) with its own identity, clinical evidence store, assistant workspace, and database completely isolated from Curie Clinic.

## 2.1 Standalone Account Authority & Sign-In (6 Formal Invariants)

- **Standalone ownership:** A patient's Curie Mobile account belongs solely to the patient. Joining, declining, or leaving a medical practice never creates a second account and never deletes the patient's personal health history.
- **Four client screens:** `Enter your email`, `Check your email`, `Signed in`, and `Session ended`.
- **Verified sign-in & handoff invariants:**
  1. **`SignInHoldsNothing`:** Whenever displaying `Enter your email`, `Check your email`, or `Session ended`, the device holds zero active session credentials and zero prior account drafts or records.
  2. **`DraftMatchesAccount`:** Any local unsaved draft belongs strictly to the currently signed-in account.
  3. **`SignedInHoldsOwnSession`:** A signed-in screen always holds a session belonging to that account (live or awaiting expiry discovery).
  4. **`ServerEndsSignOut`:** Signing out—whether online immediately or offline via a durable `owes` queue—ends that session on the server; an owed sign-out is never dropped while the session remains live.
  5. **`OwedSessionsEventuallyEnd`:** Once network connectivity returns, every queued offline sign-out flushes to the server and invalidates the session.
  6. **`NoDeadlock`:** Every reachable combination of online/offline sign-out, expiry, device handoff to another person, and returning sign-in has a clean continuation.

## 2.2 Health Records, Portals & Evidence Ingestion

- **Patient portal connections (`"Doctors & hospitals"`):** Search and connect SMART on FHIR R4 portals (Epic/MyChart, Oracle Health, athenahealth, eClinicalWorks/healow, MEDITECH Expanse, NextGen, Veradigm, VA Health, CMS Medicare Blue Button 2.0, and custom SMART on FHIR R4 endpoints) via single-use OAuth2 + PKCE `S256` and vendor-appropriate SMART v1/v2 scopes.
- **Direct document & link imports:** Ingest SMART Health Links (`shlink:/`, strict HTTPS public destination, exact 32-byte key, flags, expiry), C-CDA R2.1 XML documents, and Kipu Health behavioral-health exports.
- **Ingestion durability & isolation:**
  - All connectors publish through one canonical connector authority with **per-record savepoint isolation** (one unreadable record fails cleanly while valid siblings commit).
  - Preserves original source dates, native bytes, and provenance; refreshes connected portals independently on a daily cadence; honors per-vendor operator kill-switches without breaking existing connections; preserves imported history when a portal is disconnected.
- **42 CFR Part 2 boundaries:** Imported behavioral-health/SUD records retain Part 2 confidentiality provenance, verbatim 42 CFR § 2.32(a) prohibition-on-redisclosure notices, and separation of SUD counseling notes (42 CFR § 2.31(b)). Part 2 records lacking explicit written consent—and all SUD counseling notes—are excluded from general connected-care sharing.
- **Patient assistant workspace:** Runs on the shared bounded execution harness scoped strictly to the patient's own records; helps explain records and trends but never acts as a physician, places orders, or messages a clinic directly.

## 2.3 On-Device Health Acquisition & Upload (HealthKit / Health Connect)

- **Native platform adapters:**
  - **iOS:** HealthKit read/background-delivery bridge (entitlements `com.apple.developer.healthkit`, `com.apple.developer.healthkit.background-delivery`) + Keychain secure retention.
  - **Android:** Health Connect bridge requesting explicit granular health read permissions (steps, active calories, exercise, distance, nutrition, hydration, resting heart rate, weight, sleep, blood pressure, body fat, lean body mass, height, blood glucose, body temperature, basal body temperature, oxygen saturation, VO2 max) + permission rationale intent + Keystore secure retention.
- **Account-fenced transactional outbox:** Acquired samples persist durably on device fenced to `(account_id, account_incarnation)` and upload in idempotent batches through the client transport layer. Switching accounts or signing out never leaks samples across accounts and never silently drops committed samples.
- **UI isolation boundary:** Patient UI screens read synced health trends/records strictly through the typed client layer, never invoking native HealthKit/Health Connect bridges directly.

## 2.4 Push Notifications & Native Lifecycle

- **Registration & permissions:** Manages iOS APNs and Android FCM (`POST_NOTIFICATIONS`) registration tokens, refreshes rotated tokens, and revokes device tokens on sign-out or privacy withdrawal (`device_withdraw` / `proactive_withdraw`).
- **Mobile lifecycle resilience:** Survives OS backgrounding, pause/resume, process recreation, deep links (e.g., `/invite`), and soft-keyboard viewport resizing without losing unsaved local notes or leaking prior-account memory.

## 2.5 "For My Doctor" Curated Sharing (13 Formal Invariants)

Patients curate records and personal notes in **For my doctor** (`To send`) and explicitly confirm immutable sends to a connected practice:

1. **`NothingReachesWithoutConfirm`:** Nothing in `For my doctor` leaves Curie Mobile unless the patient explicitly confirms a send.
2. **`SendToOneConnectedPractice`:** Each confirmed send targets exactly one practice currently in `Connected` state at save time.
3. **`SendHoldsExactImmutableContent`:** Confirming a send freezes the chosen records (and every record linked to a chosen note) at their exact versions; later source edits or removals never mutate earlier sends.
4. **`SendReachesAtMostOnce`:** Retrying an uncertain send reuses the same idempotent send key so the practice receives each send at most once.
5. **`ConfirmedSendClearsCarriedOnly`:** Confirming a send as `Sent` clears only the carried marks and notes from `To send`, leaving unchosen items intact; failed or unconfirmed sends clear nothing.
6. **`ToSendExact`:** `To send` contains a record strictly while marked and present, and a note strictly until sent or deleted.
7. **`SentAndReceivedTruth`:** UI displays **Sent** only after server persistence succeeds and **Received by the care team** only after practice pickup is confirmed; if the connection ends before pickup, the send remains **Sent** in the patient's history.
8. **`DisclosureBeforeSend`:** Before confirmation, UI clearly discloses that the send cannot be taken back and is not for urgent medical needs or questions requiring an answer.
9. **`SaveHonesty`:** Marks and notes remain visibly unsaved until persisted; edits from another device never silently overwrite unsaved local typing.
10. **`SourceCorrectionSafe`:** Upstream source corrections never block a pending send and never alter already-confirmed sends.
11. **`JoinAndLeaveSafe`:** Connecting to a practice never auto-sends historical items sitting in `To send`; ending a connection never deletes earlier sends from the patient's history.
12. **`SourceRemovalClearsMark`:** If a source removes a marked record, Curie Mobile clears its mark from `To send` with a plain notice while preserving earlier sends.
13. **`ConnectionEndStopsUncertain`:** If the practice connection ends while a send is `Sending` or `Uncertain`, Curie Mobile stops retrying, removes **Try again**, and keeps the unsent marks/notes intact in `To send`.
14. **42 CFR Part 2 send gate:** Confirming a send containing any Part 2 record requires explicit written consent confirmation (with a separate confirmation for SUD counseling notes) and attaches the verbatim 42 CFR § 2.32 redisclosure notice and consent scope to the frozen snapshot.

## 2.6 Portable Export, Account Deletion & Independent Privacy Receipt Journal

- **Portable patient export:** Patients can export their full account health records and generate formatted doctor-visit PDF summaries (via native Android FileProvider / iOS share sheet / browser blob download).
- **Independent create-only receipt journal:** Every user-initiated deletion or consent withdrawal is written to an independent, create-only journal (`put_if_absent`, byte-exact `read`, fail-closed `inventory`) stored outside the primary PostgreSQL database. Each immutable receipt carries metadata only—zero patient prose, PHI, or credentials (`schema_version`, `event_id`, `account_id`, `account_incarnation`, `operation`, `scope_id`, `scope_version`, `occurred_at`).
- **13 enforceable restriction operations:** `luna_withdraw`, `sessions_revoke`, `evidence_delete`, `doctor_page_delete`, `doctor_note_delete`, `doctor_item_delete`, `publication_revoke`, `device_withdraw`, `records_withdraw`, `follow_up_pause`, `follow_up_cancel`, `proactive_withdraw`, `account_delete`.
- **Bounded restriction delivery job:** Runs as an explicit non-HTTP job verifying journal + bridge schemas and failing non-zero whenever `failed > 0`, `budget_exhausted`, `pending > 0`, or journal unavailable.
- **Mandatory restore replay gate:** Any database backup/PITR restore MUST run full independent journal inventory replay **before** serving resumes—enforcing `account_incarnation` fencing so deleted accounts/data can never resurrect, and idempotently terminating associated connected-care links and scrubbing staging transfer records.
- **Schema/runtime privilege separation:** Runtime API connects as an unprivileged DB role and never runs DDL; `/readyz` verifies expected migration state before serving.
