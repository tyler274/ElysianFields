# Elysian Fields Stutter Scenario Specification & Reference Baseline

Observer-side performance baseline and M1 performance budget contract anchored to pinned `dabney-moe` reference `01b762838d639a51f82e9d7d15e788e02bac772f`.

## 1. Agreed Stutter Scenario Specification

The motivating user-visible regression in the reference suite is **UI stutter and input lag during long streaming LLM chat transcripts while concurrently typing, scrolling, and inspecting clinical citations** in **Curie Clinic (browser)** and **Curie Mobile**.

### 1.1 Synthetic Workload Dimensions (Repeatable Contract)

All measurements use 100% synthetic clinical data (zero PHI, zero `eval/` data):

| Dimension | Specification |
| --- | --- |
| **Conversation length** | **80 turns** (40 physician prompts + 40 rich clinical assistant responses) |
| **Transcript volume** | **~40,000 characters** (~39.4 KB rendered markdown + structured citation segments) |
| **Streaming cadence** | **40 tokens/sec** (~160–200 chars/sec) sustained over **120 seconds** (or a 15-second high-resolution capture window for automated CI/local probes, pushing chunk updates at ≥ 4 Hz) |
| **Rich text structure** | Markdown headings (`##`/`###`), bullet and numbered lists, multi-column clinical lab tables, fenced JSON/code blocks, and **12 distinct inline citation markers** (`[1]`–`[12]`) per active thread |
| **Concurrent composer typing** | Continuous typing into the multiline message composer at **6 keys/sec** (~166 ms cadence) throughout active streaming |
| **Concurrent viewport scrolling** | Smooth vertical wheel/touch scrolling (±420 px/sec alternating up/down across the 80-turn transcript) every 1 second while streaming and typing |
| **Concurrent citation inspection** | Opening and switching the non-modal cited-source reading panel between a standard multi-section progress note citation (`[1]`) and a **5,000-row structured clinical lab/record table** (`[12]`) |
| **Idle phase** | 5–10 seconds with zero streaming events and zero user input after the active stress window |

### 1.2 Why the Reference Implementation Stutters (Observed Architecture)

Inspection of `crates/physician-app/src/lib.rs` (`L2399-2440`) and `crates/physician-app/src/shell.rs` (`event_refetch`) reveals the root cause of reference stutter on long transcripts:

1. Every SSE `chat_changed` event increments `workspace_attempt`, `chat_attempt`, and `recent_attempt`, triggering a **full HTTP refetch of `/patient-workspaces/{id}/chats/{chat}/snapshot`** (observed: **62 full snapshot refetches in 15 seconds**).
2. Each refetch deserializes the entire 80-turn `ChatSnapshot` JSON payload on the browser main thread, re-parses markdown (`pulldown-cmark`), and diffs ~6,460 live DOM elements (`~11,400–16,300` CDP nodes), driving **416–431 full browser layouts** and **971–1,382 style recalculations** in 15 seconds.
3. Furthermore, `physician-app` bundles `physician-demo-db` (`turso_core` + `icu_collator_data`) directly into the browser Wasm binary, producing a **26.33 MB uncompressed** (`4.17 MB` Brotli-11) Wasm download.

## 2. Reference Baseline Measurements (`01b762838`)

### 2.1 Measurement Environment

| Property | Observed Value |
| --- | --- |
| **Reference SHA** | `01b762838d639a51f82e9d7d15e788e02bac772f` |
| **Host OS / Kernel** | Linux `7.3.3` NixOS `x86_64` |
| **Host CPU** | AMD Ryzen 9 9950X3D 16-Core / 32-Thread (`x86_64`) |
| **Host RAM** | 60 GiB DDR5 (available during run: ~5.9 GiB free/buff) |
| **Rust Toolchain** | `rustc 1.99.1 (58cf8f9c3 2026-10-13)`, LLVM `23.1.1`, `wasm-bindgen 0.2.129` |
| **Browser** | Headless Chromium `155.0.8059.39` (from `nix develop .#agent`) via Playwright CDP |
| **Build Profile** | `cargo build --locked --offline --release --target wasm32-unknown-unknown` (56.49 s clean build; no `wasm-opt` pass, matching reference `nix/packages.nix`) |

