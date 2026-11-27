# 3. Streamed Clinical Chat, Cancel / Error Recovery, Reload & Durable Readback

## 3.1 Command Envelope & Idempotency (`POST /commands`)

Every state-changing chat action uses a typed command envelope (body limit `≤ 2 MiB`):

```json
{
  "type": "<command_kind>",
  "idempotency_key": "<client-uuid>",
  "payload": { ... }
}
```

- **Auth & access check:** Requires a live authenticated session and `Allowed` membership on `patient_workspace_id`; unauthorized workspaces return `404 Not Found` (never `403` leakage).
- **Idempotency:** Repeating a command with the same `idempotency_key` returns `200 OK` `{ "outcome": "accepted", ... }` without creating a duplicate turn, duplicate tool side effect, or duplicate state transition.
- **Durable acceptance before ACK:** `message.submit` persists the physician message and pending work item durably in PostgreSQL **before** returning `200 OK` to the client.
- **Validation failures:** Empty message text or blank `chat_id` returns `400 Bad Request` `{ "outcome": "rejected", "message": "..." }`. Store outages return `503 Service Unavailable`.

### Core M2 Vertical-Slice Commands

| Command `type` | Payload Fields | Acceptance / Conflict Semantics |
| --- | --- | --- |
| **`message.submit`** | `patient_workspace_id`, `chat_id`, `text`, `physician_time_zone` (default `"UTC"`), optional `delivery` (`"send_now"` \| `"send_next"`) | Persists new turn (or queues if `send_next`), wakes worker via `LISTEN`/`NOTIFY`, returns `{ outcome: "accepted", patient_workspace_id, chat_id, work_item_id }`. Submitting a newer message supersedes any earlier turn's `resume` / `try_again` handles (`null`). |
| **`work.stop`** | `patient_workspace_id`, `chat_id`, `work_item_id` | Requests immediate cancellation of the matching active turn. Returns `200 OK` with `{ outcome: "accepted", returned_messages: [...] }` or `409 Conflict` if no matching active turn exists. |
| **`work.resume`** | `patient_workspace_id`, `chat_id`, `work_item_id`, `expected_run_id` | Resumes a `stopped` turn or retries (`Try again`) a `couldnt_finish` / `cut_off` turn, fenced by `expected_run_id` so stale clicks or superseded turns return `409 Conflict` (`"This turn can no longer be resumed."`). |

## 3.2 Exchange Lifecycle & Answer State Machine

Each conversation snapshot (`GET /patient-workspaces/{workspace_id}/chats/{chat_id}/snapshot`) returns:
- `transcript`: ordered list of `Exchange` records `{ work_item_id, messages, steps, kept, answer }`.
- `sources`: map of `source_key -> CitedSourceWire` summary metadata.
- `queued`: ordered pending follow-up messages (if any).
- `archived`: boolean.

### `AnswerState` Variants

| State | Fields | Visible UI Behavior |
| --- | --- | --- |
| **`Working`** | `stop: { work_item_id }`, `writing: bool`, streamed partial text/steps | Displays live execution steps and incrementally appended markdown prose; shows active **Stop** control. |
| **`Finished`** | `response_id`, `attempt_id`, `finished_at`, `text`, `segments`, optional `basis`, `newer_records` | Displays complete formatted answer with interactive inline citation chips (`[1]`, `[2]`, …). |
| **`Stopped`** | `resume: Option<{ work_item_id, expected_run_id }>` | Retains all completed steps and any partial kept text; shows **Resume** control when `resume` is `Some`. |
| **`CutOff`** | `try_again: Option<{ work_item_id, expected_run_id }>` | Triggered when explicit turn/token/tool-step budget is reached; keeps completed work and shows **Try again** when `try_again` is `Some`. |
| **`CouldntFinish`** | `try_again: Option<{ work_item_id, expected_run_id }>` | Triggered after bounded automatic retries are exhausted (5 failures); displays clear failure notice and explicit **Try again** control. Never loops silently in the background. |

### Tool & Evidence Step Trace (`ExchangeSteps`)

- `ExchangeSteps` is explicitly tagged `Read(Vec<Step>)` or `Unavailable` (with logged cause)—never silently dropping unreadable steps or blanking the transcript.
- Ordered step kinds surfaced to the physician during and after streaming:
  - `SearchedChart { query, group, page, state }`
  - `ReadRecords { requested, state }`
  - `ReadConversation { state }`
  - `SavedMemoryNote { state }`
  - `EditedMemory { state }`
  - `CreatedFollowUp { state }`

## 3.3 Incremental Streaming & Cross-Instance Event Delivery (`GET /patient-workspaces/events`)

### Streaming Architecture Contract (Incremental Delta Delivery)

