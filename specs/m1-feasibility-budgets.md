# M1 Feasibility Performance Budgets and Stutter Workload Specification

This specification is Gatekeeper (Aponia)-approved for Builder (Vill-V) consumption inside the Elysian Realm. It defines the repeatable clinical streaming-transcript stress workload, multilingual text/accessibility qualification scope, and hard performance, memory, idle, and download-size budgets that every M1 interactive client prototype must meet before M2 is authorized.

All measurements use 100% synthetic data (zero protected health information and zero real messages).

## 1. Repeatable Stutter Workload Contract

The primary interactive smoothness workload models a long streaming clinical assistant transcript running concurrently with physician typing, viewport scrolling, and cited-source inspection in Curie Clinic (browser and desktop) and Curie Mobile.

| Dimension | Required Workload Contract |
| --- | --- |
| **Conversation length** | **80 turns** (40 clinician prompts + 40 rich clinical assistant responses) |
| **Transcript volume** | **~40,000 characters** of structured markdown and inline citation segments |
| **Streaming cadence** | **40 tokens/sec** (~160–200 chars/sec) sustained over **120 seconds** (automated CI/local capture probes run at least a **15-second** high-resolution active window pushing incremental chunk updates at >= 4 Hz) |
| **Rich text structure** | Headings, bulleted and numbered lists, multi-column clinical lab tables, fenced code/JSON blocks, and **12 distinct inline citation markers** (`[1]`–`[12]`) per active thread |
| **Concurrent composer typing** | Continuous multiline composer typing at **6 keys/sec** (~166 ms cadence) throughout active streaming |
| **Concurrent viewport scrolling** | Smooth vertical wheel/touch scrolling (±420 px/sec alternating up/down across the 80-turn transcript) every 1 second while streaming and typing |
| **Concurrent citation inspection** | Opening and switching a non-modal cited-source reading panel between a multi-section progress note citation (`[1]`) and a **5,000-row structured clinical lab/record table** (`[12]`) |
| **Idle phase** | **5–10 seconds** with zero streaming events and zero user input immediately after the active stress window |

### Behavioral Scaling Invariants

- Streaming chunk arrival must apply incremental updates to the active turn rather than re-parsing or re-laying out the full 80-turn transcript on every chunk.
- Large transcripts and the 5,000-row lab table must virtualize layout and rendering to O(visible viewport rows), never O(total document rows).
- Idle rendering must be event-driven/reactive: when no input, animation, or stream event occurs, frame submission drops to **0 frames/sec**. Browser measurement harnesses must record GPU frame submissions without running a continuous JS animation-frame polling loop during the idle window.

## 2. Multilingual Shaping, Input, and Accessibility Qualification Scope

Every interactive M1 prototype must qualify text input, bidirectional shaping, and assistive-technology integration alongside the stutter workload:

- **Shipped UI locales (9 languages)**:
  1. English (`en`, Latin, LTR)
  2. Spanish (`es`, Latin, LTR)
  3. French (`fr`, Latin, LTR)
  4. Portuguese (`pt`, Latin, LTR)
  5. German (`de`, Latin, LTR)
  6. Simplified Chinese (`zh-Hans`, Han, LTR)
  7. Japanese (`ja`, Kanji/Kana, LTR)
  8. Korean (`ko`, Hangul, LTR)
  9. Arabic (`ar`, Arabic script, **RTL**)
- **Complex-script shaping verification**: In addition to the 9 shipped UI locales, M1 text-shaping qualification must exercise synthetic corpora covering **Urdu (`ur`, RTL)** and **Indic scripts** (Hindi/Devanagari `hi`, Bengali `bn`, Marathi `mr`, Tamil `ta`, Telugu `te`) so complex shaping, ligatures, line breaking, and fallback behavior are verified upfront.
- **Bundled fonts**: GPU-rendered clients (especially browser Wasm) cannot rely on host CSS system font fallback. Prototypes must bundle permissively licensed open-source fonts (for example, SIL OFL 1.1) covering Latin, Arabic RTL, CJK, Hangul, monospace, and Indic test scripts, passing license checks before admission.
- **Input & IME**: Multiline editing, OS IME composition, text selection, clipboard copy/paste, dictation, mobile soft-keyboard resize, and focus restoration must operate with a **single state owner** (no duplicate editable text copies).
- **Screen readers**: VoiceOver (macOS/iOS), TalkBack (Android), and browser/desktop assistive technologies must read landmarks, headings, controls, validation alerts, and dynamic updates without duplicate accessibility nodes.

