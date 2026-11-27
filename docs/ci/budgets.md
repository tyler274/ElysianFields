# Elysian Fields CI & Deploy Time Budgets

CI and deploy latency is a hard merge gate in Elysian Fields. Any workflow run whose total duration or individual job duration exceeds the machine-enforced limits in [`budgets.json`](budgets.json) fails the `CI result` gate via `scripts/ci_timings.py --check-budgets`.

## Workflow Wall-Clock Budgets

Derived directly from the measured `tyler274/DabneyMoeWebsite` baseline ([`dabney-moe-website-ci-baseline.md`](dabney-moe-website-ci-baseline.md)):

| Metric (`budgets.json` key) | Baseline Median (p90) | Elysian Fields Budget | Reduction vs. Baseline Median | Architectural Mechanism |
| --- | --- | ---: | ---: | --- |
| PR feedback (`pr_feedback_secs`) | 1,368 s / 22.8 min (1,679 s p90) | **360 s (6 min)** | 3.8x faster | Per-target scoping, warm per-target Rust caches, no `wasm-opt` or release OCI builds on PRs, slow device/browser qualification off the routine path |
| Merge to live (`merge_to_live_secs`) | 1,340 s / 22.3 min (1,640 s p90) | **480 s (8 min)** | 2.8x faster | Build release binaries once and reuse uploaded artifacts; separated Nix dependency build roots (`crane.buildDepsOnly`) prevent recompiling dependencies on source edits |
| Client-only merge (`client_only_merge_secs`) | ~1,340 s / 22.3 min (full OCI + Cloud Run) | **240 s (4 min)** | 5.6x faster | Static bundle packaging and deploy for `/elysianfields` directly from `rust-wasm` output without rebuilding any server image or rolling a service revision |

## Per-Job Budgets

| Job (`per_job_budgets` key) | Budget | Scope & Responsibilities |
| --- | ---: | --- |
| `scope` | **45 s** | `scripts/ci_scope.py select` over full Git diff (`fetch-depth: 0`) |
| `lightweight` | **90 s** | Python script unit tests, offline doc/link checks, wall/ledger checks when present |
| `rust-native` | **240 s** | `cargo fmt --check`, host `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --locked --workspace`; builds native release artifact once on `main` |
| `rust-wasm` | **240 s** | Dedicated `wasm32` cache; `cargo clippy --target wasm32-unknown-unknown -- -D warnings`, `cargo build --target wasm32-unknown-unknown`; runs `wasm-opt` on `main` release builds only |
| `cargo-deny` | **90 s** | Advisories, bans, licenses, and sources via `EmbarkStudios/cargo-deny-action@v2` |
| `deploy-client` | **120 s** | Packages static `/elysianfields` bundle on `main` without rebuilding any server image and verifies the subpath asset manifest |

## Enforcement & Testing

- `scripts/ci_timings.py run --budgets docs/ci/budgets.json --check-budgets` runs inside the `result` job of `.github/workflows/ci.yml`.
- Any job exceeding its `per_job_budgets` cap or total run wall clock exceeding the applicable workflow cap (`pr_feedback_secs`, `client_only_merge_secs`, or `merge_to_live_secs`) causes `ci_timings.py` to report the exact overage in `$GITHUB_STEP_SUMMARY` and exit non-zero.
- `CI_BUDGET_TEST_EXTRA_SECS` (or `--test-extra-secs`) adds synthetic elapsed seconds during budget-gate verification tests so breach detection is exercised deterministically without inflating real CI runtime.
