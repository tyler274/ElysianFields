# M1 Build and Link Time Qualification Report

This report records empirical clean build, dependency-cached build, 1-line incremental rebuild, link-time, cached dependency disk footprint, and `wasm-opt -Oz` measurements for Elysian Fields (`crates/m1-web-delivery`, linking Bevy `0.20.1` UI/text/asset/window/state/input-focus). Every measurement is compared directly against [`docs/ci/budgets.json`](../ci/budgets.json) and [`docs/ci/budgets.md`](../ci/budgets.md).

## 1. Measurement Environment and Provenance

- **Host OS & Kernel**: Linux `x86_64`, `7.3.3 #1-NixOS SMP PREEMPT_DYNAMIC`
- **CPU**: AMD Ryzen 9 9950X3D (16 physical cores / 32 hardware threads)
- **Memory**: `60 GiB` RAM
- **Rust / LLVM / Linker**:
  - `rustc 1.99.1 (58cf8f9c3 2026-10-13)`
  - `cargo 1.99.1 (5f94df478 2026-08-27)`
  - LLVM `23.1.1`
  - `rust-lld` (`LLD 23.1.1`, compatible with GNU linkers) — already active in `.cargo/config.toml` for Linux native and default for `wasm32-unknown-unknown`
- **Wasm Tooling**: `wasm-bindgen 0.2.130`, Binaryen `wasm-opt 133`, `brotli 1.2.1`
- **Measured Commit**: `3215abef7d004dd4e8065f8470f8ce22b0d18ea5` (`crates/m1-web-delivery`, zero external crates beyond locked Bevy `0.20.1`)
- **Methodology**:
  - Each of the 4 `(target, profile)` matrices ran in a fresh, empty `CARGO_TARGET_DIR`.
  - **Clean build**: `cargo build --locked -p m1-web-delivery [--target wasm32-unknown-unknown] [--release]`.
  - **Deps-cached build**: `cargo clean -p m1-web-delivery` followed by rebuilding `m1-web-delivery` with all compiled third-party crates intact (modeling `crane.buildDepsOnly` / warm CI caches).
  - **1-line edit rebuild**: Single-line string edit in `crates/m1-web-delivery/src/main.rs` followed by `cargo build --locked`.
  - **Link time**: Measured via `RUSTC_BOOTSTRAP=1 cargo rustc --locked -p m1-web-delivery --bin m1-web-delivery ... -- -Z time-passes`, extracting exact `total`, `link_crate`, `link_binary`, and `run_linker` durations from `rustc`.
  - **Load note**: `linux-native-dev` ran while concurrent sibling workspace builds were active on the host; `linux-native-release`, `wasm-dev`, and `wasm-release` ran as host contention subsided. All numbers are reported as directly measured without adjustment.

## 2. Empirical Build, Rebuild, and Link Timings

| Target & Profile | Clean Build | Deps-Cached Build | 1-Line Edit Rebuild | Final-Crate `rustc` Total | `run_linker` (`lld`) | Link Share of Final Crate | Link Share of 1-Line Rebuild | Cached `deps/` Size (`total` target) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| **Linux Native `dev`** (`x86_64-unknown-linux-gnu`) | `92.64 s` | `7.78 s` | `6.02 s` | `2.05 s` | **`1.66 s`** | **`81.1%`** | `27.6%` | `3,133.0 MiB` (`3,953.0 MiB`)\* |
| **Linux Native `release`** (`x86_64-unknown-linux-gnu`) | `59.51 s` | `3.42 s` | `2.72 s` | `0.90 s` | **`0.81 s`** | **`89.7%`** | `29.6%` | `975.7 MiB` (`1,076.8 MiB`)\* |
| **Browser Wasm `dev`** (`wasm32-unknown-unknown`) | `59.90 s` | `7.18 s` | `7.33 s` | `5.75 s` | **`5.58 s`** | **`97.1%`** | **`76.2%`** | `2,500.0 MiB` (`4,306.0 MiB`)\* |
| **Browser Wasm `release`** (`wasm32-unknown-unknown`) | `49.81 s` | `4.48 s` | `4.21 s` | `2.18 s` | **`2.09 s`** | **`95.9%`** | **`49.6%`** | `706.0 MiB` (`961.3 MiB`)\* |