## 3. Hard M1 Performance and Size Budgets

Every prototype is evaluated on the representative low/midrange hardware tier defined in [`specs/m1-platforms.md`](./m1-platforms.md) (and under 4x CPU throttling on Chromium as a minimum local browser gate). Exceeding any budget is a failing gate.

| Metric | Browser Wasm (Desktop / Mobile Web) | Native Desktop (Linux / macOS / Windows) | Native Mobile (Android / iOS ARM64) | Chrome Extension MV3 Popup |
| --- | --- | --- | --- | --- |
| **Cold start (launch/nav -> first render)** | `<= 1,500 ms` desktop / `<= 2,200 ms` mobile web (warm HTTP cache `<= 600 ms`) | `<= 600 ms` | `<= 800 ms` | `<= 250 ms` |
| **Time to first usable interaction** | `<= 1,800 ms` desktop / `<= 2,500 ms` mobile web | `<= 750 ms` | `<= 1,000 ms` | `<= 300 ms` |
| **Typing input latency (p95 / p99)** | `<= 35 ms` p95 / `<= 50 ms` p99 | `<= 16.7 ms` p95 / `<= 25 ms` p99 | `<= 25 ms` p95 / `<= 40 ms` p99 | `<= 25 ms` p95 / `<= 40 ms` p99 |
| **Frame time (p95 / p99)** | `<= 16.7 ms` p95 / `<= 33.3 ms` p99 | `<= 16.7 ms` p95 / `<= 25.0 ms` p99 | `<= 16.7 ms` p95 / `<= 33.3 ms` p99 | `<= 16.7 ms` p95 / `<= 25.0 ms` p99 |
| **Long tasks / main-thread stalls `> 50 ms` (per 15 s streaming window)** | `0` tasks, `0 ms` TBT | `0` stalls `> 50 ms` | `0` stalls `> 50 ms` | `0` tasks `> 50 ms` |
| **Citation panel open (note / 5,000-row virtualized lab table)** | `<= 100 ms` / `<= 150 ms` | `<= 50 ms` / `<= 100 ms` | `<= 80 ms` / `<= 120 ms` | N/A |
| **Memory ceiling (tab heap + Wasm linear mem or native RSS)** | `<= 250 MiB` desktop / `<= 150 MiB` mobile web | `<= 200 MiB` RSS | `<= 150 MiB` RSS | `<= 64 MiB` |
| **Idle CPU & redraw rate** | `0 frames/sec`, `< 1%` CPU | `0 frames/sec`, `< 1%` CPU | `0 frames/sec`, `< 1%` CPU | `0 frames/sec`, `< 1%` CPU |
| **Wasm compressed download size (Brotli-11, `.wasm` + JS glue)** | `<= 3.5 MiB` Curie Clinic / `<= 2.5 MiB` Curie Mobile Web | N/A | N/A | `<= 1.0 MiB` hard cap |

## 4. Build, Link, and Governance Rules

- **Build & CI budgets**: M1 prototypes must measure clean build time, incremental build time, and link time per target alongside runtime metrics against the workspace CI budgets (`pr_feedback_secs <= 360 s`, `merge_to_live_secs <= 480 s`, `client_only_merge_secs <= 240 s`). A target that cannot meet build budgets is reported the same way as a runtime failure.
- **Target exit states**: Each target exits M1 as `pass`, `pass-with-approved-adapter` (stating exact adapter scope and maintenance cost), `blocked` (unavailable hardware — never reported as `pass`), or `fail`.
- **Failure policy (Decision D2)**: When a target fails a budget, evaluate a narrow gatekeeper-approved adapter first (for example, an OS text/IME overlay, an accessibility bridge, or a lightweight semantic HTML popup for the browser extension if Wasm exceeds `<= 1.0 MiB` or `<= 250 ms` cold open). If no narrow adapter fits, drop the non-core target with explicit owner sign-off.
- **Program stop rule**: If **Browser Wasm accessibility** or **Browser Wasm text input** fails without a narrow adapter, stop the program for review rather than building a second full UI stack.
