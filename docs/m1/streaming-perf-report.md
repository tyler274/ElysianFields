# M1 Streaming Performance Feasibility Report

This report documents the empirical measurements, architectural invariants, and budget evaluation for the M1 Bevy `0.20.1` clinical streaming prototype in [`crates/m1-streaming-perf`](../../crates/m1-streaming-perf/Cargo.toml) against [`specs/m1-feasibility-budgets.md`](../../specs/m1-feasibility-budgets.md) and [`specs/m1-platforms.md`](../../specs/m1-platforms.md).

All workloads are 100% synthetic (zero protected health information and zero real clinical messages).

## 1. Measurement Environment and Scope Disclosure

- **Prototype crate**: `crates/m1-streaming-perf` (Bevy `0.20.1`, headless-compatible ECS + `bevy_ui` + `bevy_text`, zero additional Cargo dependencies beyond the workspace `Cargo.lock`).
- **Benchmark command**: `cargo run --release -p m1-streaming-perf --bin m1_streaming_perf_bench`
- **Host hardware**: Linux `x86_64` (`7.3.3 #1-NixOS`), AMD Ryzen 9 9950X3D (16 cores / 32 threads), 60 GiB RAM, `rustc 1.99.1`, `cargo 1.99.1`.
- **Scope and honesty notice**:
  - This host is significantly faster than the low/midrange qualification hardware tiers in [`specs/m1-platforms.md`](../../specs/m1-platforms.md) (e.g., Intel i5-10210U / UHD 620, MacBook Air M1, Snapdragon 6 Gen 1, iPhone 12/SE).
  - The executable harness runs Bevy `0.20.1` ECS schedules, `Node`/`Text` component updates, virtualization slot synchronization, and domain state transitions headlessly on CPU without a live Vulkan/Metal/DX12/WebGL2 GPU swapchain or browser DOM/compositor loop.
  - Consequently, GPU rasterization, browser 4x CPU-throttled WebGL2 frames, Wasm Brotli-11 bundle size, and all physical macOS/Windows/Android/iOS/Safari measurements remain explicitly marked **`blocked-no-hardware`** until exercised on real target devices and live browser harnesses.

## 2. Repeatable Stutter Workload Verification

The deterministic synthetic generator (`SyntheticClinicalWorkload::generate` and `SyntheticTokenStream`) matches every dimension of Section 1 of [`specs/m1-feasibility-budgets.md`](../../specs/m1-feasibility-budgets.md):

| Workload Dimension | Specification Requirement | Measured Prototype Configuration |
| --- | --- | --- |
| Conversation length | 80 turns (40 clinician + 40 assistant) | **80 turns** (40 clinician + 40 assistant) |
| Transcript volume | ~40,000 characters | **41,694 characters** across rich AST blocks |
| Rich text blocks | Headings, lists, tables, code blocks, links, citations `[1]`–`[12]` | `Heading`, `Paragraph`, `List` (bulleted & numbered), `Table`, `CodeBlock` (JSON), `Link`, all 12 citations `[1]`–`[12]` |
| Streaming cadence | 40 tokens/sec (~160–200 chars/sec), chunk arrival >= 4 Hz | **40 tokens/sec** (20 Hz chunk arrival, 2 tokens/chunk; 600 tokens in 15 s active window) |
| Concurrent typing | 6 keys/sec multiline composer typing | **6 keys/sec** (90 keystrokes across 900 frames @ 60 Hz) |
| Concurrent scrolling | ±420 px/sec alternating up/down every 1 s | **±420 px/sec** (±7 px/frame @ 60 Hz, reversing every 60 frames) + simultaneous lab table scroll + periodic viewport resize (`720 px` <-> `420 px`) |
| Citation inspection | Non-modal switch between progress note `[1]` and 5,000-row lab table `[12]` | **5,000 structured lab rows** (`[12]`) + 11 multi-section progress notes (`[1]`–`[11]`), switched repeatedly during active streaming |
| Idle phase | 5–10 s with zero stream events and zero input | **300 idle ticks** (5 s @ 60 Hz) immediately following active window |
| Long-session stability | Sustained 1-hour streaming memory stability | **144,000 tokens** across 72,000 chunks (3,600 s @ 40 tok/s accelerated) with Linux `/proc/self/status` `VmRSS` sampling |

