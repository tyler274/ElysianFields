# 1. Remaining Curie Clinic Workflows (M3 Increment 1)

Covers all Curie Clinic capabilities beyond the M2 vertical slice: Today triage & priority queue, incoming patient message matching, reply drafting & withholding, longitudinal patient memory & background rounds, and multi-source EHR & communication channel ingestion.

## 1.1 Today Queue, Priority & Channel Health

### Purpose & Ordering
- **Today** orients the signed-in physician to their authorized panel's daily schedule, prepared visit briefings, urgent (`now`) and same-day (`today`) clinical concerns, unlinked or unread patient messages needing attention, and practice channel health banners.
- **Urgency & standing:** Actionable items carry explicit urgency (`now`, `today`), `waiting_since`, `kind`, `version_id`, `seen` standing, and `can_mark_done`. Marking an item seen or done records physician standing against its `version_id` so newer clinical changes resurface the item honestly.
- **Quiet vs incomplete honesty:**
  - When all panel patients were reviewed and nothing requires attention, display the explicit quiet state alongside `also_reviewed` count.
  - Whenever any upstream producer or access check fails, set `failed_parts` and `may_be_missing_someone = true` and surface the exact limitation—never presenting a partial read as a quiet day.
- **Channel health banners (`TodaySaysSo`):**
  - Transient channel read error (`temp_error`) -> display temporary **trouble** notice **without** asking the physician to reconnect.
  - Revoked channel credentials (`revoked`) -> display explicit **reconnect** prompt.
  - Healthy connection (`connected`) -> clear error banner.

## 1.2 Incoming Patient Message Matching (14 Formal Invariants)

### Conversation Matching States
| State | Meaning |
| --- | --- |
| `none` | No kept messages exist in this conversation. |
| `undecided` | First kept message awaits automated matching or the 15-minute fallback. |
| `to_match` | Surfaced on **Messages to match**, optionally carrying a suggested patient visible only to physicians authorized for that patient. |
| `filtered` | Surfaced in **Filtered** as likely non-patient traffic; still linkable by an authorized physician. |
| `linked` | Linked canonically to one patient and visible exclusively on that patient's chart. |
| `not_patient` | Explicitly dismissed as non-patient by a physician. |

### Matching, Move, Undo & Unreadable Rules (All 14 Verified Invariants)
1. **`OnlyWithAccess`:** A physician never sees a linked conversation for a patient outside their access scope.
2. **`SuggestionsWithAccess`:** **Messages to match** never reveals a suggested patient name/identity to a physician who lacks access to that patient.
3. **`JoinsItsPatient`:** While no Undo window is active, subsequent messages in a linked conversation stay bound to that patient and never auto-migrate.
4. **`LinksOnItsOwn`:** Automated linking occurs **only** when exactly one patient is confirmed with zero contradicting identity signals; ambiguous or contradicted matches route to `to_match` (or `filtered`).
5. **`RightPatient`:** A conversation appears exclusively on its current linked patient, never lingering on a previous patient after a move.
6. **`NeverInvisible`:** Every kept message lives in a concrete visible state, never counts as handled without explicit physician action, and exits `undecided` to `to_match` via a **15-minute fallback** so nothing stalls indefinitely.
7. **`UndoPutsBack`:** Moving a conversation (`linked`/`not_patient` -> accessible patient or `not_patient`) opens an Undo window preserving the original pre-move origin (even across chained moves). Pressing **Undo** restores the conversation and every kept message (including messages arrived during Undo) to the pre-move origin and cancels deferred downstream updates.
8. **`ActsAfterUndo`:** Downstream clinical record updates never execute while Undo is active, never execute for a no-op move back to origin, and always execute once a standing move's Undo window expires.
9. **`OnlyBetweenTheirPatients`:** A physician can link or move conversations only between patients they are authorized to access (or `not_patient`). Moving a conversation immediately closes active views for peer physicians lacking access to the destination patient.
10. **`TodaySaysSo`:** Today reports temporary channel trouble without asking to reconnect, and asks to reconnect only on revoked access.
11. **`ClearedOnDisconnect`:** Disconnecting a channel clears all unlinked and unreadable messages while keeping already-linked patient conversations intact.
12. **`ExactlyOnce`:** The channel read cursor advances strictly one message at a time after either keeping the message or placing it in the unreadable queue—never skipping or duplicating messages.
13. **`CantReadShows`:** Messages that fail to decode/read stay visibly surfaced in the unreadable queue until recovered on retry, explicitly marked **Done**, or cleared on channel disconnect.
14. **`BacklogOnlyWhenAccepted`:** Upon reconnecting a channel, pre-connection backlog is offered (`offered`) and **never** ingested until a physician records explicit consent (`accepted`).

