# Elysian Fields: Bevy product-suite clean-room replacement plan

Planning artifact. It authorizes no source change, deployment, release or production access. M0 and M1 are the only authorized milestones.

## 1. Mandate

Replace the product suite in `dabney-moe` by behavioral clean-room reimplementation, with Bevy owning interactive presentation on browser Wasm, desktop and mobile.

- Narrow native or DOM adapters may supply platform text input, accessibility, extension duties and indexable public documents.
- Product backends, agent execution, persistence and integrations are in scope, not only the clients.
- Development and deployment tooling stays unless replacement is necessary.
- The clean room is an engineering information boundary. It is not a claim of legally certified independence.
- Stretch goal, after everything else: ElyMalloc as the allocator and ElyLD as the linker for the whole stack (milestone M6). The program is complete without it.
- Delivery proceeds through incremental pull requests and fast CI in `github.com/tyler274/ElysianFields`.

The swarm executes the work over its message board. Linear (team Curie, key `TYL`) tracks scope, work and acceptance. The repository owns durable truth.

### Names

The program is **Elysian Fields**. **Elysia** is a subproject of it whose scope is not yet set; nothing in this plan is assigned to it. The program's names come from the Flame-Chasers of Honkai Impact 3rd. A name always appears beside the plain term it stands for, so a cold reader needs no glossary.

| Milestone | Signet | Plain name |
| --- | --- | --- |
| M0 | Discipline | Reference, wall and coverage |
| M1 | Helix | Bevy feasibility |
| M2 | Deliverance | One vertical slice |
| M3 | Infinity | Suite replacement |
| M4 | Vicissitude | Data and operations rehearsal |
| M5 | Daybreak | Adoption and retirement |
| M6 | Stars | Stretch: ElyMalloc and ElyLD |

| Role | Flame-Chaser |
| --- | --- |
| Observer | Su |
| Gatekeeper | Aponia |
| Builder | Vill-V |
| Verifier | Mobius |
| Integrator | Eden |

The builder repository is the **Elysian Realm**.

## 2. Reference

The reference is `dabney-moe` `origin/main`, pinned at the start of milestone M0 to SHA `01b762838d639a51f82e9d7d15e788e02bac772f`.

The local checkout at `/home/luluco/code/dabney-moe` sits on `tyler/messages-archive`, which shares no history with `origin/main` and is not the reference. Observers work from a read-only worktree of the pinned SHA `01b762838d639a51f82e9d7d15e788e02bac772f`. On `origin/main`:

- The patient product is Curie Mobile (`crates/mobile-*`), and the physician product is Curie Clinic.
- There are no submodules. The sealed-evaluation rule in `AGENTS.md` still applies to any `eval/` fixtures.
- The workspace has 43 members, including `ancestry*`, `dei-site`, `recovery-directory` and a typed i18n layer with RTL support in `crates/ui`.
- `intent/README.md` names no renderer, so no intent edit is needed to permit Bevy.
- `book/src/clean-room-reclamation.md` documents a two-stage protocol that has been run across the whole workspace.

`origin/main` continues moving, and the EHR-integrations project is active on it. Under decision D3, the baseline remains pinned at `01b762838d639a51f82e9d7d15e788e02bac772f`, and later `origin/main` changes are admitted incrementally as new observations carrying new requirement IDs.

## 3. Scope

M0 produces the finite inventory. This list is the seed, and every item gets a classification: replace, retain as tooling, reuse as approved asset, or out of scope.

- **Curie Clinic physician**: identity and practice access, Today, patient workspace, chart reading and citations, chat, message matching and reply drafts, patient memory and rounds, clinic and source setup.
- **Curie Mobile**: account and session, records and evidence, on-device health acquisition and upload, notifications, For my doctor, export, consent, deletion.
- **Connected care**: invitations, date-of-birth verification, Share consent, ending and rejoining connections.
- **Shared execution**: harness loop, provider and tool interfaces, skills, cancellation, quotas, telemetry.
- **Clinical library, encoder, record mapping, patient bank**.
- **Clients**: `physician-app`, `mobile-app`, the `mobile` shell, `ui`, `ui-gallery`, Android and iOS bridges under `platform/`.
- **Chrome extension**: popup, worker and content-script duties, authorized EHR reads.
- **Public site and demo**, `front-door`, `dei-site` and `recovery-directory`.
- **Unclassified until M0**: `ancestry*`, `gmail`, `spruce`, `physician-bus`, `availability-probe`, `observability-relay`, `telemetry`, `http`.
- **Retained by default**: `workspace-tools`, `android` runner, `boundaries`, Nix, CI, `infra/`.

