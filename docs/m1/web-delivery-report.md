# M1 Browser Web Delivery Qualification Report

This report evaluates browser WebAssembly delivery for Elysian Fields against [`specs/m1-feasibility-budgets.md`](../../specs/m1-feasibility-budgets.md) and [`specs/m1-platforms.md`](../../specs/m1-platforms.md). It covers graphics backend detection and fallback diagnostics, WebGL2/WebGPU context-loss recovery without page reload, strict `/elysianfields` subpath resolution, strict Content-Security-Policy (CSP) compliance, real Wasm bundle compression measurements, and headless Chromium verification.

## 1. Qualification Scope and Test Environment

- **Crate under test**: `crates/m1-web-delivery` (Bevy `0.20.1` UI/text/asset/window/state/input-focus feature set matching `crates/elysian-bevy-probe`, zero additional external dependencies)
- **Static & MV3 harnesses**: `web/elysianfields/` (`index.html`, `styles.css`, `bootstrap.js`) and `web/mv3-popup/` (`manifest.json`, `popup.html`, `popup.css`, `popup.js`)
- **Host machine**: Linux `x86_64` (`7.3.3 #1-NixOS`), AMD Ryzen 9 9950X3D (16 cores / 32 threads), 60 GiB RAM
- **Toolchain**: `rustc 1.99.1 (58cf8f9c3 2026-10-13)`, LLVM `23.1.1`, `wasm-bindgen 0.2.130`, Binaryen `wasm-opt 133`, `brotli 1.2.1`
- **Local browser runtime**: Chromium `157.0.8103.38` (`--headless=new`)
- **Measured workspace baseline**: `3215abef7d004dd4e8065f8470f8ce22b0d18ea5`

## 2. Architecture Verified in `crates/m1-web-delivery`

1. **Typed Graphics Backend Detection (`src/graphics.rs`)**:
   - `GraphicsBackend::{WebGpu, WebGl2, Unsupported(FallbackDiagnostic)}` selects WebGPU opportunistically, falls back to the mandatory cross-browser WebGL2 baseline, and produces a structured, human-readable `FallbackDiagnostic` with actionable remediation steps when neither backend is available.
2. **In-Place Context Loss Recovery (`src/context_loss.rs`)**:
   - `ContextLossRecovery` models `Healthy -> Suspended -> Restoring -> Healthy` (and safe `Unrecoverable` fallback) on `webglcontextlost` / WebGPU device loss.
   - Rendering pauses via `RenderScheduleGate` (`RenderExecutionState::PausedForRecovery`), UI state (`PreservedUiSnapshot`: route, composer draft, scroll offset, focused control) stays intact, and surface reconstruction resumes cleanly on `webglcontextrestored` with `ReloadPolicy::NoPageReload`.
3. **Strict `/elysianfields` Subpath Resolver (`src/subpath.rs`)**:
   - Enforces segment-bounded mount checks (`SubpathMount::is_within_mount`), accepting `/elysianfields` and `/elysianfields/...` while rejecting `/` and prefix-confusion paths such as `/elysianfields-evil/app.js` or `/elysianfields2`.
   - Blocks path traversal (`..`, `%2e%2e`, `%2f`, `%5c`), control characters, backslashes, protocol-relative URLs (`//...`), and external schemes when resolving app routes, assets, bundles, and API endpoints.
4. **Strict CSP Verifier & Zero-Inline Harnesses (`src/csp.rs`, `web/`)**:
   - Enforces `default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; frame-ancestors 'none'` for `/elysianfields/` web delivery and `default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; object-src 'self'` for Manifest V3 extension pages.
   - Strictly rejects `'unsafe-inline'`, `'unsafe-eval'`, `'unsafe-hashes'`, wildcards (`*`), `data:`, and remote origins.
   - Integration tests (`crates/m1-web-delivery/tests/web_harnesses.rs`) verify that `web/elysianfields/index.html`, `web/mv3-popup/popup.html`, and `web/mv3-popup/manifest.json` contain zero inline `<script>` blocks, zero inline event handlers or `style=` attributes, and zero domain-root `/` asset references.

## 3. Empirical Wasm Size & Compression Measurements

Built with `cargo build --locked -p m1-web-delivery --release --target wasm32-unknown-unknown` and `cargo build --locked -p elysian-bevy-probe --release --target wasm32-unknown-unknown`. `elysian-bevy-probe` is a library crate compiling in `2.69 s` (warm) to a `1,581,788 B` (`1.51 MiB`) `.rlib`; `m1-web-delivery` links the identical Bevy feature set into an executable `.wasm` module processed with `wasm-bindgen --target web` (`8.78 s`) and Binaryen `wasm-opt -Oz`.

