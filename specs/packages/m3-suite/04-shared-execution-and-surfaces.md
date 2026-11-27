# 4. Shared Execution Harness, Clinical Library, Extension & Public Surfaces (M3 Increments 4–5)

## 4.1 Shared Bounded Execution Harness & Content-Free Telemetry

### Ownership Split
- **Shared harness owns:** Ordered turn loop, working-window budget management, typed provider request/stream boundary, JSON Schema tool argument validation and dispatch, in-memory `skill_view` execution, explicit cancellation (`stopped`), bounded turn/token/tool-step caps (`cut_off`), retry/error typing, and structured OpenTelemetry span emission.
- **Consuming product owns:** Clinical domain rules, prompts/recipes, tool authorization and side effects, patient/account scoping, durable persistence, and UI presentation. The harness imports zero product crates and zero database/KV store crates.

### Execution & Recovery Rules
1. **Ordered event stream:** Every run emits explicit ordered events (`WindowOpened` with Unix ms, `ProviderStarted`, tool calls & structured tool outputs, streamed steps/deltas, and terminal status `finished`, `stopped`, `cut_off`, or `couldnt_finish`).
2. **Exact restart reconstruction:** After a worker crash, restart, or lease handoff, the harness reconstructs working state exclusively from the product's admitted durable records—never a parallel shadow store.
3. **Explicit model role settings:** Every enabled role supplies explicit model identifier, reasoning effort, working-window budget, and input token limit—never implicit defaults.
4. **Frozen in-memory skills (`skill_view`):** System prompt carries metadata catalog only (`name`, `description`, supporting resource paths). `skill_view` reads strictly from the frozen in-memory SHA-256-verified skill package captured at turn admission, rejecting malformed paths/arguments without touching disk.
5. **Typed tool errors:** Tool failures return structured error payloads visible to the model and audit trail rather than collapsing into empty results.
6. **Content-free operational telemetry (Allowlist policy):**
   - Operational pipelines (`traces/ops`, `logs/ops`) enforce a strict **allowlist** of operational attributes (`service.*`, HTTP route/status, run/work/job/outcome correlation IDs, model/token counts, `error.type`), clearing `status.message` and rewriting log bodies to `"operational.event"`.
   - Prompts, completions, free-form errors, and clinical text **never** reach operational telemetry backends. Optional generation inspection flows solely when explicitly opted in under HIPAA-BAA isolation and OIDC caller allowlisting, behind bounded non-blocking queues (`queue_size = 1000`, `block_on_overflow = false`) so telemetry outages never stall clinical requests.

## 4.2 Clinical Library Nightly Pipeline, Offline Encoder & Synthetic Patient Bank

### Clinical Library Nightly Rules (All 9 Core Guarantees)
Queried strictly with de-identified clinical concepts—never patient identifiers.
1. **Manual or source-driven recovery only:** Only explicit operator action, newly arrived content, or a new reader version moves an article out of human-review or waiting state.
2. **Retry scope:** Nightly runs retry only articles explicitly in waiting-to-retry state; invalid transitions fail at type and DB constraint boundaries.
3. **Complete nights only:** Only a run finishing all due work records `Succeeded`; partial or deferred nights never report success.
4. **Repeated failure code stops:** Recording the identical failure code on **2 nights** halts automatic retries until resolved (including across crash takeovers).
5. **Consecutive failed/idle streak stops:** **3 consecutive** failed or zero-progress deferred nights halt the nightly until resolved; success or explicit resolution resets the streak.
6. **Takeover only of ended runs:** A new night takes over an active slot lease only after confirming the prior holder has ended—never while holder status is unknown.
7. **Unreadable holder alert:** A second consecutive night where the slot holder cannot be read triggers an operator alert.
8. **Lost-slot isolation:** A run that lost its lease writes zero DB rows, activates zero versions, and overwrites zero candidate object files (write-once candidate folders).
9. **Single checked active version:** At most one checked candidate version is active at any time (enforced via non-copyable activation handles and PostgreSQL partial unique indexes).

### PubMed Status Refresh
- Matches NCBI NLM retraction/correction records verbatim with content hashes; unchanged records re-confirm without replacement.
- Records lacking publication types classify as unreadable (never empty); deleted citations return nothing; book records resolve PMIDs.
- All concurrent NCBI requests share one rate limiter pacing starts **≥ 100 ms apart**; throttling/5xx back off; authorization refusals fail the night immediately without retrying or exposing keys.

