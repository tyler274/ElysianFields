# Split Two-Board Swarm Protocol

This document specifies the split message-board architecture for the Elysian Fields swarm. Because a shared message board can leak reference paths, code structure, or observer context across the clean-room wall, board topology and access controls enforce the wall structurally.

## 1. Two-Board Topology

The swarm operates **two completely separate message boards** with independent storage roots, `0700` filesystem permissions, and separate role-bound membership tokens. No agent joins both boards.

### 1.1 Observer Board (Reference / Observer Environment)

Lives inside the observer-side environment. Only Observer (`Su`), Gatekeeper (`Aponia`), Verifier (`Mobius`), Integrator (`Eden`), and the observer-side coordinator hold membership tokens for this board.

| Channel | Purpose | Primary posters |
| --- | --- | --- |
| `#observe` | Reference behavior observations, inventories, baselines, and sanitized fixture handoffs | Observer (`Su`) |
| `#spec` | Specification drafting, authority-order resolutions, and Gatekeeper approval tracking | Gatekeeper (`Aponia`), Observer (`Su`) |
| `#verify` | Differential and black-box verification reports, mismatch findings routed to the Gatekeeper | Verifier (`Mobius`), Gatekeeper (`Aponia`) |
| `#integrate` | PR integration, CI/deploy status, release coordination, and wall-canary verification | Integrator (`Eden`), observer coordinator |

### 1.2 Builder Board (Elysian Realm)

Lives inside the isolated Elysian Realm environment. Only Builder (`Vill-V`) agents, Gatekeeper-facing question intake, and the builder-side coordinator hold membership tokens for this board.

| Channel | Purpose | Primary posters |
| --- | --- | --- |
| `#slice-<name>` | One channel per active implementation slice (for example, `#slice-m1`) for briefs, progress, and completion claims | Builder coordinator, Builder (`Vill-V`) |
| `#questions` | Implementation-neutral clarification requests raised by builders against a specification package | Builder (`Vill-V`), builder coordinator |

## 2. Two-Coordinator Rule

Coordinators are split by side of the wall:

- **Observer-side coordinator**: Reads `#observe`, `#spec`, `#verify`, and `#integrate`; tracks scope, milestones, verification, and integration. Because it has read observer posts, it **never authors builder briefs directly** and **never holds a builder-board posting token**.
- **Builder-side coordinator**: Operates strictly inside the Elysian Realm. It reads only the Elysian Realm repository (`specs/packages/`, engineering rules, slice diffs) and the builder board (`#slice-<name>`, `#questions`). It authors standalone builder briefs naming requirement IDs, the `specs/packages/` path, verification commands, and required return evidence.

### The Gatekeeper Bridge

The Gatekeeper (`Aponia`) is the sole bridge between the two sides:

- **Specification packages** cross the wall only as files committed under `specs/packages/` in `tyler274/ElysianFields`, never as forwarded board messages.
- **Builder questions** from `#questions` are answered by Gatekeeper (`Aponia`) committing updated or supplemental files under `specs/packages/`, after confirming the answer contains no reference paths, module layouts, internal signatures, or selectors.
- **Verifier mismatch reports** from `#verify` go to Gatekeeper (`Aponia`), who restates observable behavioral gaps as implementation-neutral requirements in `specs/packages/`.

## 3. Access Control and Wall Enforcement

The file-backed board harness (`scripts/board_split.py`) enforces split-board isolation with defense in depth:

1. **Separate storage roots**: Observer board state and builder board state reside in distinct root directories created with mode `0700` (owner-only access), with board secrets, membership registries, and channel logs stored with mode `0600`.
2. **Board-scoped HMAC membership tokens**: Each board generates its own 256-bit secret (`.board-secret`) and signs membership tokens binding `(board_id, board_side, agent_id, role)`.
3. **Role-to-side enforcement**:
   - `Vill-V` (Builder) and `coord-builder` tokens can only be issued for a `builder` board and are rejected with `WallViolationError` on any read or write attempt against an `observer` board.
   - `Su` (Observer), `Mobius` (Verifier), and `coord-observer` tokens are rejected with `WallViolationError` on any direct read or write attempt against a `builder` board.
   - An agent registered on one board cannot be registered on the opposite board (dual-membership raises `WallViolationError`).
4. **Channel allowlists**: Observer boards accept only `#observe`, `#spec`, `#verify`, and `#integrate`. Builder boards accept only `#questions` and `#slice-<slug>` channels.

## 4. Working Memory vs. Durable Records (Linear and Repository Sync)

Board posts are working memory, not durable truth:

- **Repository owns durable truth**: Architecture rules, protocol specifications, requirement ledgers, specification packages (`specs/packages/`), and verification scripts live in Git. Repository documentation carries no Linear ticket IDs.
- **Linear owns scope, work, and acceptance**: Linear sits strictly on the observer side of the wall. Builders have zero Linear credentials or connectors.
- **Syncing board outcomes to Linear**: Only the observer-side or integrator coordinator copies verified decisions, acceptance evidence, commit SHAs, and PR links from observer board posts (`#observe`, `#spec`, `#verify`, `#integrate`) to the corresponding Linear tickets. Progress, SHAs, test counts, and PR links go in ticket comments and status updates, never replacing durable repository docs.

## 5. Verification Rule: "Done" Is a Claim

A worker's completion post on either board (`[done] ...`) is treated as a **claim**, never as verified completion. Before accepting a slice, merging a PR, or advancing a Linear ticket:

1. The responsible coordinator inspects the complete diff (verifying no wall leaks, no extraneous changes, and adherence to engineering rules).
2. The coordinator independently reruns the stated verification commands (unit tests, lints, canary/harness checks) against the exact revision.
3. Only after independent diff inspection and green rerun evidence are observed does the coordinator mark the work verified and sync the evidence link.

## 6. Running the Split-Board Isolation Proof

```bash
# Run unit and wall-violation tests for the split-board harness
nix shell nixpkgs#python3 -c python3 -m unittest discover -s scripts -p 'test_board_split.py' -v

# Execute the end-to-end split-board proof and write the recorded transcript
nix shell nixpkgs#python3 -c python3 scripts/board_split.py \
  --markdown-out docs/swarm/board-split-transcript.md \
  --json-out /tmp/board-split-receipt.json
```

The latest recorded proof transcript lives at [board-split-transcript.md](board-split-transcript.md).
