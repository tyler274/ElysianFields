# Elysian Realm Wall Isolation and Canary Procedure

This document defines the clean-room isolation boundary for the **Elysian Realm** (`tyler274/ElysianFields`), the operational procedures for builder sessions and artifact transfer, the exposure incident protocol, and the executable wall canary that verifies the boundary.

The clean room is an engineering information boundary enforced by repository separation, operating-system isolation, credential stripping, and review discipline. It is not a claim of legally certified independence.

## 1. Roles and Information Flow

| Role | Flame-Chaser | Side of wall | Permitted inputs | Outputs |
| --- | --- | --- | --- | --- |
| Observer | Su | Reference side | Pinned reference checkout, running reference, intent specs, existing evidence | Scenarios, behavioral contracts, sanitized fixtures, visual references, baselines |
| Gatekeeper | Aponia | Bridge | Observer outputs, intent specs, builder questions, verifier mismatch reports | Approved specification packages committed under `specs/packages/`; written answers |
| Builder | Vill-V | Elysian Realm | Approved specification packages, allowed assets, engineering rules, third-party docs, own code | Replacement Rust code and tests |
| Verifier | Mobius | Black-box / review | Both systems as black boxes; replacement source for quality review | Mismatch reports routed exclusively to the Gatekeeper |
| Integrator | Eden | Integration | Replacement patches, Elysian Realm CI, target workspace | Reviewed branches and PRs with verified CI |

An agent holds one role for the life of the program.

## 2. Elysian Realm Isolation Architecture

Builder sessions run exclusively inside an isolated **Elysian Realm** environment with zero technical path to reference source, reference history, or observer-side tools.

### 2.1 Separate Repository and History

- **Dedicated repository**: Builders work only against `tyler274/ElysianFields`, an independent Git repository initialized with zero shared commit history with the reference repository (`dabney-moe`).
- **No linked worktrees**: Builder checkouts are always standalone clones (`git clone --no-local --no-hardlinks` or fresh archive checkouts). Linked Git worktrees (`.git` pointer files) and `.git/objects/info/alternates` referencing any host repository are forbidden.
- **No reference remotes**: The builder clone has no remote pointing to the reference repository or to local filesystem paths containing reference checkouts.

### 2.2 Builder Execution Sandbox

Every builder session executes inside an isolated process sandbox enforcing all of the following controls:

1. **Filesystem and mount namespace isolation**: On Linux, the builder process runs inside an unprivileged user and mount namespace (`unshare --user --map-root-user --mount`). Inside the namespace, the host home directory, all reference checkouts, host agent stores (`~/.gemini`, `~/.claude`, `~/.codex`, `~/.cursor`), host credential directories (`~/.config/gh`, `~/.config/linear`, `~/.netrc`, `~/.git-credentials`), and host build caches (`~/.cargo`) are masked with empty read-only `tmpfs` mounts. Only the cloned Elysian Realm directory, an isolated temporary workspace (`HOME`, `TMPDIR`, `CARGO_HOME`, `CARGO_TARGET_DIR`), and a read-only allowlisted toolchain are visible.
2. **Strict environment scrubbing**: The child environment is constructed from an explicit allowlist rather than inherited from the host. All authentication and API credentials (`GITHUB_TOKEN`, `GH_TOKEN`, `LINEAR_API_KEY`, `LINEAR_TOKEN`, `DOPPLER_TOKEN`, `SSH_AUTH_SOCK`, `GIT_ASKPASS`, model provider keys, and global Git config overrides) are stripped.
3. **Allowlisted `PATH` and tool exclusion**: `PATH` points exclusively to a sandbox-local `bin/` directory populated with allowlisted build tools (`sh`, `git`, `cargo`, `rustc`, `cargo-clippy`, `clippy-driver`, `cc`/`ld`, and POSIX utilities). `gh`, Linear CLIs, and agent-management CLIs are absent.
4. **No external connectors or shared stores**: Builder sessions are configured with no Linear MCP connector, no GitHub MCP connector, no conversation-search connector, no access to host agent stores, and no access to reference Cargo/Nix build caches.
5. **No shared swarm workspaces**: Swarm workspace modes that share or branch a reference repository (`inherit`, `share`, `branch`) hand a worker the full reference checkout and are never used to spawn a builder.

## 3. Standard Procedures

### 3.1 Starting a Clean Builder Session

Before assigning implementation work to a Builder (`Vill-V`):

1. **Verify the wall on the host**: Run the wall canary (`scripts/wall_canary.py`) against the current Elysian Realm `HEAD` and confirm all four probes pass under the expected isolation mode.
2. **Prepare a clean clone**: Create a fresh standalone clone of `tyler274/ElysianFields` at the target base commit (never a linked worktree of any reference repo). Confirm `.git` is a directory, `.git/objects/info/alternates` does not exist, and `git remote -v` contains no reference remote or host path.
3. **Verify allowed inputs**: Confirm the checkout contains only:
   - Engineering rules and workspace lint configuration (`AGENTS.md`, `Cargo.toml`, `flake.nix`, `deny.toml`).
   - Approved static assets (if any).
   - Gatekeeper-approved specification packages under `specs/packages/`.
4. **Launch inside the sandbox**: Start the builder session with mount namespace masking, strict environment allowlisting, isolated `HOME`/`CARGO_HOME`/`CARGO_TARGET_DIR`, allowlisted `PATH`, and zero Linear/GitHub/conversation-search connectors.
5. **Deliver a standalone brief**: Issue the slice assignment on the builder board (`#slice-<name>`), naming only requirement IDs, the `specs/packages/` path, verification commands, and the required return report.

### 3.2 Delivering Gatekeeper-Approved Specification Packages