### Offline Semantic Encoder & Synthetic Patient Bank
- **Offline semantic encoder:** Computes embeddings locally for semantic chart retrieval without sending chart text to external embedding APIs.
- **Synthetic patient bank:** Deterministic generator producing **468** clinically coherent primary-care charts (60 light, 122 moderate, 286 complex; 5 flagship units) spanning all 11 FHIR R4 clinical resource families + future `Appointment` calendar, physiological lab consistency, pending post-`as_of` results, multi-section >8 KB progress notes, and deterministic binary attachments (valid multi-page PDFs, isolated single-page corrupted PDFs preserving valid sibling pages, PNG/JPEG/fax scans). Strict runtime separation: runtime servers load only rendered FHIR bundles (`patients/`, `calendar/`, `manifest.json`), never authoring ledgers.

## 4.3 Chrome MV3 Read-Only EHR Extension

- **Architecture split:** Background service worker and EHR content/page-context readers stay ordinary non-Bevy extension JavaScript/TypeScript. Only the compact connection-status popup is a candidate for Bevy Wasm (gated by M1 budgets: ≤ 1.0 MiB Brotli-11, ≤ 250 ms cold start; otherwise narrow HTML/Wasm adapter per D2).
- **Strict security & privacy invariants:**
  - **Connector neutrality:** Core manifest, worker, connector, and popup HTML/JS contain zero EHR vendor, host, patient, or chart-part names.
  - **Optional host permissions only:** Declares `optional_host_permissions` (`https://*/*`, `http://*/*`) and zero pre-granted `host_permissions`; prompts only when user pairs/connects.
  - **Strict MV3 CSP:** `script-src 'self' 'wasm-unsafe-eval'; object-src 'self'` with zero inline scripts and zero `'unsafe-eval'`.
  - **Memory-only credentials:** Active bearer and refresh tokens live exclusively in `chrome.storage.session`—never persisted to `chrome.storage.local`, `chrome.storage.sync`, or disk checkpoints.
  - **Declared named reads & redaction:** Leased jobs specify only allowlisted identifiers (`id`, `lease`, `source`, `read`, `patient_id`, `item_id`) and never raw URLs. Billing, medication price, and invoice fields are stripped before computing SHA-256 payload receipts and delivering through the durable outbox.

## 4.4 Public Site, Interactive Demo & Dei Recovery Facility Finder

### Public Site & Interactive Synthetic Demo
- **Semantic, indexable HTML (zero Bevy canvas):** Serves marketing pages (`/`, `/mobile`), versioned legal & transparency documents (`/privacy`, `/terms`, `/clinical-transparency`, `/mobile/privacy`, `/mobile/privacy/:slug`, `/mobile/terms`, `/mobile/terms/:slug`, `/mobile/consumer-health-data` preserving verbatim approved legal archives), `noindex` `/invite` landing with App Store/Play Store fallback, `/robots.txt`, `/sitemap.xml`, `/health`, `/readyz`, and `/404`.
- **Embedded synthetic physician demo:** Isolated per-visitor in-memory chart store seeded strictly from synthetic fixtures, with verbatim citation matching, per-visitor turn budgets, and cloud or quantized local inference; zero access to production clinical databases or secrets.
- **Public ingress & cache posture:** Per-IP rate limiting, body caps (`64 KiB` standalone), public cache headers on static HTML/assets, strict `no-store` on errors/probes/proxied dynamic paths, and zero production exposure of staging-only observability routes.

### Dei Recovery Facility Finder (`dei.dabney.moe` & `/recovery*`)
- **Crisis lines first:** Every state and index page displays crisis resources (**988**, **911**, and **SAMHSA National Helpline**) **before** any facility listing.
- **Zero-JS semantic baseline:** Prerendered static state pages display the initial facility listing and complete city directory without JavaScript; progressive interactive finder mounts above it when JS/Wasm is enabled.
- **Versioned Cache Storage shard state machine:** Shards (`data/<ST>.json?v=<sha256>`) are versioned by content SHA-256 and cached asynchronously in Cache Storage (pruning superseded versions). Explicit load states:
  - `Loading { received, total }` (streaming byte progress over prerendered listing)
  - `Fresh` (latest shard cached, zero download)
  - `Stale` (older cached shard usable with warning while latest downloads)
  - `Failed` (keeps stale copy under warning or shows error alert + **Try again** over prerendered listing; failure **never** renders as zero results)
- **Offline reload resilience:** Service worker precaches current content-hashed JS/Wasm/CSS/icon bundle and caches visited pages network-first, retaining complete matching JS+Wasm pairs for cached pages so offline reloads continue filtering cached state shards seamlessly.
