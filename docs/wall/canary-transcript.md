# Elysian Realm Wall Canary Transcript

Recorded live execution of `scripts/wall_canary.py` against a clean clone of `HEAD`.

## Run Metadata

| Field | Value |
| --- | --- |
| Verdict | **PASS** |
| Timestamp (UTC) | `2026-11-27T13:25:53+00:00` -> `2026-11-27T13:25:54+00:00` |
| Elysian Realm `HEAD` | `e08a33d517f7b28677bb6024f3239ca652500149` |
| Reference SHA Checked | `01b762838d639a51f82e9d7d15e788e02bac772f` |
| Isolation Mode | `linux-user-mount-ns` (unshare --user --map-root-user --mount + tmpfs mask verified) |
| Rust Toolchain | `rustc 1.99.1 (58cf8f9c3 2026-10-13)` / `clippy 0.1.99 (58cf8f9c35 2026-10-13)` |

## Probe Summary

| # | Probe | Expected | Verdict | Summary |
| --- | --- | --- | --- | --- |
| 1 | `reference_file_locate` | FAIL (all reference paths blocked) | **PASS** | All 6 reference-locate attempts blocked |
| 2 | `reference_git_history` | ZERO matches (no shared history, alternates, or reference remotes) | **PASS** | Zero matches for 01b762838 across 1 realm commit(s); standalone .git verified |
| 3 | `linear_github_connectors` | FAIL (credentials, config files, and gh/linear CLIs absent/denied) | **PASS** | All Linear/GitHub env vars, credential files, and CLIs absent/denied |
| 4 | `trivial_crate_build_lint` | PASS (cargo clippy -D warnings and cargo test succeed) | **PASS** | cargo clippy --all-targets -- -D warnings and cargo test passed inside isolated realm |

## Probe Details

### Probe 1: `reference_file_locate` (PASS)

- **Expected**: FAIL (all reference paths blocked)
- **Observed**: All 6 reference-locate attempts blocked

| Target | Kind | Blocked | Observation |
| --- | --- | --- | --- |
| `synthetic-outside-realm-sentinel` | `file` | `True` | BLOCKED (FileNotFoundError: No such file or directory) |
| `reference-repo-dabney-moe` | `dir` | `True` | BLOCKED (FileNotFoundError: No such file or directory) |
| `reference-repo-coda` | `dir` | `True` | BLOCKED (FileNotFoundError: No such file or directory) |
| `reference-repo-website` | `dir` | `True` | BLOCKED (FileNotFoundError: No such file or directory) |
| `host-agent-store-gemini` | `dir` | `True` | BLOCKED (FileNotFoundError: No such file or directory) |
| `host-gh-config` | `dir` | `True` | BLOCKED (FileNotFoundError: No such file or directory) |

### Probe 2: `reference_git_history` (PASS)

- **Expected**: ZERO matches (no shared history, alternates, or reference remotes)
- **Observed**: Zero matches for 01b762838 across 1 realm commit(s); standalone .git verified
- **Standalone `.git` directory**: `True`
- **Alternates file exists**: `False`
- **Realm commits scanned**: `1`
- **Reference object present**: `False`
- **Remotes**: `origin	https://github.com/tyler274/ElysianFields.git (fetch)`, `origin	https://github.com/tyler274/ElysianFields.git (push)`

### Probe 3: `linear_github_connectors` (PASS)

- **Expected**: FAIL (credentials, config files, and gh/linear CLIs absent/denied)
- **Observed**: All Linear/GitHub env vars, credential files, and CLIs absent/denied
- **Leaked credential env vars**: `[]` (checked 10 forbidden variables)
- **Leaked credential files**: `[]`
- **CLI `gh`**: `DENIED (FileNotFoundError)`
- **CLI `linear`**: `DENIED (FileNotFoundError)`

### Probe 4: `trivial_crate_build_lint` (PASS)

- **Expected**: PASS (cargo clippy -D warnings and cargo test succeed)
- **Observed**: cargo clippy --all-targets -- -D warnings and cargo test passed inside isolated realm
- **`cargo clippy --all-targets -- -D warnings` exit code**: `0`
- **`cargo test` exit code**: `0`

```text
running 2 tests
test tests::blocked_status_halts_progress ... ok
test tests::ready_status_allows_progress ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```