No supported surface disappears without an explicit scope decision. An unavailable device or deployment is a blocked item, never a pass.

### Authority order

1. The user's latest explicit decisions.
2. Intent specs and their safety, access and consent rules.
3. Approved compatibility contracts and release obligations.
4. Observed behavior of the pinned reference.

Each requirement carries one label: observed-and-required, intended-but-unimplemented, accepted difference, or unresolved. A disagreement between levels is recorded, not settled by copying the reference.

## 4. Clean-room protocol

This plan extends the protocol in `book/src/clean-room-reclamation.md` and changes it in three places. That chapter is updated in the same PR that starts M0, so the repository states one protocol.

1. **Specifications are implementation-neutral.** The existing protocol specifies module layout, `pub(crate)` signatures, DOM selectors and CSS classes. Those describe the old implementation and have no meaning on a Bevy canvas. A requirement here states actors, preconditions, actions, visible result, persisted and external effects, failure and restart behavior, and accessibility behavior.
2. **A gatekeeper sits between observation and implementation.** Nothing crosses the wall except a gatekeeper-approved specification package.
3. **Builders get an allowlisted view, not a worktree.** "Delete the target unread" does not isolate a builder who can still read siblings and history.

### Roles

| Role | Sees | Produces |
| --- | --- | --- |
| Observer | Reference tree, running reference, intent specs, existing REA evidence | Scenarios, contracts, sanitized fixtures, visual references, performance baselines |
| Gatekeeper | Observer output, intent specs | Approved specification packages; answers to builder questions |
| Builder | Specification package, allowed assets, engineering rules, third-party docs, its own code | Replacement code and tests |
| Verifier | Both systems as black boxes; replacement source for quality review | Mismatch reports, sent to the gatekeeper |
| Integrator | Replacement patches and the workspace | Reviewed PRs on `origin/main` |

An agent holds one role for the life of the program. An exposed builder is reassigned, and the exposure is recorded.

### Builder view

Builders work in the Elysian Realm, a separate repository with no shared history, seeded with the flake, lint configuration, the "Writing code" rules from `AGENTS.md`, the allowed-asset set and the specification packages. The integrator carries their patches into `crates/*` on a `coda-sandbox` worktree, one worktree and one PR per slice. Builder sessions have no Linear, GitHub or conversation-search connectors, and no access to agent stores or build caches from the reference.

The board's `inherit`, `share` and `branch` workspace modes all hand a worker the full repository, so builders are never added through them.

### Existing inputs to classify in M0

- The 696 REA evidence bundles and their specifications. They are observer-side material. They reach builders only after the gatekeeper strips layout, signatures and selectors.
- The TLA+ models under `verification/`. They state protocol behavior and are likely approvable as specifications.
- The Chromium suites and UI Automator tests. They are observer-side scenario sources; their DOM assertions are restated as behavior.
- Brand assets, fonts, legal text, clinical skills, i18n strings and synthetic patients. Each needs an explicit allow or deny.

Production credentials, PHI, real messages and sealed evaluation material are excluded from every role.

## 5. Target architecture constraints

- One shared Bevy component, theme, motion and accessibility catalog serves both products.
- Typed domain state stays independent of ECS entity IDs and view lifetimes. The client holds no second authoritative store.
- The two products keep separate accounts, clinical types and storage. Only connected care moves data, and only with consent. The `boundaries` tests extend to the new crates.
- Backends are headless Rust services. Bevy is not a dependency of anything server-side.
- Provider and tool secrets never ship in a client bundle.
- Platform adapters own HTTP and streams, secure storage, files, deep links, notifications, IME and clipboard, health APIs and lifecycle.
- A text-input or accessibility bridge has one state owner. It never creates a second editable copy or a duplicate screen-reader element.
- The extension's worker and content scripts stay ordinary extension code. Only the popup is a candidate for Bevy.
- The web app and its APIs live under `dabney.moe/elysianfields`, so the browser client and the services it calls run from that subpath, not from a domain root.
- Public and legal documents stay semantic, indexable HTML.
- A call from a new client to a reference backend is allowed during transition and is labelled transitional.