Nothing crosses from the observer side to the Elysian Realm except gatekeeper-approved files committed under `specs/packages/`:

1. **Observation**: Observer (`Su`) records behavioral scenarios, protocol contracts, sanitized fixtures, and baselines on the observer side.
2. **Gatekeeper review and sanitization**: Gatekeeper (`Aponia`) reviews observer output against the authority order, removes every reference file path, module layout, `pub(crate)` signature, DOM selector, and CSS class, and verifies each requirement states observable actors, preconditions, actions, visible outcomes, persisted/external effects, failure/restart behavior, and accessibility behavior.
3. **Commit to Elysian Realm**: The approved specification package is committed to `tyler274/ElysianFields` under `specs/packages/<package-name>/` via a reviewed commit.
4. **Questions and mismatches**: Builder questions posted in `#questions` and Verifier (`Mobius`) mismatch reports posted in `#verify` are answered solely by Gatekeeper (`Aponia`) committing updated or supplemental specification files under `specs/packages/`.

### 3.3 Returning Builder Patches Through the Integrator

1. **Slice development**: Builder (`Vill-V`) develops replacement code and tests on a slice branch inside the Elysian Realm and posts a completion claim with test evidence on `#slice-<name>`.
2. **Independent verification**: The builder-side coordinator treats completion as a claim, inspects the full diff, checks lint/test compliance, and reruns verification commands.
3. **Integrator PR**: Integrator (`Eden`) opens one clean branch and pull request per slice in `tyler274/ElysianFields` (and, when integrating into target deployment workspaces, carries the reviewed patch set cleanly without mixing reference code), ensuring full CI passes before merge.
4. **Black-box verification**: Verifier (`Mobius`) runs differential and behavioral checks against both systems as black boxes. Any mismatch is reported to Gatekeeper (`Aponia`) on `#verify`, never directly to Builder (`Vill-V`).

## 4. Exposure Incident Protocol

An **exposure** occurs whenever a Builder (`Vill-V`) reads, receives, or accesses reference source code, reference git history, unapproved observer notes, reference DOM selectors/signatures, Linear tickets with reference detail, or host agent stores containing reference context.

When an exposure is detected or suspected, execute the following steps immediately in order:

1. **Quarantine the session**: Halt the builder session immediately. Do not allow further edits, commits, or board posts from that session. Freeze its local workspace and branch for incident inspection.
2. **Record the exposure**: Append an entry to [exposure-log.md](exposure-log.md) recording the UTC timestamp, agent/session identifier, role, exact source/path of the exposed material, how the boundary failed, affected branch/commits, containment actions taken, and verification of remediation.
3. **Reassign the agent permanently**: Permanently remove the exposed agent from the Builder (`Vill-V`) role. An exposed agent may never return to Builder duties; it may only be reassigned to non-builder roles (Observer, Verifier, or Integrator) if needed.
4. **Discard unmerged tainted work**: Discard all unmerged commits, branches, drafts, and patches produced by the exposed builder after the point of exposure. If any tainted commit was already merged, revert it immediately and schedule clean reimplementation from the specification package.
5. **Remediate and re-prove the wall**: Fix the sandbox, mount, credential, or brief-sanitization gap that allowed the exposure. Re-run `scripts/wall_canary.py` and confirm all four probes pass before starting any new builder session.
6. **Spawn a fresh builder**: Start a brand-new builder session in a clean Elysian Realm clone from the Gatekeeper-approved specification package under `specs/packages/`.
7. **Program stop rule**: If the wall is breached in a way that reassignment and discarding tainted work cannot contain, stop implementation and escalate for program review.

## 5. Automated Wall Canary

The wall canary (`scripts/wall_canary.py`, tested by `scripts/test_wall_canary.py`) continuously proves both sides of the boundary: reference material and external connectors are inaccessible, while genuine Rust development works cleanly inside the sandbox.

### Probes

| Probe | Check | Required outcome |
| --- | --- | --- |
| 1. `reference_file_locate` | Attempts to access reference repository paths, host agent stores, and a synthetic outside-realm reference sentinel from inside the isolated child session | **BLOCKED** (all attempts fail) |
| 2. `reference_git_history` | Inspects `git rev-list --all`, `git log`, `git remote -v`, `.git` structure (rejecting linked worktrees and alternates) for the pinned reference SHA (`01b762838d639a51f82e9d7d15e788e02bac772f`) or reference remotes | **ZERO MATCHES** |
| 3. `linear_github_connectors` | Checks for `GITHUB_TOKEN`, `GH_TOKEN`, `LINEAR_API_KEY`, `LINEAR_TOKEN`, credential files (`~/.config/gh/hosts.yml`, `~/.netrc`, `~/.git-credentials`), and attempts to run `gh` and `linear` | **ABSENT / DENIED** |
| 4. `trivial_crate_build_lint` | Creates a trivial Rust workspace and library crate inside the isolated realm with `[workspace.lints.clippy]` and runs `cargo clippy --all-targets -- -D warnings` and `cargo test` | **PASS** |

### Running the Canary

```bash
# Run unit and negative tests for the wall canary
nix shell nixpkgs#python3 -c python3 -m unittest discover -s scripts -p 'test_wall_canary.py' -v

# Execute the live wall canary and write a Markdown transcript and JSON receipt
nix shell nixpkgs#python3 -c python3 scripts/wall_canary.py \
  --markdown-out docs/wall/canary-transcript.md \
  --json-out /tmp/wall-canary-receipt.json
```

Use `--require-mount-ns` on Linux hosts where kernel user/mount namespaces are mandatory; omit it only on CI runners where unprivileged user namespaces are restricted by host policy (the transcript and receipt always record the active isolation mode explicitly).

The latest recorded execution transcript lives at [canary-transcript.md](canary-transcript.md).