### 2.2 Compiled Reference Wasm Bundle Sizes (`01b762838`)

| Client Target | Raw `.wasm` | Gzip-9 `.wasm` | Brotli-11 `.wasm` | JS Glue (Raw / Brotli-11) | Total Brotli-11 (`.wasm` + `.js`) |
| --- | ---: | ---: | ---: | ---: | ---: |
| **Curie Clinic (`physician-app`)** | 26,328,996 B (25.11 MiB) | 6,692,525 B (6.38 MiB) | 4,171,021 B (3.98 MiB) | 95,731 B / 11,933 B | **4,182,954 B (3.99 MiB)** |
| **Curie Mobile Web (`mobile-app`)** | 5,273,304 B (5.03 MiB) | 1,262,978 B (1.20 MiB) | 822,693 B (803.4 KiB) | 79,605 B / 10,449 B | **833,142 B (813.6 KiB)** |
| **Chrome Extension Popup (`extension-popup`)** | 1,530,582 B (1.46 MiB) | 401,215 B (391.8 KiB) | 289,852 B (283.1 KiB) | 72,676 B / 9,684 B | **299,536 B (292.5 KiB)** |

### 2.3 Observed Runtime Timings (Curie Clinic `physician-app` in Chromium 155)

Captured on the real compiled `01b762838` `physician-app` Wasm bundle rendering **80 messages** (`39,367` total transcript characters, `62` SSE snapshot refetches over a 15-second active streaming + 6 keys/sec typing + wheel scrolling window, followed by citation panel opens and a 5-second idle check):

| Metric | Desktop 1440×900 (Unthrottled 9950X3D) | Desktop 1440×900 (4× CPU Throttle Proxy) | Mobile Viewport 390×844 (4× CPU Throttle Proxy) |
| --- | ---: | ---: | ---: |
| **Cold start (nav → transcript + composer visible)** | 436.6 ms | 1,360.6 ms | 1,101.7 ms |
| **Time to first usable interaction (first key painted)** | 556.1 ms | 1,731.9 ms | 1,385.1 ms |
| **Typing input latency — p50** | 14.84 ms | 79.03 ms | 43.34 ms |
| **Typing input latency — p95** | 25.55 ms | **159.80 ms** | 60.70 ms |
| **Typing input latency — p99 / max** | 27.28 ms / 27.28 ms | **275.80 ms / 275.80 ms** | 81.64 ms / 81.64 ms |
| **Frame time (`rAF` Δ) — p50** | 16.7 ms | 16.7 ms | 16.7 ms |
| **Frame time (`rAF` Δ) — p95** | 16.7 ms | **66.8 ms** | 33.3 ms |
| **Frame time (`rAF` Δ) — p99 / max** | 16.8 ms / 83.3 ms | **100.0 ms / 150.0 ms** | 33.4 ms / 66.7 ms |
| **Frames > 16.7 ms / Frames > 50 ms** | 32.77% / 0.11% | **61.52% / 11.62%** | 36.60% / 0.34% |
| **Long Tasks (> 50 ms): count / TBT / max** | 0 / 0 ms / 0 ms | **3 / 24.0 ms / 65.0 ms** | 0 / 0 ms / 0 ms |
| **DOM elements / CDP nodes after stress** | 6,462 / 11,382 | 6,461 / 11,538 | 6,461 / 11,524 |
| **Layout / style recalc count (15 s stress)** | 428 / 1,382 | 416 / 971 | 431 / 1,324 |
| **JS heap used / total (post-stress)** | 10.31 MiB / 71.12 MiB | 4.16 MiB / 11.62 MiB | 10.10 MiB / 17.87 MiB |
| **Idle 5 s DOM mutations / layouts** | 0 / 0 | 0 / 0 | 0 / 0 |

