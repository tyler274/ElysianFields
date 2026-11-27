# M1 Feasibility Qualification Report: Multiline Text Input, IME, and International Text

This report records the clean-room M1 feasibility prototype and measured qualification results for multiline text input, OS IME bridging, bidirectional RTL layout, complex-script shaping, and open-source font bundling in Bevy `0.20.1`.

All benchmark workloads and multilingual test corpora use 100% synthetic non-PHI text.

## 1. Prototype Revision and Host Environment

| Dimension | Value |
| --- | --- |
| **Prototype crate** | `crates/m1-text-i18n` |
| **Measured prototype commit** | `59427c0eb09550bfb4ac52b8f53b0de41bcfad28` |
| **Engine & text stack** | Bevy `0.20.1` (`bevy_text` / `bevy_ui` / `bevy_input_focus` / `bevy_window`, backed by `parley 0.11.1`, `harfrust 0.12.0`, `skrifa 0.44.0`, `fontique 0.11.1`, `icu_segmenter 2.4.0`, `icu_normalizer 2.4.0`, `taffy 0.14.0`) |
| **Added third-party crates** | `0` (reuses the exact lean Bevy feature set already locked in `Cargo.lock`) |
| **Toolchain** | `rustc 1.99.1`, `cargo 1.99.1`, Rust edition `2024`, targets `x86_64-unknown-linux-gnu` and `wasm32-unknown-unknown` |
| **Measurement host** | Linux `7.3.3 #1-NixOS` `x86_64`, AMD Ryzen 9 9950X3D (16 cores / 32 threads, up to 5.76 GHz), 60 GiB RAM (headless — no GPU display server or physical mobile/macOS/Windows devices attached) |

### Measurement Scope Boundary

The numbers in Section 3 measure headless Rust/Bevy ECS execution, single-owner `ComposerDocument` state transitions, orthographic cluster boundary navigation, incremental active-turn transcript streaming, and ECS UI node synchronization. They do **not** include GPU glyph rasterization, compositor swap-chain latency, live browser main-thread throttling (4x CPU), or live OS IME daemon IPC on representative low/midrange target hardware. Targets requiring physical hardware or interactive browser/OS IME verification are explicitly marked `blocked-no-hardware` in Section 5.

## 2. Single-Owner Architecture and Bevy 0.20.1 Findings

### Strict Single-Owner State Machine (`crates/m1-text-i18n`)

- **Authoritative domain model (`src/composer.rs`, `src/grapheme.rs`, `src/locale.rs`)**: `ComposerDocument` owns the committed UTF-8 buffer, validated orthographic-cluster `CursorState` (`Caret` or normalized `Selection`), bounded undo/redo history (128 snapshots), `ImeComposition` (`Idle` or `Composing { text, cursor_offset, candidate_anchor }`), `FocusState` (`Unfocused`, `Focused`, or `SuspendedWaitingRestore`), `SoftKeyboardState` (`Hidden` or `Visible { inset_bottom_px }`), and viewport scroll clamping. Domain types carry zero Bevy ECS dependencies and zero boolean state soup.
- **Compiler-enforced adapter boundary (`src/adapter.rs`)**: Platform bridges implement `PlatformInputAdapter`, sending typed `TextInputCommand` values into `ComposerDocument` and receiving read-only `AdapterReadback` geometry/focus/revision projections. `AdapterReadback` intentionally omits any copy of the document text so platform bridges cannot drift into a second editable store.
- **Incremental ECS projection (`src/plugin.rs`)**: `TextI18nPrototypePlugin` syncs `ComposerViewNode` only when the composer generation advances and updates `ActiveTranscriptTurnViewNode` incrementally on the single mutated turn without traversing or re-laying out prior turns. Idle frames perform zero view mutations.
- **Bevy `0.20.1` complex-script shaping readiness**: Inspecting the locked Bevy `0.20.1` dependency graph confirms `bevy_text` builds on `parley` + `harfrust` (pure-Rust HarfBuzz OpenType GSUB/GPOS shaping) + `skrifa` + `fontique` + ICU4X segmenter/normalizer. OpenType Arabic/Urdu contextual joining, BiDi layout, and Indic conjunct/matra shaping compile cleanly to both native and `wasm32-unknown-unknown` with zero C/C++ HarfBuzz FFI dependencies, provided the required SIL OFL fonts are bundled.

## 3. Measured Runtime and Build Performance