## 1.3 Patient Message Reply Drafting & Withholding (12 Formal Invariants)

Curie Clinic assists with drafting patient message replies while keeping final review, editing, and outbound sending strictly in the physician's hands (Curie never auto-sends messages to patients).

### Conversation Review & Draft Model
- Tracks each patient message (`unwritten`, `open`, `answered`, `closed`), sent practice history, single active review state (`idle`, `woken`, `running`), reply state (`none`, `ready`, `withheld`, `closed_undo`), flagged clinical concern, Undo snapshot, channel connectivity, and per-physician personal drafts bound to the current reply version.
- Patient-facing reply text MUST omit all internal citation markup, source IDs, and tool identifiers; citations stay strictly in the physician-facing basis panel.

### Verified Reply Invariants (All 12)
1. **`OnlyWithAccess`:** Only physicians with access to the linked patient may view, edit, revise, close, or copy a reply.
2. **`OnlyTheReviewAnswers`:** Only a completed clinical review marks patient messages answered based on a **human** practice reply; automatic/auto-responder practice messages **never** mark a patient message answered.
3. **`ConcernStays`:** Closing a reply (**No reply needed** / **Done**) preserves any flagged clinical concern; moving the conversation to `not_patient` clears both reply and concern.
4. **`YourDraft`:** A physician's edited or AI-revised draft belongs strictly to that physician and its reply version; peer physicians and subsequent system review updates never overwrite or expose stale drafts. Physicians may **Restore system reply** to discard their personal draft.
5. **`CopiesWhatYouSee`:** **Copy reply** copies the exact text currently displayed on the physician's card without mutating reply or draft state.
6. **`CardShowsNew`:** The derived reply card always reflects every open patient message and whether a review is currently running.
7. **`OneReviewEach`:** At most one review runs per conversation at any time; server/worker restarts return an interrupted review to `woken` without duplicating or re-queuing already-consumed messages.
8. **`HistoryTrue`:** Sent practice history records strictly exact messages actually sent by the practice, never fabricated history.
9. **`ShowsUntilHandled`:** Open patient messages remain visible until handled, and every woken/running review eventually finishes.
10. **`WithheldAfterMove`:** Moving a conversation to another patient immediately withholds any prior ready reply and wakes a fresh review; a stale review for the old patient is discarded and never writes onto the new patient.
11. **`NoteWhileDisconnected`:** An open (`ready` or `withheld`) reply displays a channel-disconnected notice if and only if the backing communication channel is disconnected.
12. **`UndoBringsBack`:** Closing a reply opens an Undo window that restores the exact closed reply and open messages; Undo is invalidated cleanly if a new patient message arrives or the conversation moves.

## 1.4 Longitudinal Patient Memory & Background Rounds

1. **Monotonic rounds mark progression:** Each patient's rounds watermark advances only after an upkeep run finishes evaluating newly arrived records cleanly; failed or aborted runs never advance the watermark past unreviewed records.
2. **Quiet no-op runs:** When no material clinical change, new physician instruction, or source contradiction arrived since the prior mark, upkeep makes zero edits (no wording churn or duplicate memory entries).
3. **Dated provenance:** Every active memory item carries its clinical/observation date and distinct authority tag (`chart_record` vs `physician_instruction`).
4. **Disagreement preservation:** When physician direction contradicts an external chart record, both positions are retained with their respective dates—neither silently overwrites the other.
5. **History-preserving retirement:** Superseded clinical facts are retired with resolution date and reason rather than deleted; editing one fact never mutates unrelated active entries.
6. **Model-facing text hygiene:** All rendered skills and prompts pass automated checks enforcing zero tracker IDs and zero references to tool or schema fields absent from the offered tool catalog.