## 3. Empirical Benchmark Results (3 `--release` Trials)

Three consecutive `--release` runs of `m1_streaming_perf_bench` on `x86_64-unknown-linux-gnu`:

| Measurement | Trial 1 | Trial 2 | Trial 3 | Median |
| --- | ---: | ---: | ---: | ---: |
| Cold init + first virtualized frame | 9.074 ms | 10.688 ms | 8.984 ms | **9.074 ms** |
| Active frame time p50 (n = 900) | 2.85 us (0.0029 ms) | 3.02 us (0.0030 ms) | 2.94 us (0.0029 ms) | **2.94 us (0.0029 ms)** |
| Active frame time p95 (n = 900) | 24.03 us (0.0240 ms) | 24.26 us (0.0243 ms) | 25.00 us (0.0250 ms) | **24.26 us (0.0243 ms)** |
| Active frame time p99 (n = 900) | 30.78 us (0.0308 ms) | 30.90 us (0.0309 ms) | 31.16 us (0.0312 ms) | **30.90 us (0.0309 ms)** |
| Active frame time max (n = 900) | 347.39 us (0.3474 ms) | 347.26 us (0.3473 ms) | 311.15 us (0.3112 ms) | **347.26 us (0.3473 ms)** |
| Active stalls `> 50 ms` (15 s window) | 0 | 0 | 0 | **0** |
| Typing latency p50 (n = 90) | 2.98 us (0.0030 ms) | 3.22 us (0.0032 ms) | 3.13 us (0.0031 ms) | **3.13 us (0.0031 ms)** |
| Typing latency p95 (n = 90) | 24.27 us (0.0243 ms) | 25.23 us (0.0252 ms) | 24.93 us (0.0249 ms) | **24.93 us (0.0249 ms)** |
| Typing latency p99 (n = 90) | 24.93 us (0.0249 ms) | 26.03 us (0.0260 ms) | 26.59 us (0.0266 ms) | **26.03 us (0.0260 ms)** |
| Typing latency max (n = 90) | 25.99 us (0.0260 ms) | 27.93 us (0.0279 ms) | 29.92 us (0.0299 ms) | **27.93 us (0.0279 ms)** |
| Citation `[1]` note open p50 / p95 / max | 1.81 / 16.38 / 150.64 us | 1.69 / 23.11 / 121.57 us | 1.76 / 14.10 / 122.16 us | **1.76 / 16.38 / 122.16 us (0.122 ms max)** |
| Citation `[12]` 5,000-row table open p50 / p95 / max | 5.77 / 11.79 / 1,063.91 us | 5.54 / 12.67 / 1,087.96 us | 5.60 / 8.51 / 1,075.31 us | **5.60 / 11.79 / 1,075.31 us (1.075 ms max)** |
| Materialized ECS entities (transcript / lab / total) | 29 / 35 / 66 | 29 / 35 / 66 | 29 / 35 / 66 | **29 / 35 / 66** |
| Idle ticks / frames rendered / dirty requests | 300 / 0 / 0 | 300 / 0 / 0 | 300 / 0 / 0 | **300 / 0 / 0** |
| Snapshot restore (scroll / citation / composer / resumed tokens) | true / true / true / 10 | true / true / true / 10 | true / true / true / 10 | **Exact restore, 0 dropped** |
| 1-hour soak total time (144,000 tokens, 72,000 chunks) | 716.21 ms | 620.39 ms | 559.44 ms | **620.39 ms** |
| 1-hour soak O(1) stability (first 10k vs last 10k tokens) | 48.07 ms vs 84.46 ms | 46.56 ms vs 43.62 ms | 45.96 ms vs 41.79 ms | **46.56 ms vs 43.62 ms** |
| 1-hour soak full transcript reparses | 0 | 0 | 0 | **0** |
| 1-hour soak `VmRSS` before / mid (30 min) / after (60 min) | 22,164 / 23,132 / 23,132 KiB | 21,700 / 22,668 / 22,668 KiB | 22,004 / 22,972 / 22,972 KiB | **22,004 / 22,972 / 22,972 KiB (22.43 MiB peak)** |
| 1-hour soak `VmRSS` delta (mid -> after) | 0 KiB | 0 KiB | 0 KiB | **0 KiB (flat)** |