Crate names and dependencies are chosen after M0 and M1, not copied from the reference crate graph.

### Fast CI and deploy

Fast CI is a requirement, not a preference. The site that serves `dabney.moe` today is too slow to build and deploy, and Elysian Fields must not inherit that. A change reaches a reviewed result, and a merged change reaches `dabney.moe/elysianfields`, within budgets set in M0 and enforced from the first commit. A pipeline that exceeds its budget is a failing gate, the same as a failing test.

The existing site's pipeline already selects jobs by changed paths, runs its gates in parallel and restores a Rust build cache, and it is still too slow. M0 measures that pipeline directly ([`docs/ci/dabney-moe-website-ci-baseline.md`](ci/dabney-moe-website-ci-baseline.md)) and derives hard budgets ([`docs/ci/budgets.md`](ci/budgets.md), [`docs/ci/budgets.json`](ci/budgets.json)). Its bottlenecks:

- Scope is coarse. Any code change runs every build, test and image job.
- The release is compiled twice per run: once as the web build, and again inside the server image's Nix build.
- The server image compiles in one Nix derivation that caches dependency sources but not compiled dependencies. Any source change recompiles every dependency in release mode. This is the single largest build cost, solved with separated dependency roots.
- Every deploy ships a whole new server image and rolls a new service revision, however small the change.
- The Rust build cache is keyed on the lockfiles, so a dependency bump starts cold.
- It is one workspace and one deployable. A change to one page pays for all of it.

What `dabney-moe-rust` already does, and Elysian Fields takes as its starting point. This is CI tooling, which the plan retains; it is not product code and does not cross the wall:

- **Run only what a change affects.** A scope script selects jobs from the changed files. Pushes to `main` and shared-infrastructure changes run everything.
- **Build dependencies once.** Third-party dependencies are separate cached build roots, so a source change recompiles only workspace code.
- **Share a binary cache.** Dependency roots, container images and the dev shells are published to a private Nix cache from `main` only, and every run and developer pulls from it.
- **One local command equals CI.** A single script runs the same checks in tiers: touched crates, pre-commit, full. A failure is found locally, not after a push.
- **Keep slow proof off the routine path.** Device and browser qualification runs on demand or on a schedule, and a green routine run does not claim it.
- **Share build output across worktrees.** New worktrees start from a shared seed of third-party build output.

What is new here, because Bevy makes this harder:

- Bevy is a large dependency with long cold builds and long links. The dependency cache must cover every target: native, Wasm, Android and iOS.
- Wasm size optimization is slow. It runs on release builds only, never on the routine path.
- Scope is per deployable and per target, not one switch for all code.
- A release artifact is built once. The image and the deploy consume it and never compile again.
- A client-only change deploys static files without a new service image or revision.
- Elysian Fields deploys on its own. A change here does not rebuild or redeploy the rest of `dabney.moe`, and the reverse holds.
- Feasibility (M1) measures clean and incremental build time and link time per target, beside the runtime measurements. A target that cannot meet the build budget is reported the same way as one that fails a runtime budget.

ElyLD's incremental linking is the M6 stretch goal for shipped binaries. Using it earlier for development builds only is a separate, smaller decision, taken on the M1 link-time measurements.

## 6. Milestones

M1 needs nothing from the reference, so it runs alongside M0. No backend reimplementation starts before M1 exits.

### M0. Reference, wall and coverage

- Pin the reference SHA (`01b762838d639a51f82e9d7d15e788e02bac772f`), build IDs, configuration profiles and fixture versions.
- Inventory and classify every scope item, existing input and target platform.
- Stand up the builder repository and prove the wall with a canary: a builder session asked to find a named reference file fails.
- Bring the board onto the `origin/main` lineage (section 8).
- Create the requirement ledger.
- Measure where the existing site's pipeline spends its time, set the CI and deploy time budgets from that, and stand up the pipeline that meets them on an empty workspace.

**Exit:** agreed finite scope, platform register, allowed-input list, working wall, working board, a pipeline within its time budgets, and decisions D2 to D4 answered.

### M1. Bevy feasibility