*\*Note on disk footprint*: Running the `-Z time-passes` step with `RUSTC_BOOTSTRAP=1` inside the same `CARGO_TARGET_DIR` stores a second dependency hash alongside the standard build. Single-pass `target/*/deps` footprints on a clean CI cache are approximately half of the two-pass totals above (~`1.5 GiB` native `dev`, ~`500 MiB` native `release`, ~`1.25 GiB` Wasm `dev`, ~`350 MiB` Wasm `release`).

### Wasm Post-Processing (`wasm-bindgen` + `wasm-opt -Oz`)

| Step | Elapsed Time | Input Size | Output Size | Raw Bytes Saved | Brotli-11 Output |
| --- | ---: | ---: | ---: | ---: | ---: |
| `wasm-bindgen 0.2.130 --target web` | `8.78 s` | `35,106,472 B` (`33.48 MiB`) | `31,809,342 B` `.wasm` + `13,425 B` `.js` | `3,297,130 B` (`9.39%`) | `2,230,953 B` (`2.13 MiB`) |
| `wasm-opt -Oz` (on `wasm-bindgen` `.wasm`) | **`238.56 s`** | `31,809,342 B` (`30.34 MiB`) | **`10,042,485 B` (`9.58 MiB`)** | **`21,766,857 B` (`68.43%`)** | **`1,484,906 B` (`1.42 MiB`)** |
| `wasm-opt -Oz` (directly on raw cargo `.wasm`) | **`235.33 s`** | `35,106,472 B` (`33.48 MiB`) | `10,499,306 B` (`10.01 MiB`) | `24,607,166 B` (`70.09%`) | `1,558,143 B` (`1.49 MiB`) |

## 3. Side-by-Side Comparison Against `docs/ci/budgets.json`

| CI Budget Key (`docs/ci/budgets.json`) | Hard Cap | Measured Local Scenario | Measured Elapsed | Budget Verdict |
| --- | ---: | --- | ---: | --- |
| `per_job_budgets.rust-native` | `240 s` | Native `dev` clean / deps-cached / 1-line edit | `92.64 s` / `7.78 s` / `6.02 s` | **`within`** |
| `per_job_budgets.rust-native` | `240 s` | Native `release` clean / deps-cached / 1-line edit | `59.51 s` / `3.42 s` / `2.72 s` | **`within`** |
| `per_job_budgets.rust-wasm` (PR path, no `wasm-opt`) | `240 s` | Wasm `dev` clean / deps-cached / 1-line edit | `59.90 s` / `7.18 s` / `7.33 s` | **`within`** |
| `per_job_budgets.rust-wasm` (PR path, no `wasm-opt`) | `240 s` | Wasm `release` clean / deps-cached (no `wasm-opt`) | `49.81 s` / `4.48 s` | **`within`** |
| `per_job_budgets.rust-wasm` (`main` release + unstripped `wasm-opt -Oz`) | `240 s` | Wasm `release` deps-cached (`4.48 s`) + `wasm-bindgen` (`8.78 s`) + `wasm-opt -Oz` (`238.56 s`) | **`251.82 s`** | **`over`** (`+11.82 s` over cap) |
| `pr_feedback_secs` | `360 s` | Full local `dev-check.sh --tier full` (warm cache; fmt + python + host/wasm clippy + tests + docs) | ~`20–30 s` (cold native+wasm `<= 153 s`) | **`within`** |
| `client_only_merge_secs` (`main` release + unstripped `wasm-opt -Oz`) | `240 s` | Wasm `release` deps-cached + `wasm-bindgen` + `wasm-opt -Oz` (`251.82 s`) + static deploy packaging | **`> 251.82 s`** | **`over`** (driven entirely by `238.56 s` `wasm-opt -Oz`) |
| `merge_to_live_secs` | `480 s` | Parallel native release (`59.51 s` clean / `3.42 s` warm) + Wasm release (`251.82 s` warm with `wasm-opt -Oz`) + packaging | `~252–312 s` (warm/cold) | **`within`** (tight if GitHub Actions runner CPU is slower than 16-core local host) |

### Key CI Bottleneck Identified: Unstripped `wasm-opt -Oz`

The single budget breach found in M1 is **`wasm-opt -Oz` taking `238.56 s`** on the default unstripped `31.8 MiB` release `.wasm` binary. Because PR CI skips `wasm-opt` (`docs/ci/budgets.md`), PR feedback stays deeply `within` budget (`< 10 s` warm, `< 60 s` clean for Wasm). On `main` client-only merges, however, `238.56 s` alone breaches both `rust-wasm` (`240 s`) and `client_only_merge_secs` (`240 s`).