## 4. Comparison Against Every M1 Feasibility Budget

Status legend:
- **`WITHIN BUDGET`**: Measured empirically on Linux headless Bevy `0.20.1` (`--release`) inside the stated ceiling; GPU/low-tier hardware qualification remains tracked in Section 6.
- **`OVER BUDGET`**: Measured value exceeds the hard budget.
- **`blocked-no-hardware`**: Target platform, GPU driver, or physical device is unavailable in this environment (never reported as passing).

| Budget Metric (`specs/m1-feasibility-budgets.md`) | Native Linux Desktop Budget | Measured (Linux Headless `--release` Median) | Native Linux Status | Browser Wasm (Desktop / Mobile Web) | Native Desktop (macOS / Windows) | Native Mobile (Android / iOS) | Chrome Extension MV3 Popup |
| --- | --- | --- | --- | --- | --- | --- | --- |
| **Cold start (launch/nav -> first render)** | `<= 600 ms` | **9.074 ms** (App init + 80-turn load + first ECS sync; excludes GPU swapchain init) | **WITHIN BUDGET** *(CPU/ECS)* | `blocked-no-hardware` (`<= 1,500 ms` / `<= 2,200 ms`) | `blocked-no-hardware` (`<= 600 ms`) | `blocked-no-hardware` (`<= 800 ms`) | `blocked-no-hardware` (`<= 250 ms`) |
| **Time to first usable interaction** | `<= 750 ms` | **9.074 ms** (composer + citation panel interactive after first update) | **WITHIN BUDGET** *(CPU/ECS)* | `blocked-no-hardware` (`<= 1,800 ms` / `<= 2,500 ms`) | `blocked-no-hardware` (`<= 750 ms`) | `blocked-no-hardware` (`<= 1,000 ms`) | `blocked-no-hardware` (`<= 300 ms`) |
| **Typing input latency (p95 / p99)** | `<= 16.7 ms` / `<= 25 ms` | **0.0249 ms** p95 / **0.0260 ms** p99 (**0.0279 ms** max) | **WITHIN BUDGET** *(CPU/ECS)* | `blocked-no-hardware` (`<= 35 ms` / `<= 50 ms`) | `blocked-no-hardware` (`<= 16.7 ms` / `<= 25 ms`) | `blocked-no-hardware` (`<= 25 ms` / `<= 40 ms`) | `blocked-no-hardware` (`<= 25 ms` / `<= 40 ms`) |
| **Frame time (p95 / p99)** | `<= 16.7 ms` / `<= 25.0 ms` | **0.0243 ms** p95 / **0.0309 ms** p99 (**0.3473 ms** max) | **WITHIN BUDGET** *(CPU/ECS)* | `blocked-no-hardware` (`<= 16.7 ms` / `<= 33.3 ms`) | `blocked-no-hardware` (`<= 16.7 ms` / `<= 25.0 ms`) | `blocked-no-hardware` (`<= 16.7 ms` / `<= 33.3 ms`) | `blocked-no-hardware` (`<= 16.7 ms` / `<= 25.0 ms`) |
| **Long tasks / stalls `> 50 ms` (per 15 s window)** | `0` stalls `> 50 ms` | **0 stalls** (max frame `0.3473 ms`, > 140x below 50 ms threshold) | **WITHIN BUDGET** *(CPU/ECS)* | `blocked-no-hardware` (`0` tasks, `0 ms` TBT) | `blocked-no-hardware` (`0` stalls) | `blocked-no-hardware` (`0` stalls) | `blocked-no-hardware` (`0` tasks) |
| **Citation panel open (`[1]` note / `[12]` 5,000-row table)** | `<= 50 ms` / `<= 100 ms` | **0.0164 ms** p95 (`0.122 ms` cold max) / **0.0118 ms** p95 (`1.075 ms` cold max) | **WITHIN BUDGET** *(CPU/ECS)* | `blocked-no-hardware` (`<= 100 ms` / `<= 150 ms`) | `blocked-no-hardware` (`<= 50 ms` / `<= 100 ms`) | `blocked-no-hardware` (`<= 80 ms` / `<= 120 ms`) | N/A |
| **Memory ceiling (RSS / heap)** | `<= 200 MiB` RSS | **22.43 MiB** peak `VmRSS` (`22.59 MiB` max across trials; **0 KiB** growth 30 min -> 60 min) | **WITHIN BUDGET** *(CPU/ECS)* | `blocked-no-hardware` (`<= 250 MiB` / `<= 150 MiB`) | `blocked-no-hardware` (`<= 200 MiB`) | `blocked-no-hardware` (`<= 150 MiB`) | `blocked-no-hardware` (`<= 64 MiB`) |
| **Idle CPU & redraw rate** | `0 frames/sec`, `< 1%` CPU | **0 frames rendered**, **0 dirty frames requested** over 300 idle ticks (`0.435 ms` total CPU over 5 s simulated = `< 0.01%`) | **WITHIN BUDGET** | `blocked-no-hardware` *(policy compiles for `wasm32`, live GPU submission unverified)* | `blocked-no-hardware` | `blocked-no-hardware` | `blocked-no-hardware` |
| **Wasm compressed download size (Brotli-11)** | N/A | Library compiles cleanly to `wasm32-unknown-unknown`; final `.wasm` + JS glue bundle size is measured by the dedicated web bundle prototype | N/A | `blocked-no-hardware` (`<= 3.5 MiB` / `<= 2.5 MiB`) | N/A | N/A | `blocked-no-hardware` (`<= 1.0 MiB`) |