### Concurrent Streaming-Above-Composer Latency Benchmark

Command executed on commit `59427c0eb09550bfb4ac52b8f53b0de41bcfad28`:

```sh
cargo test -p m1-text-i18n --test text_i18n_qualification -- --nocapture --test-threads=1
```

Workload contract (`specs/m1-feasibility-budgets.md`):

- **Transcript**: 80 turns (40 clinician prompts + 40 structured assistant responses containing headings, bulleted/numbered lists, lab tables, fenced JSON, and inline citations `[1]`–`[12]`), **47,931 characters** total.
- **Active window**: 15 simulated seconds at 60 Hz (`900` frames), streaming **40 tokens/sec** (`600` incremental chunks) into the active turn while concurrently typing **6 keys/sec** (`90` multiline typing events) into the composer.
- **Idle window**: 5 simulated seconds (`300` frames) with zero stream or input events immediately after the active window.

| Locale | Script / Dir | Typing p50 (µs) | Typing p95 (µs) | Typing p99 (µs) | Typing max (µs) | Frame p95 (µs) | Frame p99 (µs) | Stalls `> 50 ms` | Idle View Syncs |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `en` | Latin (LTR) | 95.44 | 126.12 | 1,098.45 | 1,140.09 | 96.58 | 117.13 | 0 | 0 |
| `zh-Hans` | Han (LTR) | 106.02 | 139.98 | 148.74 | 156.47 | 106.02 | 134.56 | 0 | 0 |
| `ja` | Kanji/Kana (LTR) | 101.14 | 133.33 | 243.43 | 316.70 | 101.14 | 128.15 | 0 | 0 |
| `ko` | Hangul (LTR) | 101.13 | 153.16 | 569.31 | 618.98 | 99.15 | 128.99 | 0 | 0 |
| `ar` | Arabic (RTL) | 90.80 | 118.62 | 130.37 | 137.62 | 89.77 | 113.67 | 0 | 0 |
| `ur` | Arabic/Urdu (RTL) | 96.61 | 134.43 | 145.59 | 340.43 | 95.43 | 119.13 | 0 | 0 |
| `hi` | Devanagari (LTR) | 112.33 | 139.32 | 179.21 | 1,379.05 | 112.33 | 132.34 | 0 | 0 |
| `bn` | Bengali (LTR) | 103.27 | 132.79 | 140.71 | 153.96 | 103.09 | 127.31 | 0 | 0 |
| `ta` | Tamil (LTR) | 109.84 | 148.52 | 160.75 | 202.53 | 109.18 | 143.46 | 0 | 0 |
| `te` | Telugu (LTR) | 111.42 | 148.01 | 215.00 | 508.69 | 111.23 | 136.31 | 0 | 0 |

Comparison against M1 budgets (`specs/m1-feasibility-budgets.md`):

- **Typing latency**: Headless state + ECS sync measures `0.12–0.15 ms` p95 and `0.13–1.10 ms` p99 across all 10 benchmarked locales — leaving >99% of the `16.7 ms` (native desktop) and `35 ms` (browser Wasm) p95 budgets available for GPU text shaping and presentation.
- **Long tasks / stalls `> 50 ms`**: `0` across all locales.
- **Idle redraw invariant**: `0` composer or transcript view syncs across all `300` idle frames per locale.

### Build, Lint, and Test Timing (`59427c0`)

| Check | Measured Wall Time | Result |
| --- | ---: | --- |
| Warm incremental `cargo clippy -p m1-text-i18n --all-targets -- -D warnings` | `3.00 s` | Pass (`0` warnings) |
| Warm incremental `cargo clippy -p m1-text-i18n --target wasm32-unknown-unknown -- -D warnings` | `2.98 s` | Pass (`0` warnings) |
| Incremental `cargo test -p m1-text-i18n` compile + link (`dev` profile) | `8.79 s` | Pass |
| Full qualification suite (`12` integration tests including 10-locale benchmark) | `0.40 s` | Pass (`12 / 12`) |

## 4. Multilingual and Complex-Script Qualification Matrix

All 9 shipped UI locales and 6 complex-script shaping probe locales are verified in `crates/m1-text-i18n/tests/text_i18n_qualification.rs`:

| Locale | Tier | Script | Direction | Input Profile | Bundled Font Family | Headless Prototype Coverage |
| --- | --- | --- | --- | --- | --- | --- |
| `en` | Shipped UI | Latin | LTR | Direct keyboard | Noto Sans | Multiline edit, selection, cut/copy/paste, undo/redo, dictation auto-space, soft-keyboard clamp, 80-turn benchmark |
| `es` | Shipped UI | Latin | LTR | Direct keyboard | Noto Sans | Diacritics, Bevy ECS view sync, zero-redraw idle verification, full cluster round-trip |
| `fr` | Shipped UI | Latin | LTR | Direct keyboard | Noto Sans | Accented Latin, single-owner `HeadlessSimulatedAdapter` round-trip, full cluster round-trip |
| `pt` | Shipped UI | Latin | LTR | Direct keyboard | Noto Sans | Cedilla/tilde diacritics, full cluster navigation and cut/undo round-trip |
| `de` | Shipped UI | Latin | LTR | Direct keyboard | Noto Sans | Umlauts/ß, full cluster navigation and cut/undo round-trip |
| `zh-Hans` | Shipped UI | Han | LTR | OS IME composing | Noto Sans CJK SC | IME preedit/cancel, script-continuous dictation (no spurious space), 80-turn benchmark |
| `ja` | Shipped UI | Kanji/Kana | LTR | OS IME composing | Noto Sans CJK JP | IME preedit, candidate anchor progression, commit, 80-turn benchmark |
| `ko` | Shipped UI | Hangul | LTR | OS IME composing | Noto Sans CJK KR | Hangul multiline editing, background/foreground focus & selection restoration, preedit teardown, 80-turn benchmark |
| `ar` | Shipped UI | Arabic | **RTL** | RTL bidirectional | Noto Sans Arabic | RTL direction detection, visual left/right caret inversion, harakat combining-mark cluster integrity, 80-turn benchmark |
| `ur` | Shaping Probe | Arabic | **RTL** | RTL bidirectional | Noto Sans Arabic | RTL direction detection, visual caret inversion, Nastaliq/Naskh punctuation, 80-turn benchmark |
| `hi` | Shaping Probe | Devanagari | LTR | Complex Indic | Noto Sans Devanagari | Virama conjunct (`क्षा`) single-cluster navigation & atomic forward delete, 80-turn benchmark |
| `bn` | Shaping Probe | Bengali | LTR | Complex Indic | Noto Sans Bengali | Virama conjunct (`ক্ষি`) single-cluster navigation & atomic forward delete, 80-turn benchmark |
| `mr` | Shaping Probe | Devanagari | LTR | Complex Indic | Noto Sans Devanagari | Virama conjunct (`क्ति`) single-cluster navigation & atomic forward delete, full cluster round-trip |
| `ta` | Shaping Probe | Tamil | LTR | Complex Indic | Noto Sans Tamil | Pulli conjunct (`க்ஷி`) single-cluster navigation & atomic forward delete, 80-turn benchmark |
| `te` | Shaping Probe | Telugu | LTR | Complex Indic | Noto Sans Telugu | Virama conjunct (`క్షి`) single-cluster navigation & atomic forward delete, 80-turn benchmark |

## 5. Per-Target Platform Qualification Table

Every target in `specs/m1-platforms.md` is evaluated honestly below. Because this qualification run executed on a headless Linux host without physical macOS, Windows, Android, iOS, or interactive desktop/mobile browser hardware, targets requiring real-device OS IME and GPU canvas verification remain `blocked-no-hardware` until hardware verification completes, alongside their architectural adapter verdict.