Disposable prototypes from public documentation and synthetic fixtures, on a pinned Bevy version, on real devices and browsers. Record the baseline first on the agreed motivating stutter scenario: **long streaming LLM chat transcript while typing/scrolling and opening citations in Curie Clinic (browser) and Curie Mobile**, measured on the pinned reference before measuring any prototype.

Qualify:

- Multiline text input, IME, selection, clipboard, dictation, mobile keyboard resize and focus restoration.
- Shaping and bidirectional layout for the nine shipped languages, including Arabic and the Indic scripts.
- Screen readers on every target: VoiceOver, TalkBack, and browser and desktop readers. Establish which targets Bevy's accessibility layer reaches natively and what each remaining target's adapter costs.
- Long streaming transcripts, large records, citations and rich text while typing, resizing and backgrounding.
- Wasm download size, cold start, WebGL2 and WebGPU coverage, context loss, and CSP.
- Android and iOS pause, resume, process recreation, deep links, notifications and health permissions.
- Extension popup cold start under MV3 CSP.
- Idle redraw and battery behavior.
- Clean build, incremental build and link time per target, against the CI budgets.

Budgets are set from the baseline before a prototype is judged.

**Exit, per target:** pass; pass with an approved adapter and its cost; or fail. Under decision D2, a failed target first tries a narrow approved adapter; if none fits, a non-core target may be dropped, while a failed browser accessibility or text-input result stops the program for review.

### M2. One vertical slice

Shared catalog, platform adapters and one physician journey: sign-in, patient selection, evidence reading, streamed answer, cancel and error recovery, reload and durable readback. It runs against a newly written thin backend and harness slice with deterministic provider fixtures.

**Exit:** the journey passes on browser, one desktop target, Android and iOS within budget, with recordings, accessibility evidence, traces and persisted readback. The measured effort of this slice becomes the estimate for M3.

### M3. Suite replacement in increments

1. Remaining physician workflows, access, ingestion and communications.
2. Curie Mobile accounts, records, health, notifications, privacy and export.
3. Connected care.
4. Remaining shared execution, clinical library, encoder and background work.
5. Extension, public site, demo, and the surfaces M0 added.

Each increment carries its own data path, background work, failure behavior and platform UI. Observation and specification for an increment run one increment ahead of its build.

**Exit, per increment:** requirement coverage, independent review, passing contract and differential tests, runtime evidence, and no open safety, access or consent failure.

### M4. Data and operations rehearsal

Runs only for domains where M0 finds stored data that must survive. Prefer logical export and import. Rehearse on synthetic data at scale: interruption, retry, deletion, account boundaries, pending jobs. One writer per domain; shadow runs disable every outbound effect. Rollback is defined before cutover.

**Exit:** migration and recovery evidence, dashboards and alerts, packaging and signing proof, and a rollout proposal for the user.

### M5. Adoption and retirement

Release by approved environment and cohort. Retire a reference path once its consumers, parity, retention duties and recovery are covered. Remove transitional adapters.

**Exit:** every scope item has a replacement disposition and evidence, and no reference product code remains on an accepted execution path.

### M6. Stretch: ElyMalloc and ElyLD

Starts after M5 exits. It is outside the completion contract, and no earlier milestone depends on it or designs around it.