### Build and CI Timing Measurements

| Target Build Check | Budget Reference (`specs/m1-feasibility-budgets.md`) | Measured Wall Time | Status |
| --- | --- | ---: | --- |
| Clean host `--release` build (`m1_streaming_perf_bench`) | `pr_feedback_secs <= 360 s` | **69.7 s** | **WITHIN BUDGET** |
| Incremental host `--release` rebuild (edit `plugin.rs`) | `client_only_merge_secs <= 240 s` | **9.8 s** (11.4 s incl. lock wait) | **WITHIN BUDGET** |
| Clean `wasm32-unknown-unknown` `--release --lib` build | `pr_feedback_secs <= 360 s` | **57.4 s** | **WITHIN BUDGET** |
| Workspace tests (`cargo test -p m1-streaming-perf`) | `pr_feedback_secs <= 360 s` | **0.09 s** (5/5 tests) | **WITHIN BUDGET** |

## 5. Why This Architecture Eliminates the Streaming Stutter Scenario

Long-transcript streaming stutter arises from four compounding costs in naive UI clients: re-parsing full markdown history on every token chunk, laying out `O(total_rows)` DOM/ECS nodes for transcripts and tables, running continuous `requestAnimationFrame` loops when idle, and allowing unbounded heap/entity accumulation over hour-long clinical sessions. The prototype eliminates all four structurally:

1. **O(1) Incremental Delta-Append Streaming (`StreamingPrototypeStore::apply_stream_chunk`)**:
   - Every arriving `StreamChunk` carries a strict monotonic `StreamSeq`. Out-of-order chunks return `StreamingPerfError::OutOfOrderStreamChunk` rather than corrupting state.
   - Text deltas append directly onto the active tail `MarkdownBlock::Paragraph` buffer without touching or re-parsing any earlier turn (`full_reparse_count == 0` across all 72,000 chunks).
   - Once the active tail paragraph reaches `STREAMING_PARAGRAPH_ROLLOVER_CHARS` (`420` characters), the engine rolls over to a fresh tail block in O(1), bounding per-block text layout and formatting cost permanently.
   - Only the single mutated `BlockId` is pushed to `dirty_blocks`. As a result, throughput for the last 10,000 tokens of the 1-hour soak (`43.62 ms`) matches the first 10,000 tokens (`46.56 ms`) with zero `O(n)` slowdown.