| # | Ledger Platform Key | Current Qualification Status | Architectural Target Verdict | Narrow Single-Owner Adapter Scope & Estimated Maintenance Cost | Blocking Evidence Required on Target Hardware |
| --- | --- | --- | --- | --- | --- |
| 1 | `browser-wasm-desktop` | **`blocked-no-hardware`** | `pass-with-adapter` | **Hidden/positioned `<textarea>` IME bridge** (~280–400 LOC JS/Wasm glue; low maintenance). Translates browser `compositionstart`/`compositionupdate`/`compositionend`, `beforeinput`, clipboard, and dictation events into `TextInputCommand`, positioning the invisible proxy at `AdapterReadback::caret_rect` / `preedit_anchor` and clearing the proxy after commit so `ComposerDocument` stays the single text owner. | Interactive Chromium >= 120, Firefox >= 121, Safari >= 17.4 verification with CJK/Arabic IME + 4x CPU throttle GPU canvas latency capture. |
| 2 | `browser-wasm-mobile` | **`blocked-no-hardware`** | `pass-with-adapter` | **Shared hidden `<textarea>` IME bridge + `window.visualViewport` inset listener** (~320–450 LOC combined with desktop web; low–moderate maintenance). Emits `UpdateSoftKeyboard` and `AppBackgrounded`/`AppForegrounded` (`visibilitychange`) into `ComposerDocument`. | Real Android 14 Chrome (Snapdragon 6 Gen 1 tier) and iOS 17 Safari (iPhone 12/SE tier) soft-keyboard resize, dictation, and WebGL2 context-loss recovery test. |
| 3 | `desktop-linux` | **`blocked-no-hardware`** | `pass-with-adapter` | **Native `winit` IME (`WindowEvent::Ime`) + `bevy_clipboard` bridge** (~180–250 LOC Rust; low maintenance). Feeds `Preedit`/`Commit` events into `ComposerDocument` and calls `Window::set_ime_position` from `AdapterReadback::preedit_anchor`. Headless Rust/Bevy prototype + benchmark already pass on Linux `x86_64`. | Live Wayland/X11 session on representative i5-10210U / Ryzen 5 5500U GPU tier with IBus/Fcitx5 CJK IME and Arabic RTL rendering capture. |
| 4 | `desktop-macos` | **`blocked-no-hardware`** | `pass-with-adapter` | **Native `winit` IME (`WindowEvent::Ime`) + macOS clipboard/dictation shim** (~200–280 LOC Rust + minimal AppKit hook if dictation bypasses `winit`; low maintenance). | Physical MacBook Air M1 (macOS 14/15, Metal) live IME candidate placement, macOS Dictation, and VoiceOver verification. |
| 5 | `desktop-windows` | **`blocked-no-hardware`** | `pass-with-adapter` | **Native `winit` IME (`WindowEvent::Ime`) + Windows TSF/clipboard bridge** (~180–250 LOC Rust; low maintenance). | Physical Windows 11 23H2 (i5-1135G7 / Iris Xe, DX12) Microsoft IME (Japanese/Chinese/Korean/Arabic) and touch-keyboard test. |
| 6 | `android-native` | **`blocked-no-hardware`** | `pass-with-adapter` | **Android `InputConnection` + `WindowInsetsCompat.Type.ime()` JNI bridge** (~350–500 LOC Kotlin + Rust JNI; moderate maintenance). Streams `setComposingText`/`commitText`/key events into `TextInputCommand` and reads `AdapterReadback` without holding a duplicate editable buffer. | Physical Pixel 6a / Galaxy A35 (ARM64, Android 14, Vulkan 1.1) Gboard IME, voice dictation, soft-keyboard resize, and process background/restore test. |
| 7 | `ios-native` | **`blocked-no-hardware`** | `pass-with-adapter` | **iOS `UIView<UITextInput>` + keyboard frame notification bridge** (~350–500 LOC Swift/Obj-C + Rust FFI; moderate maintenance). Bridges marked text (preedit), dictation, clipboard, and `keyboardWillChangeFrame` into `ComposerDocument`. | Physical iPhone 12 / SE 3rd gen (ARM64, iOS 17/18, Metal) Japanese/Chinese/Arabic keyboard, iOS Dictation, and background/foreground focus restore test. |
| 8 | `chrome-extension-popup` | **`blocked-no-hardware`** | `pass-with-adapter` | **Read-only connector popup (no multiline composer required)**; under Decision D2, if Wasm exceeds `<= 1.0 MiB` or `<= 250 ms` cold open, use a narrow semantic HTML popup surface (~200–350 LOC HTML/TS; low maintenance). | Chrome 120+ MV3 cold toolbar popup open timing, CSP validation, and Brotli-11 size measurement. |
| 9 | `hosted-services` | **`pass`** | `pass` | **Not applicable (headless backend services)**. Pure Rust domain types (`locale`, `grapheme`, `composer`) carry no client rendering or windowing dependencies. | None (validated natively on Linux `x86_64`). |
| 10 | `static-public-web` | **`pass`** | `pass` | **Semantic HTML/CSS** (`lang`, `dir="rtl"`/`dir="auto"`, WOFF2 web fonts). Native browser text shaping and accessibility with zero canvas rendering. | None for text input/IME (static non-canvas surfaces). |

### Program Stop-Rule Assessment

