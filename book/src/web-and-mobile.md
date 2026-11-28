# Web, PWA, Extension, and Mobile Shims

Browser delivery, the Chrome MV3 connector extension, and native Android/iOS OS shims all follow the single-owner model: business state lives in Rust crates, while platform shells project UI, accessibility landmarks, and OS capabilities through minimal, typed boundaries.

## Compiled Bevy Wasm `cdylib` ABI

`ef-clinic-client`, `ef-mobile-client`, and `m1-web-delivery` compile to `wasm32-unknown-unknown` `cdylib` modules via `cargo rustc --locked --release --target wasm32-unknown-unknown -p <crate> --lib --crate-type cdylib` without adding third-party JS/Wasm binding dependencies:

- **Zero-Glue C-ABI**: `ef-clinic-client/src/wasm_abi.rs` and `ef-mobile-client/src/wasm_abi.rs` expose compact memory allocators and JSON snapshot exports returning Rust-owned `LocalizedChrome`, locale direction, autonyms, and state transitions.
- **Strict CSP Compatibility**: All compiled `.wasm` modules pass `WebAssembly.compile` under `default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; connect-src 'self'; object-src 'none'` across Chromium, Firefox, and WebKit with zero CSP violations.
- **Brotli-11 Budgets**: Release Wasm bundles are measured at Brotli quality 11 in CI against mobile (`<= 2.5 MiB`) and desktop (`<= 3.5 MiB`) compressed ceilings.

## Browser Clinic Shell and Mobile PWA (`web/elysianfields/`)

Mounted strictly under `/elysianfields/`:

- **Clinic Web Shell** (`web/elysianfields/{index.html,boot.js,a11y-bridge.js,locales.js,styles.css}`):
  - Single-owner `ElysianNarrowA11yBridge` projects landmark and control ARIA nodes without duplicating business state.
  - Supports all 7 complex-script QA locales (`en`, `he`, `ar`, `zh-Hans`, `ja`, `vi`, `hi`), CSS logical properties for automatic RTL mirroring, WCAG 2.1 AA Dark/Light themes, 200% text-zoom reflow, `Ctrl/Cmd+Shift+L` Privacy Cover, `?` Shortcuts modal, WebGPU/WebGL2 fallback, and live `/elysianfields/api/*` HTTP/SSE wiring.
- **Curie Mobile PWA** (`web/elysianfields/mobile/{index.html,mobile-boot.js,mobile.css,manifest.webmanifest,sw.js}`):
  - Touch-friendly mobile viewport (`>= 44px` touch targets, zero horizontal overflow in LTR and RTL).
  - Offline Service Worker (`sw.js`) precaches the mobile shell and crisis helplines (`988`, `911`, `SAMHSA`) while never caching `/elysianfields/mobile-api/*` clinical or authentication responses.

## Chrome MV3 Extension Popup (`extension/`)

The connector extension (`extension/{manifest.json,popup.html,popup.css,popup.js,connector.js,worker.js}`) uses a lightweight semantic-HTML popup paired with `ef-extension` protocol tests:

- Stores pairing tokens exclusively in `chrome.storage.session` (memory-only; never persisted to `localStorage` or disk).
- Enforces strict MV3 CSP with zero inline scripts or styles, WCAG 2.1 AA Light/Dark contrast, and `<= 250 ms` popup cold-start latency verified in Playwright.

## Three-Browser Playwright QA Suites

- `scripts/browser_e2e.py`: validates `/elysianfields/`, `/elysianfields/mobile/` (with Mobile Chromium Pixel 7 and Mobile WebKit iPhone 14 touch emulation and offline reload), MV3 popup budget, JavaScript-disabled `ef-public-site` rendering, and live compiled Wasm instantiation under strict CSP.
- `scripts/qa_multilingual_gui.py`: tests all 7 writing systems across Desktop (`1280×800`) and Mobile (`390×844`) — 14 views per browser engine — checking RTL geometry mirroring, zero horizontal overflow, Canvas 2D non-tofu glyph rendering, and BiDi composer round-trips.
- `scripts/qa_ux_accessibility.py`: verifies computed WCAG 2.1 AA contrast ratios in Light and Dark themes, 200% zoom reflow, keyboard Privacy Cover / Shortcuts modals, MV3 popup contrast, and physical crisis-line ordering on the searchable zero-JS `/dei?state=&service=&q=` recovery finder (`crates/ef-public-site/tests/dei_finder_filtering.rs`).

## Native Android and iOS OS Shims (`platform/`)

Stateless OS bridges connect mobile platform capabilities to the Rust core using the `EFSS` v1 binary snapshot contract from `m1-mobile-lifecycle`:

- **Android (`platform/android/`)**: clean-room Gradle project (`settings.gradle.kts`, `build.gradle.kts`, `AndroidManifest.xml` with `allowBackup="false"`, `usesCleartextTraffic="false"`, and explicit `tel:` crisis intent query) plus pure-Kotlin shims (`ElysianMobileActivity.kt`, `HealthConnectBridge.kt`, `KeystoreVault.kt`, `ElysianFcmService.kt`, `MobileAppConfig.kt`) and executable `ElysianMobileShimTest.kt` compiled and executed on CI with `kotlinc` + `java`, alongside `cargo check --target aarch64-linux-android` via Android NDK.
- **iOS (`platform/ios/`)**: SwiftPM package (`Package.swift`) with stateless shims (`ElysianAppDelegate.swift`, `HealthKitBridge.swift`, `KeychainVault.swift`, `ApnsHandler.swift`) and XCTest suite (`ElysianMobileShimTests.swift`) verified via `swiftc -typecheck` (iOS Simulator SDK), `swift test`, and `cargo check --target aarch64-apple-ios-sim` on `macos-15`.