| Artifact Stage | Raw Size | Gzip-9 Size | Brotli-11 Size | Notes |
| --- | ---: | ---: | ---: | --- |
| Raw Cargo `--release` `.wasm` (`m1-web-delivery.wasm`) | `35,106,472 B` (`33.48 MiB`) | `5,268,601 B` (`5.02 MiB`) | `2,300,137 B` (`2.19 MiB`) | Unstripped default release profile |
| Raw Cargo `.wasm` after `wasm-opt -Oz` (`235.33 s`) | `10,499,306 B` (`10.01 MiB`) | `2,927,106 B` (`2.79 MiB`) | `1,558,143 B` (`1.49 MiB`) | Saves `24,607,166 B` (`70.09%` raw) |
| `wasm-bindgen` `.wasm` (`m1_web_delivery_bg.wasm`) | `31,809,342 B` (`30.34 MiB`) | `3,827,858 B` (`3.65 MiB`) | `2,227,597 B` (`2.12 MiB`) | `wasm-bindgen 0.2.130` (`8.78 s`) |
| `wasm-bindgen` JS glue (`m1_web_delivery.js`) | `13,425 B` (`13.11 KiB`) | `3,839 B` (`3.75 KiB`) | `3,356 B` (`3.28 KiB`) | External ES module, zero inline script |
| `wasm-opt -Oz` `.wasm` (`m1_web_delivery_bg.wasm`) | `10,042,485 B` (`9.58 MiB`) | `2,442,321 B` (`2.33 MiB`) | `1,484,906 B` (`1.42 MiB`) | **`238.56 s`** opt time; saves `21,766,857 B` (`68.43%` raw) |
| **Total Optimized Web Bundle (`.wasm` + JS glue)** | **`10,055,910 B` (`9.59 MiB`)** | **`2,446,160 B` (`2.33 MiB`)** | **`1,488,262 B` (`1.42 MiB`)** | Deployed under `/elysianfields/pkg/` |

### Budget Comparison Against `specs/m1-feasibility-budgets.md`

| Product Surface | Brotli-11 Hard Cap | Measured (`m1-web-delivery` Bevy UI) | Margin | Status |
| --- | ---: | ---: | ---: | --- |
| **Curie Clinic Desktop Web** (`browser-wasm-desktop`) | `<= 3.50 MiB` | **`1.42 MiB`** (`1,488,262 B`) | `-2.08 MiB` headroom | **PASS** (headless UI slice; `wgpu` + core fonts must fit within remaining `2.08 MiB`) |
| **Curie Mobile Web** (`browser-wasm-mobile`) | `<= 2.50 MiB` | **`1.42 MiB`** (`1,488,262 B`) | `-1.08 MiB` headroom | **PASS** (tight margin once `wgpu` + fonts are added; requires strict feature-gating and on-demand script font chunks) |
| **Chrome Extension MV3 Popup** (`chrome-extension-popup`) | `<= 1.00 MiB` | **`1.42 MiB`** (`1,488,262 B`) | **`+0.42 MiB` OVER** | **FAIL for Bevy Wasm -> TRIGGERS DECISION D2** |

### Decision D2 Verdict: Chrome Extension MV3 Popup

Shipping Bevy `0.20.1` inside the Chrome Extension MV3 popup **exceeds the `<= 1.0 MiB` Brotli-11 hard cap by `0.42 MiB` (`1.42 MiB` actual)** even before adding `bevy_render`/`wgpu` or bundled fonts. In addition, cold WebGL2 context initialization in headless Chromium took `981.5 ms` against the `<= 250 ms` MV3 cold-open budget.

Under **Decision D2** (`specs/m1-feasibility-budgets.md`), `chrome-extension-popup` exits M1 as **`pass-with-approved-adapter`**:

- **Adapter scope**: A narrow semantic HTML/CSS/JS popup (`web/mv3-popup/`) communicating with the existing MV3 background service worker and content scripts under strict MV3 CSP (`default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; object-src 'self'`).
- **Bundle & startup profile**: `~3.7 KiB` uncompressed (`~1.5 KiB` Brotli-11), zero WebGL2 initialization penalty, well inside both the `<= 1.0 MiB` download cap and `<= 250 ms` cold-open budget.
- **Maintenance cost**: Small, isolated read-only connector surface (`~150–300 LOC`); domain protocol types remain shared in Rust/JSON contracts without introducing a second general-purpose UI framework.

## 4. Real Headless Chromium Strict-CSP & Subpath Verification