**Outcome:** every shipped binary allocates through [ElyMalloc](https://github.com/tyler274/ElyMalloc), both as the Rust global allocator and as the C `malloc` for native code in the same process. Builds link with [ElyLD](https://github.com/tyler274/ElyLD): ThinLTO for release, incremental for development.

What the two projects' READMEs state today, set against this program's targets:

| Target | ElyMalloc | ElyLD |
| --- | --- | --- |
| Linux services and desktop | `GlobalAlloc` and C ABI; checked on x86-64, aarch64, riscv64 | Supported (ELF) |
| Browser Wasm | `wasm32-unknown-unknown` checks clean; smoke crates exist | Experimental, behind a feature |
| Android | Not listed | ELF ARM64 supported; Android not stated |
| macOS, iOS | `aarch64-apple-darwin` checks clean; iOS not listed | Mach-O experimental, behind a feature |
| Windows | `x86_64-pc-windows-msvc` checks clean | No COFF support |

"Checks clean" is `cargo check`, which is not a runtime qualification. Every cell needs its own evidence.

Adoption is per target, in this order: Linux services, developer builds, browser Wasm, Android, Apple, Windows. A target stays on its platform allocator and linker until it passes.

Rules:

- Patches land in the two upstream repositories. This workspace pins released revisions through the flake and carries no fork.
- One allocator instance per process. Rust and native code never free each other's memory across two allocators.
- Switching a target back is a configuration change, and that switch is rehearsed before the target adopts.
- ElyLD's incremental mode falls back to a full link under `--gc-sections`, which Rust passes by default. Development builds need an explicit choice between the two.
- ElyMalloc's mitigations are always on. Their cost is measured against the M1 budgets, not assumed.

Validation, per target:

- The target's full acceptance suite passes with no other change.
- A soak on the streaming-transcript and long-session scenarios holds memory and latency within the M1 budgets.
- ElyMalloc's fuzz and chaos suites pass on the target itself.
- An incremental link produces a binary that behaves the same as a clean link of the same inputs.
- An ElyLD ThinLTO release binary passes the same suite as the default linker's, and crash reports still symbolize.

**Exit, per target:** adopted with evidence, or left on the platform default with the blocking upstream gap named.

### Verification

Traceability runs requirement, specification, acceptance test, replacement revision, platform, evidence. Unknown, blocked, waived and passed are distinct states.

- Domain and state-machine tests, and the existing TLA+ properties where approved.
- API, event and protocol contract tests.
- Differential scenarios against the pinned reference and the replacement.
- Real input and accessibility tests on devices. Engine test hooks supplement them.
- Persistence, isolation, restart and migration tests.
- Performance, cold-start and idle-power qualification.

Existing gates stay in force while the reference serves users, including the physician `site` suite. Each DOM-based gate gets a behavior-equivalent replacement gate, approved before the old one is retired. For LLM paths, deterministic fixtures compare access, event order, tool results and persisted effects; live model quality is judged separately.

## 7. Linear

One project in team Curie: **Elysian Fields**. It is related to, and separate from, "Clean-Room Reclamation & Code Ownership", whose outcome was authorship.

- **Project description**: the mandate, the authority order, links to the repository chapters that own the protocol and architecture, and the list of decisions made. No progress snapshot; status updates carry progress.
- **Milestones**: M0 to M6 as above, each with its exit gate as the milestone description. M6 holds one backlog ticket until M5 exits.
- **Labels**: one `wall` label per ticket, named for the role that does it (`Su · observe`, `Aponia · spec`, `Vill-V · build`, `Mobius · verify`, `Eden · integrate`), plus `decision`.
- **Relations**: `related` by default; `blocks` only when work cannot start without the other deliverable.

Every ticket follows the repository's `write-linear-ticket` skill:

1. First sentence: what is true when the work is done.
2. Scope, with decided, open and idea kept apart.
3. Where to start: the owning spec section and the owning crate or feature.
4. What it consumes and produces, and who owns each side.
5. Numbered acceptance criteria. Each names the condition, where to observe it and the evidence.

Progress, SHAs, test counts and PR state go in comments and status updates, never in a description.

Linear sits on the observer side of the wall. Builders have no Linear access. A build ticket is still written so it could be shown to a builder: it names requirement IDs and the specification package, and no reference path, symbol or selector. The gatekeeper approves the text of each build ticket.

The requirement ledger and specification packages live in the repository. Tickets point at ranges of requirement IDs. Repository docs carry no ticket IDs.

### First tickets (M0 and M1)

| Kind | Outcome |
| --- | --- |
| decision | D2 to D4 below are answered and recorded |
| integrate | The reference SHA, build IDs and fixtures are pinned and published in the ledger |
| observe | Every workspace member, surface and existing input has a classification |
| observe | The platform register lists each target with versions and current support evidence |
| integrate | The builder repository exists and the wall canary fails as required |
| integrate | The board runs from `origin/main` with the channel layout in section 8 |
| integrate | CI and deploy meet their time budgets on an empty workspace |
| spec | The protocol chapter states the revised protocol and the specification format |
| observe | The stutter scenario is named and measured on the reference |
| build | Text input and IME prototype qualified per target |
| build | Accessibility prototype qualified per target, with adapter costs |
| build | Streaming transcript and large-record prototype measured against budget |
| build | Wasm size, cold start, graphics backend and CSP measured, including the extension popup |
| build | Mobile lifecycle prototype qualified on Android and iOS |
| build | Build and link time measured per target against the CI budgets |
| decision | M1 results are reviewed and each target is passed, adapted or failed |
| idea (M6) | ElyMalloc and ElyLD serve every target that can pass with them |

## 8. Swarm and message board

Per `AGENTS.md`, the swarm authors, reviews, merges and records ticketed work, and an operator session hands work to the swarm's coordinator and does not start it.

The board tooling (`coda-swarm` MCP server, `crates/workspace-tools/swarm`, the `swarm-coordinator` skill) exists only on `swarm-board-supervisor`, which is on the old lineage. It is not on `origin/main`, not in `.cursor/mcp.json`, and no `.swarm/` exists. `swarm-pipeline` is on the `origin/main` lineage and carries the credentials and pipeline doctor only. Landing the board on `origin/main` is M0 work.

The board is a channel across the wall, so its layout enforces the wall:

- **Two boards.** The observer board lives in the main checkout. The builder board lives in the builder repository. No agent joins both.
- **Two coordinators.** A coordinator that has read observer posts cannot write builder briefs.
- **Observer board channels**: `#observe`, `#spec`, `#verify`, `#integrate`.
- **Builder board channels**: one per slice, plus `#questions` for the gatekeeper.
- **The gatekeeper is the only bridge.** Specification packages and answers cross as files committed to the builder repository, not as board posts. Verifier mismatches go to the gatekeeper, who restates them as requirements.
- **Briefs stand alone.** A builder brief names the requirement IDs, the package path, the verification commands and what to post back.
- **A worker's "done" is a claim.** The coordinator reads the diff and reruns verification before a ticket moves.

Board posts are working memory. Decisions and evidence move to the Linear ticket; durable rules move to the repository.

## 9. Program decisions

- **D1. What the program reimplements. Decided: everything.** Clients, backends, harness and persistence are all reimplemented behind the wall. Existing workspace code is reference material, not a reusable input.
- **D2. What a failed target means. Decided: narrow approved adapter first; otherwise drop non-core targets, and stop for review on browser web accessibility or text-input failure.** Try a narrow approved adapter first; if none fits, drop the non-core target, but stop for review if browser web accessibility or text input fails.
- **D3. Reference intake. Decided: pin `dabney-moe` `origin/main` at `01b762838d639a51f82e9d7d15e788e02bac772f` and admit later `origin/main` changes as new observations with new requirement IDs.**
- **D4. Desktop targets. Decided: Linux, macOS, and Windows are all required desktop targets.**
- **Motivating stutter scenario. Decided:** long streaming LLM chat transcript while typing/scrolling and opening citations in Curie Clinic (browser) and Curie Mobile.
- **Delivery model. Decided:** incremental pull requests and fast CI in `github.com/tyler274/ElysianFields`.

## 10. Estimation and stop rules

No whole-program estimate exists before M2. Authorize M0 and M1 only. M2 is authorized on the M1 results, and M3 on the measured cost of M2.

Stop for review when: a required target fails M1 with no acceptable adapter; the pipeline cannot be brought within its time budgets; an adapter grows into a second UI stack; the wall is breached in a way reassignment cannot contain; or an increment costs more than twice its estimate.

## 11. Basis

Read for this plan: `AGENTS.md`, `intent/README.md`, `book/src/clean-room-reclamation.md`, `book/src/black-box-replacement.md`, `book/src/ui-surface-map.md`, section headings of `book/src/release.md`, and the root `Cargo.toml` member list on `origin/main`; the `swarm-coordinator` skill at `swarm-board-supervisor`; branch and worktree metadata; the Curie team's projects and issues in Linear; the READMEs of ElyMalloc (`main3`) and ElyLD (`main`); and, for CI, the "Continuous integration" and "Private Nix binary cache" sections of `book/src/release.md` plus direct GitHub Actions timing measurements of `tyler274/DabneyMoeWebsite` (`ci.yml`) recorded in [`docs/ci/dabney-moe-website-ci-baseline.md`](ci/dabney-moe-website-ci-baseline.md).

No product source was read. No application was run, no platform was measured, and no board was started when drafting this plan. The author of this plan is on the observer side of the wall.
