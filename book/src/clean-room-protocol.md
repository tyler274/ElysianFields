# Elysian Fields Clean-Room Protocol

The Elysian Fields program replaces the product suite through behavioral clean-room reimplementation. The clean room is an engineering information boundary that ensures replacement clients, backends, execution harnesses, and persistence layers are built from verified behavioral specifications rather than by copying implementation structure. It is not a claim of legally certified independence.

Every Flame-Chaser name in this protocol appears beside the plain role term it stands for, so any reader can follow the rules without a glossary.

## 1. Authority Order

When sources disagree on required behavior, resolve the priority strictly in this order:

1. **Owner explicit decisions** — the project owner's latest explicit decisions.
2. **Intent specifications and safety, access, and consent rules** — product intent documents and non-negotiable clinical safety, access-control, privacy, and consent guarantees.
3. **Approved compatibility contracts and release obligations** — explicitly approved wire formats, persistence guarantees, platform store rules, and operational obligations.
4. **Observed behavior of the pinned reference** — behavior recorded from the pinned reference revision.

A disagreement between authority levels is recorded explicitly in the requirement's provenance and status (`accepted-difference` or `unresolved`). Disagreements are never settled silently by copying the reference.

### Reference Pinning and Intake

Observation runs against a single reference commit pinned by SHA at program start. Later changes on the reference mainline do not drift silently into active builds; each admitted change enters as a new observation with its own requirement IDs and passes through gatekeeper review before crossing the wall.

### Universal Exclusions

The following material is excluded from every role and never enters any staging directory, specification package, test fixture, or repository commit:

- Production credentials, API keys, tokens, and secrets.
- Protected health information (PHI) and real patient or clinician records.
- Real patient or clinician messages.
- Sealed evaluation material (`eval/`).

## 2. Roles and Permanence

Five roles divide observation, specification, implementation, verification, and integration across the clean-room wall.

| Role | Flame-Chaser | Sees | Produces | Must Not Access or Do |
| --- | --- | --- | --- | --- |
| **Observer** | **Su** | Pinned reference tree, running reference build, intent specifications, existing reverse-engineering evidence | Behavioral observations, scenario drafts, contracts, sanitized fixtures, visual references, performance baselines | Never writes into builder source crates, builder specification packages, or builder boards |
| **Gatekeeper** | **Aponia** | Observer outputs, intent specifications, verifier mismatch reports, builder questions | Approved implementation-neutral specification packages, ledger updates, allowed-input classifications, written answers to builder questions | Never writes replacement product code; never passes unstripped reference layout, symbols, or selectors across the wall |
| **Builder** | **Vill-V** | Approved specification packages, allowed assets, engineering rules, third-party public documentation, replacement source and tests | Replacement code, unit and integration tests, clean pull requests in the Elysian Realm | Never reads the reference repository, reference build caches, observer staging, observer board, or issue tracker |
| **Verifier** | **Mobius** | Both systems as black boxes; replacement source for independent quality and behavioral review | Differential test results, runtime evidence, and behavioral mismatch reports sent to Gatekeeper (Aponia) | Never sends raw reference internals to Builder (Vill-V); never edits replacement implementation code directly |
| **Integrator** | **Eden** | Replacement patches, CI pipelines, workspace tooling, and release infrastructure | Reviewed merges, CI/deploy enforcement, ledger coverage updates, and release builds | Never bypasses Gatekeeper (Aponia) to carry observer material into builder workspaces |

### One-Role-for-Life Rule

Every agent holds exactly one role for the life of the program. An agent assigned as Observer (Su), Gatekeeper (Aponia), Verifier (Mobius), or Integrator (Eden) never becomes a Builder (Vill-V), and a Builder (Vill-V) never joins observer-side work while retaining builder authority.

### Exposure Reassignment Rule

If a Builder (Vill-V) session is exposed to reference source code, reference git history, unapproved observer staging, or any other material from the observer side of the wall:

1. Quarantine the exposed builder session immediately and halt all edits.
2. Record the incident in the repository exposure log with timestamp, scope of exposure, and affected branches or workspaces.
3. Permanently reassign that agent out of the Builder (Vill-V) role; it may never author clean-room implementation code again.
4. Discard all unmerged work produced by the exposed session.
5. Spawn a fresh, unexposed Builder (Vill-V) session starting solely from the Gatekeeper (Aponia)-approved specification package.

## 3. The Clean-Room Wall

The wall is enforced by repository, workspace, board, and automated linting boundaries.

### Elysian Realm Isolation

Builders (Vill-V) work exclusively inside the **Elysian Realm** — a separate Git repository with zero shared commit history with the reference repository. A builder environment contains only:

- The build environment, toolchain, lint configuration, and engineering rules.
- Gatekeeper (Aponia)-approved specification packages under `specs/packages/` and the requirement ledger at `specs/ledger.toml`.
- Explicitly allowlisted assets.
- Replacement source crates and tests authored behind the wall.