- **SSE transport:** Long-lived `text/event-stream` (`Cache-Control: no-cache`), emitting keep-alive comment frames every **15 seconds** (`KEEP_ALIVE_EVERY`) and monotonic event `id` cursors (`?since=<cursor>` for reconnect catch-up).
- **Incremental delta delivery (M2 requirement):** While an exchange is `Working`, the stream delivers lightweight incremental token/text delta frames and step-progress frames (debounced at ~50–75 ms so UI updates coalesce smoothly at 60 fps without per-character thrashing). The client appends deltas into the active turn's typed domain buffer rather than re-downloading and re-parsing the entire 80-turn transcript on every token.
- **Lossy invalidation & `resync`:** Cross-instance fan-out uses a lossy pub/sub bus (Valkey/Redis). The bus never stores durable work. On subscriber lag, broker restart, or reconnect gap, the server emits an explicit `resync` event, causing connected clients to fetch the authoritative snapshot from PostgreSQL without losing turns, steps, or citations.
- **Zero idle DB polling:** Idle open SSE connections wait solely on pub/sub and PostgreSQL `LISTEN` notifications; idle Postgres transaction count remains near zero.
- **Authority termination:** Revoking or signing out a session broadcasts an authority notice that closes that session's open SSE streams on every server instance immediately while leaving other sessions unaffected.
- **Client reconnect backoff:** On stream drop or >30 s silence past keep-alive, the client aborts the connection and reconnects with bounded backoff from the last seen event `id`.

## 3.4 Worker Durability, Lease Recovery & Bounded Retry

- **Single-claim lease:** Background workers claim due turns via atomic PostgreSQL leases (`60 s` default lease, renewed while executing; `5 s` due-work sweep plus immediate `LISTEN`/`NOTIFY` wakeup on `physician_work`).
- **Stall & crash reclaim:** If a worker dies or stops making progress past the stall timeout (`300 s`) or fails lease renewal, another worker reclaims the expired lease and reconstructs the exact working state from admitted durable records (never from a second parallel recovery store).
- **Frozen skill & recipe snapshot:** When a turn is accepted, the active skill package and role model settings are frozen alongside the work item so retries and restarts execute identically.
- **Bounded retry schedule:**
  - Counted failures retry automatically with exponential backoff: **`10 s` initial delay, doubling per failure up to a `600 s` ceiling** (`10 s`, `20 s`, `40 s`, `80 s`, …; configurable to `0 s` in local deterministic tests, refused in hosted mode).
  - Maximum **5 failed attempts**. On the 5th failure, the turn transitions permanently to `CouldntFinish` (`WorkStatus::Failed`) and waits for explicit physician **Try again** (`work.resume`).
- **Cancel honesty:** Stopping an active turn (`work.stop`) cancels the active provider stream or tool call promptly, preserves all completed steps and partial kept output, and transitions the turn to `Stopped` with a fresh `expected_run_id`.

## 3.5 Composer Draft Autosave, Optimistic Concurrency & Reload Readback

Per `(patient_workspace_id, chat_id)` target (using `"draft"` before a chat is created):

- **Read/write endpoints:** `GET` / `PUT` `/physician-state/drafts/{workspace_id}/{chat_id}` carrying `{ text, expected_revision }` -> returns `{ text, revision }` on save or `409 Conflict` with `{ outcome: "conflict", current: { text, revision } }`.
- **Single-owner dirty derivation:** Whether a draft is unsaved is derived solely from `local_text != saved_baseline_text` (never a separate mutable `dirty` boolean).
- **Single in-flight save with generation fencing:** At most one save runs per target. Every keystroke increments a local `generation` counter; a late save reply updates only its own baseline and never overwrites newer keystrokes (instead scheduling a trailing save).
- **Validated conflict resolution:** On `409 Conflict`, autosave pauses and presents the server's newer revision alongside two explicit physician choices:
  - **Retry (keep mine):** saves local text over the server's reported revision.
  - **Use newer text:** replaces local text with the server's newer revision.
- **Transient save failure:** Shows `"Not saved"` and waits for the next edit or slow backoff retry—never a tight retry loop and never treats an unvalidated failure as authorization to overwrite.
- **Submit & durable clear:** Submitting a message captures `(text, generation, idempotency_key)`. Once accepted, only the submitted prefix is cleared (text typed after pressing Send is preserved), and the durable draft clear queues behind any in-flight save.
- **Reload & restart durability:** Refreshing the browser (or restarting the backend server/worker) reloads both persisted chat exchanges (including `Working`/`Stopped`/`CouldntFinish`/`Finished` states, steps, and citations) and the saved composer draft from PostgreSQL without losing user words.

## 3.6 Deterministic Provider Fixture Contract (M2 Verification)

To qualify M2 end-to-end without live external LLM credentials or nondeterminism, the harness supports a deterministic loopback provider fixture (`model_execution = stub`) with the following behavioral contract:

1. **Scripted tool-call rounds:** Accepts a configurable ordered list of tool-call rounds (e.g., Round 1: `search_chart`, Round 2: `read_records`, Round 3: streamed final clinical answer with verified inline citations) matched by deterministic `call_id` progression (`fixed-<round>-<index>`).
2. **Streamed chunk pacing:** Supports configurable per-chunk delay (`delay_ms`) so browser/client tests can reliably trigger mid-stream **Stop**, concurrent typing/scrolling, and live step rendering.
3. **Scripted fault injection:** Supports atomic countdown fault triggers:
   - `failures` counter: returns transient upstream `5xx` / mid-stream drop for the next `N` provider calls (used to deterministically verify bounded 5-attempt retry -> `CouldntFinish` -> **Try again** recovery).
   - `rejections` counter: returns explicit non-retryable / tool-refusal responses to verify honest error surfacing.
4. **Request transcript capture:** Records every provider request JSONL payload to an inspectable verification artifact so verifiers can assert prompt scoping, zero cross-patient leakage, and exact tool schema compliance.