**Required M2 mitigation**:
1. Add `strip = "symbols"` (or `debug = false`) for `wasm32-unknown-unknown` release profiles so `wasm-opt` does not parse tens of megabytes of debug/name sections.
2. Benchmark `wasm-opt -Os` / `-O2` (or `--low-memory-unused --strip-debug`) against `-Oz` on the stripped binary so optimization completes in `< 30 s` while preserving Brotli-11 sizes under the `<= 3.5 MiB` / `<= 2.5 MiB` caps.

## 4. Link-Time Analysis and Linker Recommendation

### Is link time the largest part of a rebuild?

**Yes — unequivocally across every profile.**

- In `rustc`'s final-crate compile+link pass (`-Z time-passes`), `run_linker` accounts for:
  - **`81.1%`** on Linux native `dev` (`1.66 s` of `2.05 s`)
  - **`89.7%`** on Linux native `release` (`0.81 s` of `0.90 s`)
  - **`97.1%`** on Browser Wasm `dev` (`5.58 s` of `5.75 s`)
  - **`95.9%`** on Browser Wasm `release` (`2.09 s` of `2.18 s`)
- Even including Cargo orchestration and library+binary rebuild overhead (`one_line_rebuild_secs`), `run_linker` alone represents **`76.2%`** of a Wasm `dev` 1-line edit (`5.58 s` of `7.33 s`) and **`49.6%`** of a Wasm `release` 1-line edit (`2.09 s` of `4.21 s`).

### Is a faster linker (`lld` / `mold`) needed before M6 stretch?

**No.** `rust-lld` (`LLD 23.1.1`) is already active in `.cargo/config.toml` for Linux native builds and is the built-in linker for `wasm32-unknown-unknown`:

- Native Linux 1-line rebuilds already complete in **`6.02 s` (`dev`) / `2.72 s` (`release`)**, with `run_linker` at **`1.66 s` / `0.81 s`**.
- `mold` is available on NixOS/Linux hosts as an optional local-developer optimization for Linux ELF binaries, but it cannot link `wasm32-unknown-unknown` (which stays on `rust-lld`).
- Keeping `lld` as the workspace default through M2–M5 avoids linker divergence between local builds and Linux CI while leaving ample headroom under every native CI budget (`240 s`). Re-evaluate `mold` or Bevy dynamic linking (`dynamic_linking` feature for local dev only) only if full-app native `dev` link times grow beyond ~`5–8 s` once `bevy_render`/`wgpu` and full application crates are added.

## 5. Cross-OS Target Build Matrix

| Target Platform | Architecture / Triple | Local Linux Host Status | M2 Verification Path |
| --- | --- | --- | --- |
| **Linux Desktop (`desktop-linux`)** | `x86_64-unknown-linux-gnu` | **Measured (`within` all budgets)** | Local Nix + GitHub Actions `ubuntu-latest` (`rust-native` job) |
| **Browser Wasm (`browser-wasm-*`)** | `wasm32-unknown-unknown` | **Measured (PR `within`; `main` unstripped `wasm-opt -Oz` `over`)** | Local Nix + GitHub Actions `ubuntu-latest` (`rust-wasm` job) after M2 release stripping |
| **macOS Desktop (`desktop-macos`)** | `aarch64-apple-darwin`, `x86_64-apple-darwin` | **`blocked-no-hardware`** | GitHub Actions `macos-14` / `macos-15` Apple Silicon runners in M2 (Metal + Darwin `ld64`/`lld`) |
| **Windows Desktop (`desktop-windows`)** | `x86_64-pc-windows-msvc` | **`blocked-no-hardware`** | GitHub Actions `windows-latest` MSVC runners in M2 (DX12 + `lld-link` evaluation) |
| **Android Native (`android-native`)** | `aarch64-linux-android`, `x86_64-linux-android` | **`blocked-no-hardware`** | Linux NDK cross-compile CI job + Android API 34–35 emulator / physical ARM64 device in M2 |
| **iOS Native (`ios-native`)** | `aarch64-apple-ios`, `aarch64-apple-ios-sim` | **`blocked-no-hardware`** | GitHub Actions `macos-15` Xcode runner + physical iPhone TestFlight qualification in M2 |