Builder sessions never use linked worktrees of the reference repository, never mount reference build caches or agent stores, and carry no issue-tracker, GitHub, or conversation-search connectors.

### Two Boards and Two Coordinators

Communication is physically split so observer context cannot leak into builder instructions:

- **Observer board** (observer-side environment) with four channels: `#observe`, `#spec`, `#verify`, and `#integrate`.
- **Builder board** (Elysian Realm environment) with per-slice channels (`#slice-<name>`) and `#questions` for Gatekeeper (Aponia).
- **No shared membership**: no agent joins both boards.
- **Two coordinators**: a coordinator that reads observer-board posts never authors builder briefs directly. Builder briefs stand alone and reference only requirement IDs, approved package paths under `specs/packages/`, verification commands, and reporting expectations.

### Gatekeeper-Only File Bridge

Gatekeeper (Aponia) is the sole bridge across the wall:

- Nothing crosses from the observer side to the Elysian Realm except Gatekeeper (Aponia)-approved specification packages, approved assets, and written answers committed as version-controlled files under `specs/`.
- Raw board posts never cross the wall.
- Mismatch reports from Verifier (Mobius) go to Gatekeeper (Aponia), who translates each discrepancy into an implementation-neutral requirement clarification, contract update, or new acceptance scenario before Builder (Vill-V) sees it.

### Forbidden Crossings

The following artifacts and details are forbidden in any file behind the wall (`specs/` and replacement `crates/`), and in protocol documentation (`book/`):

- Absolute or relative filesystem paths into the reference repository or observer staging directories.
- Reference crate names, internal module hierarchies, and non-public symbol or visibility signatures (such as crate-private function signatures) inside behavioral specifications.
- DOM selectors, test-attribute hooks, CSS class names, and HTML element IDs inside behavioral specifications.
- Reference UI framework names or renderer-specific implementation details.
- Issue-tracker ticket IDs in repository documentation or specifications (issue trackers track progress externally; repository files state durable truth).

Automated linting (`scripts/check_wall.py`) runs in CI and local verification to reject any forbidden crossing before merge.

## 4. Implementation-Neutral Specification Format

Specifications describe observable behavior, domain state transitions, contracts, and accessibility semantics. They never prescribe module layout, internal Rust visibility signatures, DOM structure, or styling hooks from the reference.

Every requirement in a specification package MUST state all eleven elements below:

1. **ID** — unique requirement identifier (`EF-<DOMAIN>-<NNNN>`).
2. **Product and Platform(s)** — the owning product (`curie-clinic`, `curie-mobile`, `connected-care`, `shared-platform`, `browser-extension`, `public-web`) and every target platform in scope.
3. **Provenance** — applicable authority level(s), requirement status, and an implementation-neutral provenance note (intent principle, contract name, or abstract reference observation — never a reference source path or symbol).
4. **Actors and Authority** — every human role, client surface, or service participating in the interaction and the permissions each holds.
5. **Preconditions** — starting system state, authentication standing, network/storage conditions, and required test data shape.
6. **Actions** — ordered user or system inputs expressed in domain and interaction terms.
7. **Visible Result** — what the user perceives on screen, in navigation state, or in audio/assistive output.
8. **Persisted and External Effects** — durable state transitions, audit records, network/protocol effects, and data-isolation invariants.
9. **Failure and Restart Behavior** — explicit handling for network loss, service errors, invalid inputs, duplicate triggers, process restarts, reloads, and background/resume transitions. Failed operations must look like failure and never silently read as empty or complete.
10. **Accessibility and Input Behavior** — semantic landmarks, focus management, keyboard navigation, screen-reader announcements, IME/composition, clipboard/paste, and bidirectional/multi-script text expectations.
11. **Executable Acceptance Scenario** — at least one machine-readable scenario (`SCN-<DOMAIN>-<NNNN>`) in the package's `scenarios.toml` that verifies the requirement end-to-end.

### Product Boundary Invariant

Curie Clinic (physician) and Curie Mobile (patient) maintain separate accounts, clinical data types, and stored state. Specifications and implementations never share clinical state across products except through explicit connected-care flows governed by verified consent.

## 5. Requirement Statuses and Coverage States

Requirement status (what the program decides about a requirement) and coverage state (what verification has proved on a specific platform) are strictly separate axes and are never conflated.

### Requirement Statuses

Every requirement carries exactly one status:

| Status | Meaning |
| --- | --- |
| `observed-and-required` | Confirmed against authority sources and required in the replacement. |
| `intended-but-unimplemented` | Required by owner decision or intent specification, even though the pinned reference does not yet implement it. |
| `accepted-difference` | Replacement intentionally differs from the pinned reference under a higher-authority decision or contract; the difference is documented explicitly. |
| `unresolved` | A conflict or ambiguity exists between authority levels or observations; blocked from builder implementation until resolved by Gatekeeper (Aponia) or the project owner. |

### Coverage States

Every requirement tracks verification coverage per target platform using exactly one state:

| Coverage State | Meaning | Required Evidence Fields |
| --- | --- | --- |
| `unknown` | Not yet verified on this platform. | `revision = ""`, `evidence = ""` |
| `blocked` | Cannot be verified yet due to missing hardware, unprovisioned deployment, or an upstream dependency gap. Never recorded as `passed`. | Non-empty `reason` explaining the block |
| `waived` | Explicitly exempted for this platform by an approved authority decision. | Non-empty `reason` citing the authority decision |
| `passed` | Verified on this platform against a specific replacement commit with inspectable evidence. | Non-empty `revision` and existing repository `evidence` path |

Traceability runs continuously across six links: **Requirement -> Specification -> Acceptance Scenario -> Replacement Revision -> Platform -> Evidence**.

## 6. Identifier Scheme and Specification Package Layout

### Requirement and Scenario IDs

Requirement identifiers follow `EF-<DOMAIN>-<NNNN>` (four-digit zero-padded sequence). Executable acceptance scenarios follow `SCN-<DOMAIN>-<NNNN>` with the matching domain code.

| Domain Code | Example Requirement ID | Owning Surface |
| --- | --- | --- |
| `CLINIC` | `EF-CLINIC-0001` | Curie Clinic physician workflows |
| `MOBILE` | `EF-MOBILE-0001` | Curie Mobile patient workflows |
| `CONN` | `EF-CONN-0001` | Connected care invitations, verification, consent, and lifecycle |
| `HARNESS` | `EF-HARNESS-0001` | Shared execution harness, clinical library, encoder, and shared services |
| `PLATFORM` | `EF-PLATFORM-0001` | Cross-platform UI catalog, text input/IME, accessibility, graphics, and platform adapters |
| `EXT` | `EF-EXT-0001` | Browser extension popup, worker, and content-script duties |
| `PUBLIC` | `EF-PUBLIC-0001` | Public web pages, demo surfaces, and indexable legal documents |

### Canonical Target Platform Slugs

The requirement ledger and platform register share one canonical set of platform keys:

- `browser-wasm-desktop`
- `browser-wasm-mobile`
- `desktop-linux`
- `desktop-macos`
- `desktop-windows`
- `android-native`
- `ios-native`
- `chrome-extension-popup`
- `hosted-services`
- `static-public-web`

### Repository Layout

Specification packages and the machine-readable ledger live under `specs/`:

```text
specs/
  ledger.toml
  packages/
    <slug>/
      spec.md
      scenarios.toml
      contracts/          # Optional machine-readable schemas, state models, or protocol contracts
```

- `specs/ledger.toml` is the single machine-readable index of every requirement, its status, its owning package, its acceptance scenario ID, and its per-platform coverage records.
- `specs/packages/<slug>/spec.md` holds the human-readable implementation-neutral specification.
- `specs/packages/<slug>/scenarios.toml` holds executable acceptance scenarios referenced by the ledger.
- `specs/packages/<slug>/contracts/` holds optional machine-readable contracts (JSON Schema, state machine definitions, or protocol fixtures) stripped of all reference internals.

## 7. Gatekeeper (Aponia) Review and Approval Checklist

Before any specification package, builder answer, allowed asset, or verifier mismatch restatement crosses the wall into the Elysian Realm, Gatekeeper (Aponia) verifies every item below:

- [ ] **Implementation neutrality**: Zero reference repository paths, reference crate or module names, crate-private signatures, DOM selectors, CSS class names, HTML IDs, or reference UI framework names appear anywhere in the package.
- [ ] **Format completeness**: Every requirement includes all eleven mandatory sections (ID, product & platforms, provenance, actors & authority, preconditions, actions, visible result, persisted & external effects, failure & restart behavior, accessibility & input behavior, and at least one executable acceptance scenario).
- [ ] **Authority and status accuracy**: Authority order is respected; conflicts between intent and observation are recorded explicitly (`accepted-difference` or `unresolved`), never papered over by copying the reference.
- [ ] **Ledger and scenario traceability**: Every requirement ID is unique, matches `EF-<DOMAIN>-<NNNN>`, is registered in `specs/ledger.toml`, resolves to an existing package directory and `spec.md`, and points to an existing `SCN-<DOMAIN>-<NNNN>` entry in `scenarios.toml`.
- [ ] **Coverage integrity**: Every declared platform has an explicit coverage record using only `unknown`, `blocked`, `waived`, or `passed`; unavailable platforms are marked `blocked` with a reason, never `passed`.
- [ ] **Privacy and exclusion check**: Zero production credentials, zero PHI, zero real messages, zero sealed `eval/` material, and zero issue-tracker ticket IDs appear in the package or book.
- [ ] **Product boundary compliance**: Curie Clinic and Curie Mobile clinical state remain strictly separated except through explicit connected-care consent contracts.
- [ ] **Automated gates green**: `scripts/check_ledger.py`, `scripts/check_wall.py`, and `mdbook build book` all pass with zero warnings or errors.