2. **Single-Owner O(viewport) Virtualization (`VirtualizedWindow` + `VirtualizedEcsPoolState`)**:
   - Both the 80-turn transcript and the 5,000-row clinical lab table (`[12]`) delegate visible-slice calculation to `VirtualizedWindow::visible_range(total_rows)`.
   - At a `720 px` viewport with `28 px` lab rows and `4`-row overscan, Bevy ECS spawns a reusable pool of **35 `LabTableRowSlot` entities** instead of 5,000 (**99.3% fewer entities**), and **29 `TranscriptSlot` entities** for the transcript.
   - Scrolling across all 5,000 rows rebinds the same 35 pooled entities in place using zero-allocation `ClinicalLabRow::write_row_text` buffer reuse, producing sub-millisecond cold open (`1.075 ms`) and `11.8 us` p95 switch times.

3. **Reactive Zero-Redraw Idle Policy (`IdleRedrawPolicy`)**:
   - Every state mutation at the domain boundary calls `request_redraw(RedrawReason)` where multiple events in one tick coalesce into a single dirty request.
   - `reactive_sync_ui_system` calls `evaluate_tick()` before inspecting any ECS queries; on `RedrawDecision::Idle` it exits immediately.
   - Across 300 consecutive idle ticks (5 simulated seconds at 60 Hz), `frames_rendered == 0` and `dirty_frames_requested == 0`, proving the event-driven zero-idle-redraw contract required to stay below `1%` idle CPU and preserve laptop/mobile battery life.

4. **Bounded Memory Stability and Background/Return Resumption**:
   - Retained transcript blocks live in a bounded ring (`DEFAULT_MAX_RETAINED_BLOCKS = 1,024`) and recent delta diagnostics live in a fixed `256`-entry ring (`RECENT_DELTA_RING_CAPACITY`).
   - Across the accelerated 1-hour soak (144,000 tokens), total materialized ECS entities remain fixed at **66**, and Linux `VmRSS` stabilizes at **22.43 MiB** with **0 KiB** additional resident memory allocated between minute 30 and minute 60.
   - `capture_snapshot` and `restore_snapshot` preserve exact transcript scroll offset, non-modal citation panel scroll offset (`[12]`), composer draft, and `StreamSeq`, resuming live chunks through the same `apply_stream_chunk` owner with zero dropped or duplicated tokens.

## 6. Target Platform Qualification Status (`specs/m1-platforms.md`)

| Ledger Platform Key | Target Platform | Streaming Perf Prototype Status | Notes |
| --- | --- | --- | --- |
| `desktop-linux` | Linux Desktop | **WITHIN BUDGET *(headless CPU/ECS)***; GPU/low-tier hardware `blocked-no-hardware` | Headless Bevy `0.20.1` CPU/ECS, virtualization, idle policy, and 1-hour RSS soak verified on Linux x86_64; Wayland/Vulkan GPU frame timing on Intel i5-10210U / 8 GB RAM still required before M1 exit. |
| `desktop-macos` | macOS Desktop | `blocked-no-hardware` | Requires physical Apple Silicon M1 Air (Metal + VoiceOver). |
| `desktop-windows` | Windows Desktop | `blocked-no-hardware` | Requires physical Windows 11 x86_64 laptop (DX12/Vulkan). |
| `browser-wasm-desktop` | Browser Wasm — Desktop | `blocked-no-hardware` | Crate compiles cleanly to `wasm32-unknown-unknown`; requires live Chromium/Firefox/Safari WebGL2 4x-throttled frame capture and Brotli-11 bundle measurement. |
| `browser-wasm-mobile` | Browser Wasm — Mobile Web | `blocked-no-hardware` | Requires physical Android Chrome and iOS Safari devices. |
| `android-native` | Android Native | `blocked-no-hardware` | Requires physical ARM64 Android 14 device (Vulkan 1.1 / GLES 3.0). |
| `ios-native` | iOS Native | `blocked-no-hardware` | Requires physical ARM64 iOS 17/18 device (Metal). |
| `chrome-extension-popup` | Chrome Extension MV3 Popup | `blocked-no-hardware` | Requires live Chromium MV3 popup cold-open and `<= 1.0 MiB` Brotli-11 check. |