*Measurement notes (exact transparency):*
- Citation clicks confirmed `sourceFetches: 2` (standard note `[1]` and 5,000-row lab panel `[12]`). Raw harness timers included fixed 300 ms / 500 ms settle sleeps (`340.0–360.3 ms` gross -> **~40–60 ms net** for standard note; `531.0–562.6 ms` gross -> **~31–63 ms net** for 5,000-row lab source fetch+render).
- During the 5 s idle check, `dom_mutations == 0` and `layout_count_delta == 0`, while the harness's own `requestAnimationFrame` callback loop caused ~300 style recalcs (~60 Hz). M1 Bevy idle tests must measure GPU frame submissions without an active JS `rAF` probe loop so true idle redraw (`0 frames/sec`) is verified.
- **Curie Mobile (`mobile-app`) cold boot (390×844 Chromium mobile viewport, unthrottled):** `101.9 ms` to first rendered content, `2.44 MiB` JS heap used (sign-in screen; authenticated mobile streaming/native shell metrics require real mobile device hardware below).

### 2.4 Unavailable Hardware & Device Targets (`blocked-no-hardware`)

Per program rules, no unavailable target is estimated, fabricated, or marked passed. The following targets are explicitly **`blocked-no-hardware`** on this Linux x86_64 host and must be measured on real devices before M1 exit:

| Target Platform | Status | Missing Capability / Hardware |
| --- | --- | --- |
| **Browser Wasm — Desktop (Firefox ≥ 121, Safari ≥ 17.4)** | `blocked-no-hardware` | Firefox and macOS Safari not installed on this headless Linux NixOS host |
| **Browser Wasm — Mobile Web (Real iOS Safari, Real Android Chrome)** | `blocked-no-hardware` | Physical iPhone (A14/A15, iOS 17/18) and physical midrange Android phone (Snapdragon 6 Gen 1, Android 14) required for real WebGL2, touch scroll, and soft-keyboard resize |
| **Linux Desktop (Midrange Laptop Wayland/X11)** | `blocked-no-hardware` | Reference has no shipped desktop build (`none`); midrange Intel/AMD iGPU Wayland laptop required for M1 Bevy Vulkan/GL qualification |
| **macOS Desktop (Apple Silicon M1/M2 Metal)** | `blocked-no-hardware` | No macOS host available; reference has no desktop build |
| **Windows Desktop (x86_64 DX12)** | `blocked-no-hardware` | No Windows host available; reference has no desktop build |
| **Android Native ARM64 (Curie Mobile)** | `blocked-no-hardware` | No physical ARM64 Android device attached; reference CI runs `x86_64` headless AVD only |
| **iOS Native ARM64 (Curie Mobile)** | `blocked-no-hardware` | No macOS/Xcode/iOS hardware available; Swift bridges cannot compile on Linux |
| **Chrome Extension MV3 Popup (Interactive Chrome Toolbar Cold Open)** | `blocked-no-hardware` | Artifact size measured (`299.5 KiB` Brotli-11); real Chrome toolbar popup cold open under MV3 CSP requires interactive/headed extension harness |

## 3. M1 Performance Budget Table

These budgets are set **before** any Bevy feasibility prototype is judged. Every prototype is evaluated on the representative low/midrange tier named in `platform-register.md` (and on 4× CPU-throttled Chromium as a minimum local gate). Any failure triggers Decision D2.