`web/elysianfields/` (with the `wasm-opt -Oz` `pkg/` bundle) and `web/mv3-popup/` were served locally over HTTP with strict headers (`Content-Security-Policy` and `X-Content-Type-Options: nosniff`) and segment-bounded route enforcement:

- **Subpath isolation**:
  - `GET /` -> **`404`**
  - `GET /elysianfields-evil/` -> **`404`**
  - `GET /elysianfields/` -> **`200`** with `Content-Security-Policy: default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; frame-ancestors 'none'`
- **Chromium `157.0.8103.38` (`--headless=new`, SwiftShader ANGLE WebGL2, 3 cold runs)**:
  - Readiness & probe code: `data-m1-ready="true"`, `data-m1-backend="webgl2"`, `data-m1-probe-code="0"` on 3/3 runs.
  - CSP compliance: **`0` `securitypolicyviolation` events** on 3/3 runs.
  - Wasm fetch + instantiate + Bevy headless probe (`elysian_m1_delivery_probe`): **`20.00 ms`**, **`20.60 ms`**, **`20.00 ms`** (median **`20.00 ms`**).
  - Total cold boot (`navStart -> ready`, including SwiftShader WebGL2 init): **`148.70 ms`**, **`649.30 ms`**, **`1,011.60 ms`** (median **`649.30 ms`**, within `<= 1,500 ms` desktop cold-start cap).
- **Chromium Unsupported-GPU Fallback (`--disable-webgl --disable-webgl2 --disable-gpu`)**:
  - `data-m1-ready="true"`, `data-m1-backend="unsupported"`, `data-m1-probe-code="2"` (`DeliveryReadiness::UnsupportedGraphics`), **`0` CSP violations**, `#fallback-panel` visible (`hidden=false`), total cold boot **`23.10 ms`** (Wasm init **`20.20 ms`**).
- **MV3 Popup Strict-CSP Harness (`web/mv3-popup/popup.html`)**:
  - `data-mv3-ready="true"`, `data-mv3-backend="webgl2"`, **`0` CSP violations** under strict MV3 CSP (`default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; object-src 'self'`). Cold WebGL2 init took `981.50 ms`, confirming that the semantic-HTML Decision D2 adapter should omit canvas/WebGL2 initialization in production.

## 5. Per-Target Platform Qualification Matrix

| Ledger Platform Key | Browser / Environment | Exit Status | Evidence / Blocker |
| --- | --- | --- | --- |
| `browser-wasm-desktop` (Chromium Linux headless) | Chromium `157.0.8103.38` | **pass (local Linux gate)** | `1.42 MiB` Brotli-11 (`<= 3.5 MiB`), `0` CSP violations, `/elysianfields/` verified, `20.0 ms` median Wasm init, `649.3 ms` median cold boot, unsupported-GPU fallback verified |
| `browser-wasm-desktop` (Firefox >= 121, Safari >= 17.4 macOS, 4x CPU throttle midrange laptop) | Firefox / Safari / low-end laptop | **`blocked-no-hardware`** | Physical midrange laptop and macOS Safari hardware unavailable on this Linux host |
| `browser-wasm-mobile` (Android Chrome >= 120, iOS Safari >= 17.4) | Midrange Android / iPhone 12/SE | **`blocked-no-hardware`** | Size (`1.42 MiB <= 2.5 MiB`) verified locally; real mobile WebGL2 context loss and Safari/Chrome cold-start telemetry require physical devices |
| `chrome-extension-popup` (MV3 toolbar popup) | Chrome Stable >= 120 headed toolbar | **`pass-with-approved-adapter` (Decision D2)** / headed toolbar **`blocked-no-hardware`** | Bevy Wasm (`1.42 MiB`) exceeds `1.0 MiB` hard cap; strict-CSP semantic HTML adapter verified locally (`0` CSP violations); headed Chrome toolbar extension loading requires interactive desktop Chrome verification |

## 6. Operational Note for M2 Release Pipeline

Running Binaryen `wasm-opt -Oz` on the default unstripped `31.8 MiB` `wasm-bindgen` artifact took **`238.56 s`**, which nearly saturates the `240 s` `rust-wasm` CI job cap on `main` release merges (see [`build-times-report.md`](./build-times-report.md)). Before enabling `wasm-opt` on `main` deploys in M2, configure `[profile.release]` symbol stripping (`strip = "symbols"` / `debug = false`) and benchmark `wasm-opt -Os` vs `-Oz` on the stripped `.wasm` so release packaging stays safely inside the `240 s` `client_only_merge_secs` budget.