Under `specs/m1-feasibility-budgets.md` Section 4, Browser Wasm text input does **not** trigger a program stop at the architecture/prototype stage: the hidden/positioned `<textarea>` IME bridge preserves a strict single state owner (`ComposerDocument` + read-only `AdapterReadback`) in ~280–450 LOC without duplicating editable text or building a second UI stack. Final M1 sign-off remains gated on running this narrow bridge against real Chromium, Firefox, Safari, Android Chrome, and iOS Safari hardware.

## 6. Bundled Font Payload and SIL OFL 1.1 Licensing Breakdown

GPU-rendered clients (especially browser Wasm) cannot rely on host CSS font fallback. All 10 required font families use the **SIL Open Font License 1.1 (`SIL OFL 1.1`)**, which permits commercial redistribution and embedding provided copyright notices and the OFL license text accompany the distribution and fonts are not sold standalone.

| Font Family | Covered Locales / Scripts | License | Approx. Compressed Payload (Regular + Bold WOFF2/Brotli) | Delivery Strategy |
| --- | --- | --- | ---: | --- |
| **Noto Sans** | `en, es, fr, pt, de` (Latin + diacritics) | SIL OFL 1.1 | `145 KiB` | Core eager bundle |
| **Noto Sans Mono** | Code/JSON blocks, lab tables, numeric vitals | SIL OFL 1.1 | `95 KiB` | Core eager bundle |
| **Noto Sans Arabic** | `ar, ur` (Arabic & Urdu RTL shaping) | SIL OFL 1.1 | `120 KiB` | On-demand script chunk (`/elysianfields/fonts/`) |
| **Noto Sans CJK SC** | `zh-Hans` (Simplified Chinese, unicode-range subset chunks) | SIL OFL 1.1 | `1,850 KiB` | On-demand script chunk (`/elysianfields/fonts/`) |
| **Noto Sans CJK JP** | `ja` (Japanese Kanji/Kana, unicode-range subset chunks) | SIL OFL 1.1 | `1,700 KiB` | On-demand script chunk (`/elysianfields/fonts/`) |
| **Noto Sans CJK KR** | `ko` (Korean Hangul, unicode-range subset chunks) | SIL OFL 1.1 | `1,450 KiB` | On-demand script chunk (`/elysianfields/fonts/`) |
| **Noto Sans Devanagari** | `hi, mr` (Hindi, Marathi) | SIL OFL 1.1 | `110 KiB` | On-demand script chunk (`/elysianfields/fonts/`) |
| **Noto Sans Bengali** | `bn` (Bengali) | SIL OFL 1.1 | `125 KiB` | On-demand script chunk (`/elysianfields/fonts/`) |
| **Noto Sans Tamil** | `ta` (Tamil) | SIL OFL 1.1 | `65 KiB` | On-demand script chunk (`/elysianfields/fonts/`) |
| **Noto Sans Telugu** | `te` (Telugu) | SIL OFL 1.1 | `105 KiB` | On-demand script chunk (`/elysianfields/fonts/`) |

### Download Budget Impact

- **Browser Wasm initial download (`CoreEagerBundle`)**: **`240 KiB`** (`Noto Sans` + `Noto Sans Mono`), representing `6.7%` of the `3.5 MiB` Curie Clinic Wasm budget and `9.4%` of the `2.5 MiB` Curie Mobile Web budget.
- **On-demand non-CJK script packs (Arabic/Urdu + all 4 Indic scripts)**: `65–125 KiB` per script (`525 KiB` combined), fetched lazily under `/elysianfields/fonts/` only when a matching locale or Unicode script run enters the active session.
- **On-demand CJK script packs (`zh-Hans`, `ja`, `ko`)**: `1.45–1.85 MiB` per subsetted CJK language (`5.00 MiB` combined). Eagerly bundling CJK fonts into the initial Wasm payload would breach both `<= 3.5 MiB` and `<= 2.5 MiB` budgets; therefore browser Wasm **must** serve CJK font chunks on demand (matching the active locale or detected Han/Kana/Hangul runs) and cache them in HTTP/CacheStorage.
- **Native Desktop and Native Mobile installers**: Full offline multi-script bundle (`5,765 KiB` / `~5.63 MiB` compressed across all 10 families) is practical inside native `.deb`/AppImage/`.dmg`/MSIX/AAB/IPA packages where no Wasm download cap applies.