## 1.5 Clinic Setup, EHR Adapters & Communication Channels

### Practice Administration
- Practice admins manage physician roster membership, active/deactivated account standing (persisting across migrations and restarts), panel assignments, connected-care patient invitations, and clinical/channel source connections.

### Lossless Record Mapping (Closed 16-Bucket Model)
Every clinical source adapter preserves **untouched native source bytes** (JSON, XML, HL7 text) with SHA-256 digest, source identifier, revision/version ID, acquisition timestamp, and clinical dates, mapping into one closed set of **16 buckets**:
`Patient`, `Problems`, `Medications`, `Allergies`, `Results`, `Vitals`, `Notes`, `Documents`, `Visits`, `Procedures`, `Immunizations`, `Orders`, `History`, `Forms`, `Messages`, `Other`.

- **Mandatory patient-binding validation:** Every record validates patient identity (`Patient`/`subject`/`participant` in FHIR, `casefile_id` in Kipu, `recordTarget/patientRole/id` in C-CDA, `PID-3` in HL7 v2) before admission; missing, ambiguous, or mismatched bindings are rejected before storage.
- **Honest partial delivery (`FailedPart`):** Unreadable attachments (corrupted PDF pages, binary `nonXMLBody`, HL7 `OBX` `ED`) or malformed sibling records emit explicit `FailedPart` records without dropping valid sibling records from the batch.

### Supported External Sources & Channels
1. **Multi-vendor SMART on FHIR R4 EHRs:** Closed vendor catalog (Epic/MyChart, Oracle Health/Cerner, athenahealth, eClinicalWorks/healow, MEDITECH Expanse, NextGen, Veradigm, Elation Health, Custom FHIR R4). Supports `.well-known/smart-configuration` (+ `metadata` fallback), SMART v1 (`.read`) & v2 CRUDS (`.rs`) scopes, exact-base-URL `aud`, PKCE `S256` (+ confidential `client_secret` where required), explicit US Core scopes where wildcards are refused, bounded same-origin bundle pagination, and `$everything` with per-resource fallback. EHRs lacking live FHIR (e.g., SimplePractice) surface explicit export/assisted onboarding rather than fake live checks.
2. **KipuEMR Behavioral Health & 42 CFR Part 2:** Supports API v3 (`APIAuth` HMAC-SHA1 / `APIAuth-HMAC-SHA256`, `Accept: application/vnd.kipusystems+json; version=3`) and API v4 OAuth2 Bearer. Classifies every record under `ConfidentialityTag::Standard`, `Part2Sud`, or `Part2SudCounselingNotes` (42 CFR § 2.31(b)), attaches verbatim 42 CFR § 2.32(a) statutory redisclosure notice + § 2.32(b) consent scope, and gates Part 2 / SUD counseling notes from general exports without verified written consent.
3. **C-CDA R2.1 XML:** Validates `ClinicalDocument` & `recordTarget/patientRole`, maps LOINC (`2.16.840.1.113883.6.1`) & template OIDs across the 16 buckets, retains native XML slices + SHA-256, isolates `nullFlavor`/unrecognized/`nonXMLBody` into `FailedPart`.
4. **HL7 v2.x (`ADT`, `ORU`, `MDM`, `SIU`):** Parses dynamic `MSH-1`/`MSH-2` separators and escape sequences (`\F\`, `\S\`, `\T\`, `\R\`, `\E\`, `\.br\`), validates `PID-3`, maps `PID`/`PV1`/`SCH`/`DG1`/`AL1`/`RXE`/`OBR`/`OBX`/`TXA`/`NTE` into canonical buckets with untouched native bytes and per-segment failure isolation.
5. **SMART Health Links (`shlink:/`):** Validates HTTPS-only public destinations, exact 32-byte decryption keys, flags, and expiration before ingesting clinical payloads.
6. **Clinic Communication Channels (Gmail & Spruce):** Product-neutral Gmail API / authenticated SMTP AUTH adapter (for sign-in mail delivery and clinic inbox ingestion) and Spruce messaging channel adapter (inbound patient messages, channel health monitoring, and manual-copy reply workflows).
