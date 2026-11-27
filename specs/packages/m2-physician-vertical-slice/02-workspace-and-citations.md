# 2. Patient Selection, Workspace Access Control & Evidence / Citation Inspection

## 2.1 Practice-Scoped Access Control (Three-Tier Membership)

Every clinical read, workspace load, chat command, and citation lookup MUST verify authorization server-side against `(practice_id, physician_id, patient_workspace_id)`:

| Membership Tier | Meaning | Permitted Operations |
| --- | --- | --- |
| **`Allowed`** | Patient belongs to the physician's practice and is on this physician's panel. | Full read/write workspace access: chart reader, clinical synopsis, conversations, streaming chat, citations, follow-ups, memory. |
| **`ClinicDemographic`** | Patient belongs to the physician's practice, but is not on this physician's panel. | Demographic roster coordination only (name/identity card, panel assignment where permitted); **no** clinical chart content, chat transcript, or citation body disclosure. |
| **`Denied`** | Patient belongs to another practice or does not exist. | Server returns `404 Not Found` (`Cache-Control: no-store`) on all workspace, chat, and citation routes so existence of cross-practice patients is never leaked. |

## 2.2 Patient List & Workspace Selection

### Patient List (`GET /patient-workspaces`) & Roster Search (`GET /patient-roster?query=...`)

- Returns only patients authorized for the signed-in physician's practice/panel:
  - `patients`: ordered list of `{ patient_workspace_id, display_name, cursor }`.
  - `incomplete`: optional boolean indicating partial upstream availability.
  - `may_be_missing_someone`: boolean set `true` whenever a source outage or authorization store error prevented checking every candidate patient.
- **Honest empty vs error states:**
  - Zero panel patients with healthy backend -> explicit empty state (e.g., quiet Today / no matching patients for query).
  - Unmatched search query -> empty result list; pressing `Escape` clears populated search text while leaving the finder overlay open; a second `Escape` closes the finder and restores focus.
  - Backend error (`503` / network failure) -> explicit error state with **Try again**; a failed read **never** renders as an empty patient list.

### Patient Workspace Detail (`GET /patient-workspaces/{workspace_id}`)

- Returns `{ patient_workspace_id, display_name, membership, time_zone, chats, identity, patient_synopsis, connected_sources, connection_status, ... }`.
- Selecting a patient transitions the client to `Workspace(Patient { workspace_id, chat_id })` and loads the active (or most recent non-archived) conversation snapshot plus workspace evidence summaries.
- Deep-link URL query parameters `?patient=<workspace_id>&chat=<chat_id>` select the target workspace and chat on authenticated load (subject to `Lands` and `TodayFirst` rules in `01-auth-and-sessions.md`).

## 2.3 Evidence Reading & Inline Citation Inspection

### Structured Answer & Citation Model

Each completed or streaming assistant answer carries structured segments:
- `Text { text }`: narrative markdown prose.
- `Citation { source_key, number, label, quote, target }`: inline numbered citation chip (`[1]`, `[2]`, …) where `target` is one of:
  - `WholeRecord` (entire clinical document or lab panel)
  - `Passage` (specific verbatim excerpt inside a multi-section note)
  - `PassageNotFound` (explicitly surfaces notice: `"Cited passage could not be matched in this version of the record."`)

Each cited source payload (`GET /patient-workspaces/{workspace_id}/chats/{chat_id}/sources/{source_key}`) returns:
- `source_key`, `number`, `source_label`, `title`, `record_type`, `recorded_at`, `record_status`, `origin` (`SourceRecord` vs `PreparedByAssistant`), `quote`, `body`, `readable_parts` (ordered `{ label, text }` sections), `fields` (structured `{ key, name, value, unit, range, status }` lab/clinical rows), optional `raw_content`, and optional `external_url`.

### Record Status Classification

Record status badges distinguish settled clinical records from flagged records (case-insensitive, `_` normalized to `-`):
- **Flagged** (visually highlighted warning): `"preliminary"`, `"inactive"`, `"refuted"`, `"entered-in-error"`.
- **Settled** (neutral): `"final"`, `"amended"`, or other settled clinical statuses.

### Non-Modal Cited-Source Reading Panel State Machine

The citation reader opens beside the transcript (non-modal on wide viewports; slide-over sheet/pane on compact mobile viewports) so the physician can keep reading or typing while inspecting source evidence:

| State | Trigger / Behavior |
| --- | --- |
| **`Idle`** | Panel closed; no source selected. |
| **`Loading { request_id, chip }`** | Clicking an inline citation chip immediately shows loading state and increments a monotonic `request_id`. Any late HTTP response whose `request_id` is stale is discarded so rapid clicking across chips never shows the wrong record. |
| **`Loaded { chip, source }`** | Renders record header (title, type, clinical date, status badge, provenance), highlighted verbatim quote/passage inside `readable_parts`, structured `fields` table, and optional raw view. |
| **`Unavailable { eyebrow, title, reason }`** | Shown when the cited record was withdrawn, redacted, or access was revoked (`404`/`403` or access-change event). Clears all previously displayed clinical text immediately—never leaves stale protected text visible. |
| **`Failed { request_id, chip, message }`** | Transient network/`5xx` error; displays explicit failure message and **Try again** control that re-requests the same `source_key`. |

### Provenance, Isolation & Redaction Invariants

1. **Exact-read provenance:** Every clinical citation traces to a specific retained record revision actually read during the run. No fabricated IDs, dates, or numeric lab values from uninspected snippets.
2. **Verbatim snippet fidelity:** Quoted passages, lab values, units, reference ranges, and clinical timestamps match the retained source revision bytes.
3. **Cross-patient & cross-practice isolation:** Resolving `source_key` enforces both `patient_workspace_id` ownership and physician panel access; a citation in patient A's chat can never resolve or return a record from patient B.
4. **Withdrawn & stale record redaction:** If a cited source is withdrawn or superseded after an answer was generated, opening the citation redacts withdrawn content or explicitly marks it as no longer current rather than presenting stale data as live chart truth.
5. **Unreadable attachments stay explicit:** Corrupted or unreadable attachments (e.g., an isolated unreadable PDF page) are explicitly noted beside the answer while valid sibling pages remain readable.
6. **Large structured record performance (M1/M2 budget):** Opening a structured lab/record source containing **5,000 rows** (`fields`) MUST use virtualized row rendering (`O(viewport rows)` draw/layout cost) and complete within budget (`≤ 150 ms` browser Wasm, `≤ 100 ms` desktop, `≤ 120 ms` mobile) with zero >50 ms main-thread stalls.
7. **Original record language provenance:** Verbatim clinical source records always remain in their original source language; UI displays the localized provenance notice (`OriginalRecordLanguageNote`) when viewing records under a non-English UI locale.
