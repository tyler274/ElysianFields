# M6 Stars Evaluation: ElyMalloc and ElyLD

This document records the per-target qualification matrix, measured benchmark and linker evidence, upstream pin configuration, and config-only switch-back procedure for adopting [`tyler274/ElyMalloc`](https://github.com/tyler274/ElyMalloc) and [`tyler274/ElyLD`](https://github.com/tyler274/ElyLD) across Elysian Fields targets.

## Pinned Upstream Revisions

This repository carries no forks of `ElyMalloc` or `ElyLD`. Both repositories are pinned as non-breaking `flake = false` inputs in [`flake.nix`](../../flake.nix) and exposed through the opt-in `nix develop .#m6` shell (`lib.m6Pins`):

| Tool | Upstream Repository | Branch | Pinned Revision | Default CI / Release Impact |
| --- | --- | --- | --- | --- |
| `ElyMalloc` | `github:tyler274/ElyMalloc` | `main3` | `cce11466569646e4e1f7afe9b44c6c80dd613b9f` | None (opt-in via `scripts/m6_toolchain.sh`; default stays on platform allocator) |
| `ElyLD` | `github:tyler274/ElyLD` | `main` | `1b1e263ff40d5eea10966f6d2651f50ce43831ab` | None (opt-in via `scripts/m6_toolchain.sh`; default stays on `lld` / platform linker) |

Workspace `.cargo/config.toml`, `Cargo.toml`, `Cargo.lock`, `deny.toml`, and default CI workflows remain untouched on platform defaults until explicit per-target promotion sign-off.

## Per-Target Qualification Matrix

Targets are evaluated in strict rollout order: **Linux services -> developer builds -> browser Wasm -> Android -> Apple (macOS/iOS) -> Windows**. Compile-only checks never qualify a runtime target.

| Order | Target | ElyMalloc (`cce11466`) | ElyLD (`1b1e263f`) | Disposition & Exact Blocking Gap (when on platform default) |
| --- | --- | --- | --- | --- |
| 1 | **Linux services** (`x86_64-unknown-linux-gnu`: `ef-clinic-server`, `ef-mobile-server`, `ef-migration-rehearsal`) | **Qualified (opt-in)** | **Qualified (opt-in)** | Both pass full single-allocator isolation (`bwrap`), `111/111` core + `16/16` extended chaos tests, `8/8` service acceptance tests, 1-hour soak within M1 RSS/latency ceilings, ThinLTO release builds, and DWARF `addr2line` symbolization. Opt-in via `scripts/m6_toolchain.sh`; default CI/release stays on glibc + `lld` until deployment sign-off. |
| 2 | **Developer builds** (`x86_64-unknown-linux-gnu`) | **Qualified (opt-in)** | **Qualified (opt-in)** | Clean dev relink matches `lld` (`1.365 s` vs `1.320 s`); ThinLTO release symbolization passes. Incremental in-place ELF patching (`15` `.incr` dirs) requires explicit `--gc-mode incremental-no-gc` (`-Wl,--no-gc-sections -Wl,--incremental`) because default `--gc-sections` falls back to a full padded link (`0` `.incr` dirs). |
| 3 | **Browser Wasm** (`wasm32-unknown-unknown`) | **Platform default** | **Platform default** | **Blocking upstream gaps**: `elymalloc-core` and `elymalloc-wasm-smoke` pass `cargo check --target wasm32-unknown-unknown`, and `elyld --features wasm` passes `cargo check`, but Wasm `ElyMalloc` uses `memory.grow` without `munmap` shrinking and disables guard pages/threading, `elyld-wasm` is an experimental non-default feature, and neither has been qualified on Bevy `0.20.1` browser Wasm bundles or browser memory-soak/bundle-size budgets. |
| 4 | **Android** (`aarch64-linux-android`) | **Platform default** | **Platform default** | **Blocking upstream & hardware gaps**: `ElyMalloc` does not list Android/Bionic support in its supported target matrix; `ElyLD` supports Linux ELF AArch64 but Android NDK/Bionic dynamic-linker compatibility is unqualified; no Android device/NDK qualification hardware is attached (`blocked-no-hardware`). |
| 5 | **Apple (macOS / iOS)** (`aarch64-apple-darwin`, `aarch64-apple-ios`) | **Platform default** | **Platform default** | **Blocking upstream & hardware gaps**: `ElyMalloc` checks cleanly for `aarch64-apple-darwin` but iOS is unlisted and Darwin runtime soak/zone integration is unverified; `ElyLD` Mach-O support (`elyld-macho`) is experimental behind `--features macho` and AArch64-only; no Apple macOS/iOS host or device is attached (`blocked-no-hardware`). |
| 6 | **Windows** (`x86_64-pc-windows-msvc`) | **Platform default** | **Platform default** | **Blocking upstream & hardware gaps**: `ElyLD` has **no COFF/PE linker support** (hard upstream gap); `ElyMalloc` checks cleanly for `x86_64-pc-windows-msvc` but CRT/runtime soak is unverified because no Windows host is attached (`blocked-no-hardware`). |

---

## 1. Linux Services

### Single-Allocator Isolation Methodology

Hosts with system-wide allocator preloads (`/etc/ld-nix.so.preload` or `/etc/ld.so.preload`) can inadvertently load two allocator libraries in one process. Every Linux measurement isolates the preload file with `bwrap --dev-bind / / --ro-bind /dev/null <resolved-preload-path>` and verifies `/proc/self/maps`:

- **Pure glibc control**: `0` `libmimalloc` mappings in `/proc/self/maps`.
- **Pinned ElyMalloc (`cce11466`)**: strictly one `libmimalloc.so` mapping pointing to the freshly built `cce11466` library (`132 KiB`, SONAME `libmimalloc.so.3`, always-on `MI_SECURE` mitigations).

### Upstream Allocator Correctness and Chaos Stress

Built from pinned `tyler274/ElyMalloc` (`cce11466569646e4e1f7afe9b44c6c80dd613b9f`) with `rustc 1.99.1`:

- `libmimalloc.so` release build (`cdylib`): `2.23 s`, `132 KiB` stripped shared object, SONAME `libmimalloc.so.3`.
- `cargo test -p elymalloc-core`: **111 passed, 0 failed** (`0.51 s`), covering property tests, sequential heap fuzzing, cross-thread free chaos, fork chaos, guard pages, SIMD twins, and `GlobalAlloc`.
- Extended chaos (`MIMALLOC_CHAOS_STEPS=8192 MIMALLOC_CHAOS_SEED=42 cargo test --release -p elymalloc-core chaos`): **16 passed, 0 failed** (`0.11 s`).
- `elymalloc-alloc-stress` under isolated `LD_PRELOAD` of pinned `libmimalloc.so`: `alloc-stress ok`.

### Service Acceptance Suites

All Linux service acceptance suites (`ef-clinic-server` TCP vertical slice + SSE `Last-Event-ID` + restart durability `2/2`, `ef-mobile-server` independent WAL + `/readyz` `1/1`, `ef-migration-rehearsal` shadow/fence/drain/privacy/468-patient crash-resume `5/5`) were executed across four configurations:

| Configuration | `ef-clinic-server` | `ef-mobile-server` | `ef-migration-rehearsal` | Total |
| --- | --- | --- | --- | --- |
| Pure glibc + `lld` (baseline) | 2/2 PASS | 1/1 PASS | 5/5 PASS | **8/8 PASS** |
| Pinned `ElyMalloc` (`cce11466`) + `lld` | 2/2 PASS | 1/1 PASS | 5/5 PASS | **8/8 PASS** |
| `ElyLD` (`1b1e263f`) dev & incremental (`--no-gc-sections`) | 2/2 PASS | — | 5/5 PASS | **7/7 PASS** |
| `ElyLD` (`1b1e263f`) ThinLTO release (`strip=none`, `debug=line-tables-only`) | 2/2 PASS | 1/1 PASS | 5/5 PASS | **8/8 PASS** |

### Streaming Performance and 1-Hour Soak (`m1_streaming_perf_bench`)

Measured on the M1 stress workload (`80` turns, `41,694` chars, `5,000` lab rows, `900` active stress frames, `300` idle ticks, `144,000`-token / `72,000`-chunk 1-hour soak) under `bwrap` single-allocator isolation against the ceilings in [`docs/m1/decision-review.md`](../m1/decision-review.md):

| Metric | M1 Budget | Pure glibc (baseline) | Pinned `ElyMalloc` (`cce11466`) | `ElyLD` ThinLTO + `ElyMalloc` (`cce11466`) |
| --- | --- | --- | --- | --- |
| Cold init + first frame | `<= 600 ms` | `0.593 ms` | `6.653 ms` | `8.343 ms` |
| Active frame time p50 | — | `1.17 us` | `1.82 us` | `1.77 us` |
| Active frame time p95 | `<= 16.7 ms` | `2.15 us` (`0.0022 ms`) | `18.67 us` (`0.0187 ms`) | `18.44 us` (`0.0184 ms`) |
| Active frame time p99 | `<= 25.0 ms` | `2.93 us` (`0.0029 ms`) | `24.08 us` (`0.0241 ms`) | `23.33 us` (`0.0233 ms`) |
| Active frame time max / stalls `> 50 ms` | `0` stalls | `18.11 us` / `0` | `254.58 us` / `0` | `244.97 us` / `0` |
| Typing latency p95 / p99 / max | `<= 16.7 / 25.0 ms` | `2.29 / 2.52 / 3.13 us` | `19.48 / 19.57 / 20.67 us` | `18.92 / 19.01 / 20.35 us` |
| Citation `[1]` note open max | `<= 50 ms` | `9.21 us` | `84.24 us` | `90.39 us` |
| Citation `[12]` 5,000-row table open max | `<= 100 ms` | `48.85 us` | `836.87 us` | `834.11 us` |
| Idle redraws (`300` ticks) | `0` frames | `0` (`0.261 ms`) | `0` (`0.264 ms`) | `0` (`0.257 ms`) |
| 1-hour soak total (`144k` tokens) | Linear, `0` full reparses | `90.41 ms` (`0` reparses) | `472.40 ms` (`0` reparses) | `453.90 ms` (`0` reparses) |
| Soak `first_10k` -> `last_10k` | Non-degrading | `6.27 ms` -> `6.17 ms` | `34.27 ms` -> `31.31 ms` | `33.11 ms` -> `30.45 ms` |
| Process `VmRSS` before -> mid -> after | `<= 200 MiB`, stable | `19,248 -> 20,016 -> 20,032 KiB` (`19.56 MiB`) | `21,468 -> 22,436 -> 22,436 KiB` (`21.91 MiB`) | `14,632 -> 15,604 -> 15,604 KiB` (`15.24 MiB`) |

**Trade-off summary**: `ElyMalloc` ships with `MI_SECURE` mitigations always enabled (guard pages, encoded free lists, random allocation offsets). Relative to glibc on the same workload, that adds `~2.35 MiB` of guard-page RSS (still `9x` below the `200 MiB` ceiling, and `15.24 MiB` in ThinLTO release with `0 KiB` drift between 30m and 60m) and raises microsecond frame/soak overhead while remaining `~900x` below the `16.7 ms` p95 frame budget.

---

## 2. Developer Builds

Built `elyld` from pinned `tyler274/ElyLD` (`1b1e263ff40d5eea10966f6d2651f50ce43831ab`; reports `GNU ld (ElyLD) 2.44` / `ElyLD 1b1e263ff40d5eea10966f6d2651f50ce43831ab (compatible with GNU linkers)`).

### Link Speed and Incremental `--gc-sections` Trade-off

Measured on single-file relinks of `ef-clinic-server` (after `touch crates/ef-clinic-server/src/lib.rs`) using separate `CARGO_TARGET_DIR` directories:

| Linker Configuration | `.incr` Dirs Created | Cargo Time | Wall Time | Acceptance Parity |
| --- | --- | --- | --- | --- |
| `lld` (baseline) | n/a | `1.12 s` | `1.320 s` | `7/7 PASS` |
| `elyld` clean dev (`--gc-sections` default) | `0` | `1.18 s` | `1.365 s` | `7/7 PASS` |
| `elyld --incremental` with default `--gc-sections` | `0` (falls back to full padded link) | `1.68 s` | `1.854 s` | `7/7 PASS` |
| `elyld --no-gc-sections --incremental` (opt-in) | `15` (true in-place ELF patching) | `1.87 s` (compiled 3 test targets) | `2.051 s` | `7/7 PASS` |

**Explicit choice exposed in `scripts/m6_toolchain.sh`**:
- `--gc-mode gc-sections` (default): retains dead-code elimination for release/CI parity; `ElyLD` cleanly falls back to a full padded link.
- `--gc-mode incremental-no-gc`: passes `-C link-arg=-Wl,--no-gc-sections -C link-arg=-Wl,--incremental` so local iterative builds get true in-place ELF patching when developers opt into larger un-GC'd dev binaries.

### ThinLTO Release Build and Crash/Stack Symbolization

Built `m1_streaming_perf_bench` and `ef-clinic-server` in `--release` (workspace ThinLTO enabled) with `CARGO_PROFILE_RELEASE_STRIP=none` and `CARGO_PROFILE_RELEASE_DEBUG=line-tables-only` linked by `elyld`:

- **Build time**: `45.126 s` wall (`44.77 s` cargo) for both release binaries including Bevy `0.20.1`.
- **ELF `.comment` identity**: `Linker: ElyLD 1b1e263ff40d5eea10966f6d2651f50ce43831ab (compatible with GNU linkers)`.
- **Preserved DWARF sections**: `.debug_abbrev`, `.debug_info`, `.debug_str`, and `.debug_line` verified present via `readelf -S`.
- **`addr2line -f -C` symbolization**:
  - `ef-clinic-server`: `ef_clinic_server::main` resolved to `crates/ef-clinic-server/src/main.rs:4`; `Entry::or_insert_with` resolved to `alloc/src/collections/btree/map/entry.rs:201`.
  - `m1_streaming_perf_bench`: `m1_streaming_perf::benchmark::run_benchmark` resolved to `crates/m1-streaming-perf/src/benchmark.rs:152`; `m1_streaming_perf::plugin::TranscriptSlot` command drop glue resolved to `core/src/ptr/mod.rs:847`.
- **ThinLTO acceptance tests**: `8/8 PASS` (`ef-clinic-server` `2/2`, `ef-mobile-server` `1/1`, `ef-migration-rehearsal` `5/5`).

---

## 3. Browser Wasm

- **Compile checks run**:
  - `cargo check -p elymalloc-core --target wasm32-unknown-unknown` (`0.47 s`, 2 upstream unused warnings) and `cargo check -p elymalloc-wasm-smoke --target wasm32-unknown-unknown` (`0.33 s`) both succeeded.
  - `cargo check --locked -p elyld --features wasm` (`9.38 s`) succeeded.
- **Why Browser Wasm stays on platform default**:
  1. `ElyMalloc` on `wasm32-unknown-unknown` allocates via `memory.grow`, cannot shrink linear memory (`munmap` is a no-op), and disables guard pages and thread-local heaps.
  2. `ElyLD`'s Wasm backend (`elyld-wasm`) is gated behind the non-default experimental `wasm` feature.
  3. Neither has been qualified on the workspace's Bevy `0.20.1` Wasm client bundles, `wasm-bindgen` / `wasm-opt -O2 --strip-debug` output sizes, or browser streaming soak tests.

## 4. Android

- **Why Android stays on platform default**:
  1. `ElyMalloc` (`cce11466`) does not list Android/Bionic in its supported target matrix.
  2. `ElyLD` (`1b1e263f`) targets Linux ELF AArch64, but Android NDK / Bionic dynamic-linker and JNI packaging behavior is unqualified.
  3. No Android NDK toolchain or physical Android device is attached in this environment (`blocked-no-hardware`).

## 5. Apple (macOS and iOS)

- **Compile check run**: `cargo check --locked -p elyld --features macho` (`1.47 s`) succeeded.
- **Why Apple targets stay on platform default**:
  1. `ElyMalloc` (`cce11466`) lists `aarch64-apple-darwin` check support but does not list iOS, and Darwin malloc-zone / runtime soak qualification requires an Apple host.
  2. `ElyLD`'s Mach-O linker (`elyld-macho`) is experimental behind `--features macho`, AArch64-only, and untested on Apple SDK code signing / `dyld` loading.
  3. No macOS or iOS hardware is attached (`blocked-no-hardware`).

## 6. Windows

- **Why Windows stays on platform default**:
  1. `ElyLD` (`1b1e263f`) has **no COFF/PE linker backend** (hard upstream gap).
  2. `ElyMalloc` (`cce11466`) checks cleanly for `x86_64-pc-windows-msvc`, but Windows CRT/runtime soak qualification requires a Windows host (`blocked-no-hardware`).

---

## Config-Only Opt-In and Switch-Back Procedure

[`scripts/m6_toolchain.sh`](../../scripts/m6_toolchain.sh) switches between platform defaults and `ElyMalloc`/`ElyLD` solely through process environment variables (`LD_PRELOAD`, `RUSTFLAGS`, and optional `bwrap` preload isolation), leaving `.cargo/config.toml`, `Cargo.toml`, and `Cargo.lock` completely unmodified:

```sh
# Enter optional M6 shell exposing pinned upstream source paths
nix develop .#m6

# Run a Linux service or test with both ElyMalloc and ElyLD (default --gc-sections)
./scripts/m6_toolchain.sh \
  --mode both \
  --gc-mode gc-sections \
  --elymalloc-lib /path/to/libmimalloc.so \
  --elyld-bin /path/to/elyld \
  --isolate-preload \
  -- cargo test --locked -p ef-clinic-server

# Developer opt-in for ElyLD in-place incremental ELF patching
./scripts/m6_toolchain.sh \
  --mode elyld \
  --gc-mode incremental-no-gc \
  --elyld-bin /path/to/elyld \
  -- cargo test --locked -p ef-clinic-server

# Instant config-only switch-back to platform defaults (clears LD_PRELOAD & RUSTFLAGS)
./scripts/m6_toolchain.sh --mode default --isolate-preload -- cargo test --locked -p ef-clinic-server
```

### Rehearsed Switch-Back Verification Transcript

1. **`--mode both --gc-mode gc-sections --isolate-preload`** running `cargo test --locked -p ef-clinic-server --test vertical_slice_e2e`:
   - Compiled in `6.59 s`; `vertical_slice_e2e` passed **2/2** (`0.61 s`).
   - `readelf -p .comment` confirmed `Linker: ElyLD 1b1e263ff40d5eea10966f6d2651f50ce43831ab (compatible with GNU linkers)`.
2. **`--mode default --isolate-preload`** running the identical command immediately afterward:
   - Compiled in `4.55 s`; `vertical_slice_e2e` passed **2/2** (`0.04 s`).
   - `readelf -p .comment` confirmed `Linker: LLD 23.1.1`.
3. **Zero repository mutation**: `git diff --stat -- .cargo Cargo.toml Cargo.lock` returned empty output.