| Metric | Browser Wasm (Desktop / Mobile Web) | Native Desktop (Linux / macOS / Windows) | Native Mobile (Android / iOS ARM64) | Chrome Extension MV3 Popup | Rationale & Baseline Anchor |
| --- | --- | --- | --- | --- | --- |
| **Cold start (launch/nav → first render)** | **≤ 1,500 ms** desktop / **≤ 2,200 ms** mobile web (warm HTTP cache ≤ 600 ms) | **≤ 600 ms** | **≤ 800 ms** | **≤ 250 ms** | Reference desktop 4× throttle is `1,361 ms` (`437 ms` unthrottled); Bevy Wasm adds shader/WebGL2 init so must avoid regressing cold start past 1.5 s desktop / 2.2 s mobile; MV3 popups tear down on every close and feel broken past 250 ms. |
| **Time to first usable interaction** | **≤ 1,800 ms** desktop / **≤ 2,500 ms** mobile web | **≤ 750 ms** | **≤ 1,000 ms** | **≤ 300 ms** | Reference 4× throttle is `1,732 ms` desktop / `1,385 ms` mobile viewport; replacement must reach interactive composer focus within budget. |
| **Typing input latency — p95 / p99** | **≤ 35 ms** p95 / **≤ 50 ms** p99 | **≤ 16.7 ms** p95 / **≤ 25 ms** p99 | **≤ 25 ms** p95 / **≤ 40 ms** p99 | **≤ 25 ms** p95 / **≤ 40 ms** p99 | Core stutter motivation: reference hit **159.8 ms p95 / 275.8 ms p99** under 4× throttle. Staying ≤ 50 ms p99 keeps typing below the 50–100 ms human perception threshold even during 40 tok/s streaming. |
| **Frame time — p95 / p99** | **≤ 16.7 ms** p95 / **≤ 33.3 ms** p99 (60 Hz target) | **≤ 16.7 ms** p95 / **≤ 25.0 ms** p99 | **≤ 16.7 ms** p95 / **≤ 33.3 ms** p99 | **≤ 16.7 ms** p95 / **≤ 25.0 ms** p99 | Core stutter motivation: reference degraded to **66.8 ms p95 / 100.0 ms p99** (`150 ms` max) under 4× throttle due to full-snapshot DOM reflows. Bevy incremental text layout + virtualized list/table must hold 60 fps p95. |
| **Long tasks > 50 ms (per 15 s streaming window)** | **0 tasks** (> 50 ms), **0 ms TBT** | **0 main-thread stalls > 50 ms** | **0 main-thread stalls > 50 ms** | **0 tasks > 50 ms** | Reference suffered 3 long tasks (max 65 ms) under 4× throttle; off-main-thread / incremental chunk parsing + virtualized 5,000-row table must eliminate >50 ms stalls completely. |
| **Citation panel open (note / 5,000-row lab table)** | **≤ 100 ms** note / **≤ 150 ms** 5,000-row virtualized table | **≤ 50 ms** / **≤ 100 ms** | **≤ 80 ms** / **≤ 120 ms** | N/A | Virtualized table rendering must keep 5,000-row opening O(viewport rows), not O(5,000 DOM nodes). |
| **Memory ceiling (RSS / browser tab heap + Wasm linear mem)** | **≤ 250 MiB** desktop tab / **≤ 150 MiB** mobile web tab | **≤ 200 MiB** RSS | **≤ 150 MiB** RSS (prevents iOS Jetsam / Android LMK kills on 4 GB devices) | **≤ 64 MiB** | Reference JS heap stayed low (~10 MiB JS + Wasm linear mem), whereas GPU/Bevy atlases and glyph caches can balloon; hard ceiling protects 4 GB iPhone SE/12 and 6 GB midrange Android devices. |
| **Idle CPU & redraw rate** | **0 frames/sec** redraw, **< 1% CPU** when idle (reactive `winit` / event-driven) | **0 frames/sec**, **< 1% CPU** | **0 frames/sec**, **< 1% CPU** (mandatory for battery life) | **0 frames/sec**, **< 1% CPU** | Continuous 60 Hz game-loop rendering drains laptop/phone batteries and is explicitly unacceptable for a clinical workspace (`UpdateMode::reactive` required). |
| **Wasm compressed download size (Brotli-11, `.wasm` + JS glue)** | **≤ 3.5 MiB** Curie Clinic / **≤ 2.5 MiB** Curie Mobile Web (with `wasm-opt -Oz` + lean Bevy features) | N/A | N/A | **≤ 1.0 MiB** (hard cap; triggers D2 HTML fallback if Bevy exceeds) | Reference `physician-app` is `3.99 MiB` Brotli (bloated by `turso_core` SQLite demo DB), `mobile-app` is `813.6 KiB`, `extension-popup` is `292.5 KiB`. Bevy Wasm with lean 2D/UI features + `wasm-opt -Oz` must beat reference `physician-app` (`≤ 3.5 MiB`) and keep MV3 popup ≤ 1.0 MiB or switch popup to a narrow HTML/Wasm adapter per D2. |
