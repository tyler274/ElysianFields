# M1 Mobile Lifecycle Prototype & Platform Qualification Report

This report defines the clean-room M1 Bevy `0.20.1` mobile lifecycle architecture, platform adapter contracts, estimated maintenance costs, headless qualification results, and physical-hardware release gates for **Android Native (`android-native`)** and **iOS Native (`ios-native`)**.

All tests and fixtures in this prototype use 100% synthetic data.

## 1. Crate & Architecture Summary

The workspace crate [`crates/m1-mobile-lifecycle`](../../crates/m1-mobile-lifecycle) isolates all mobile product invariants in pure, typed Rust modules with zero ECS entanglement and zero third-party dependency additions beyond the pinned workspace `bevy = 0.20.1` lockfile:

- **Lifecycle & render-loop suspension** (`lifecycle.rs`): `AppLifecycleState::{Foreground, BackgroundPaused, ProcessTerminated}` with a single render gate owner (`LifecycleController::should_draw()`). Backgrounded and terminated states draw **0 frames**.
- **Deterministic process-death restoration** (`state_restore.rs`): `SavedScreenState` encodes route, draft text, scroll offset, and active record ID into a versioned, checksummed (`FNV-1a`) deterministic binary snapshot (`EFSS` v1). Secrets and sign-in tokens are excluded from `SavedScreenState` by type construction.
- **Deep-link validation & routing** (`deep_link.rs`): `DeepLinkRoute::parse` accepts only `curie://` and verified HTTPS links strictly under `https://dabney.moe/elysianfields/`, handling both `ColdStart` and `WarmResume` sign-in flows while rejecting untrusted schemes, hosts, subpaths, path traversal, and duplicate/malformed tokens.
- **Notification tap routing** (`notifications.rs`): `NotificationPayload` and `NotificationRouter` open the target screen directly from a closed/terminated process (`ColdStart`) or active session (`WarmResume`).
- **Health data access** (`health.rs`): `HealthAccessAdapter` and `HealthCoordinator` unify `IosHealthKit` and `AndroidHealthConnect` behind `HealthPermissionState::{NotDetermined, Granted, Denied, Unavailable}`. Grant reads authorized synthetic vitals samples; denial yields a non-blocking UI notice without crashing or looping permission prompts.
- **Secure credential storage** (`secure_storage.rs`): `SecureStorageAdapter` binds typed `SecureKey` slots to `IosKeychain` and `AndroidKeystore`. `SecretValue` redacts `Debug`, omits `Display`, zeroes memory on drop, and surfaces `DeviceLocked` errors rather than reporting empty data.
- **Safe-area viewport layout** (`viewport.rs`): `ViewportInsets::compute` resolves portrait/landscape rotation, split-screen multi-window resize, display cutouts (notch, Dynamic Island, landscape camera cutouts), system bars, and soft-keyboard insets into a validated `LayoutRect` and responsive `LayoutColumns`.
- **Headless Bevy `0.20.1` integration** (`plugin.rs`): `MobileLifecyclePrototypePlugin` binds the domain controllers and injected platform adapters into a headless Bevy `Update` schedule.

## 2. Platform Adapter Architecture & Maintenance Cost Breakdown

Both mobile shells follow Decision D2 and the platform adapter boundary in [`specs/m1-platforms.md`](../../specs/m1-platforms.md): Rust owns all domain state and routing rules; Kotlin and Swift remain thin, stateless OS shims.

### Android Native (`android-native`)

- **Target ABI & OS**: `aarch64-linux-android` (production), `x86_64-linux-android` (CI emulator); Android 14 / 15 (API 34–35).
- **Graphics backend**: Vulkan 1.1+ primary; OpenGL ES 3.0+ fallback.
- **Adapter bridge contract**: [`crates/m1-mobile-lifecycle/platform/android/README.md`](../../crates/m1-mobile-lifecycle/platform/android/README.md) (9 narrow JNI touchpoints).

| Subsystem | Android Platform API | Estimated Kotlin/JNI Footprint | Maintenance Cost |
| --- | --- | --- | --- |
| Lifecycle & state bundle | `Activity` / `ProcessLifecycleOwner` + `onSaveInstanceState` `Bundle` | ~90 LOC | Low (annual API target check) |
| Deep links (App Links) | `assetlinks.json` + `Intent` (`onCreate` / `onNewIntent`) scoped to `/elysianfields/` | ~55 LOC | Low (route-stable) |
| Push notifications | FCM (`FirebaseMessagingService`) + `PendingIntent` typed extras | ~55 LOC | Low |
| Health permissions & vitals | Health Connect (`androidx.health.connect:connect-client` + `PermissionController`) | ~180 LOC | Moderate (Jetpack Health Connect updates) |
| Secure credential storage | Android Keystore (`AndroidKeyStore` AES-256-GCM master key + encrypted private blob) | ~120 LOC | Low (stable API) |
| Safe-area & multi-window | `WindowInsetsCompat` (`systemBars`, `displayCutout`, `ime`) + `isInMultiWindowMode` | ~80 LOC | Low |
| **Total Android shell** | **9 narrow JNI calls, zero UI/business logic** | **~580 LOC** | **< 1 engineer-day / quarter** |

### iOS Native (`ios-native`)

- **Target ABI & OS**: `aarch64-apple-ios` (device), `aarch64-apple-ios-sim` (simulator); iOS 15.0+ minimum, qualified on iOS 17 / 18.
- **Graphics backend**: Metal mandatory (`CADisplayLink` paused on `sceneDidEnterBackground`).
- **Adapter bridge contract**: [`crates/m1-mobile-lifecycle/platform/ios/README.md`](../../crates/m1-mobile-lifecycle/platform/ios/README.md) (9 narrow C-ABI `extern "C"` touchpoints).

| Subsystem | iOS Platform API | Estimated Swift/FFI Footprint | Maintenance Cost |
| --- | --- | --- | --- |
| Lifecycle & state restoration | `UISceneDelegate` + `NSUserActivity` restoration bundle | ~100 LOC | Low (annual iOS SDK check) |
| Deep links (Universal Links) | `apple-app-site-association` + `scene(_:continue:)` / `scene(_:openURLContexts:)` scoped to `/elysianfields/` | ~50 LOC | Low |
| Push notifications | APNs + `UNUserNotificationCenterDelegate` | ~50 LOC | Low |
| Health permissions & vitals | HealthKit (`HKHealthStore`, `requestAuthorization`, `HKSampleQuery`) with read-privacy empty-set handling | ~170 LOC | Low |
| Secure credential storage | Keychain Services (`SecItem*` with `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`) | ~110 LOC | Low (stable C API) |
| Safe-area, cutouts & keyboard | `UIWindow.safeAreaInsets`, `interfaceOrientation`, keyboard frame notifications, iPad split view | ~90 LOC | Low |
| **Total iOS shell** | **9 narrow C-FFI calls, zero UI/business logic** | **~570 LOC** | **< 1 engineer-day / quarter** |

## 3. Qualification Matrix

Host verification environment: Linux `7.3.3` `x86_64`, `rustc 1.99.1`, `cargo 1.99.1`, Bevy `0.20.1` (`default-features = false`, reusing the locked workspace `Cargo.lock` with zero new third-party crates).

Per [`specs/m1-feasibility-budgets.md`](../../specs/m1-feasibility-budgets.md) and [`specs/m1-platforms.md`](../../specs/m1-platforms.md), targets without physical test hardware are recorded strictly as `blocked-no-hardware`, never `pass`.

| Qualification Criterion | Headless Bevy + Adapter Contract Suite (`x86_64` Linux) | Physical Android ARM64 (`android-native`) | Physical iPhone ARM64 (`ios-native`) |
| --- | --- | --- | --- |
| Background pause suspends rendering (`0` frames while hidden) | **PASS** (2 tests: foreground -> background -> 10 suppressed ticks -> resume; invalid transition gate) | `blocked-no-hardware` | `blocked-no-hardware` |
| OS process death + deterministic state restoration | **PASS** (2 tests: Bevy `App` drop & cold relaunch restore; checksum/truncation rejection) | `blocked-no-hardware` | `blocked-no-hardware` |
| Cold-start & warm-resume sign-in deep links (`curie://`, `https://dabney.moe/elysianfields/*`) | **PASS** (2 tests: cold/warm sign-in, record/vitals routes, 8 untrusted/malformed URI rejections, zero secret leakage) | `blocked-no-hardware` | `blocked-no-hardware` |
| Closed-app push notification tap-to-open | **PASS** (2 tests: terminated-state cold launch to `RecordDetail`, warm resume to `VitalsOverview`, malformed payload rejection) | `blocked-no-hardware` | `blocked-no-hardware` |
| Health permissions (grant + synthetic vitals read, denial without loop, unavailable) | **PASS** (2 tests: Health Connect grant/denial/fail-closed read + HealthKit grant/`AuthorizedNoData`/single-prompt denial/unavailable) | `blocked-no-hardware` | `blocked-no-hardware` |
| Secure credential storage across restart + redaction | **PASS** (2 tests: iOS Keychain & Android Keystore restart round-trip, `DeviceLocked` error propagation, redacted `Debug`) | `blocked-no-hardware` | `blocked-no-hardware` |
| Safe-area layout (rotation, split-screen, cutouts, soft keyboard) | **PASS** (1 test: Dynamic Island, soft keyboard, landscape cutout -> `SplitPane`, 50% split-screen -> `SinglePane`, inset overflow error) | `blocked-no-hardware` | `blocked-no-hardware` |
| Cold start (`<= 800 ms`), interaction (`<= 1,000 ms`), frame time (`p95 <= 16.7 ms`), RSS (`<= 150 MiB`), Vulkan/Metal surface, TalkBack/VoiceOver | Out of scope on headless host | `blocked-no-hardware` | `blocked-no-hardware` |

**Headless Bevy + adapter suite summary**: **12 / 12 integration tests PASS** (`cargo test -p m1-mobile-lifecycle`), host + `wasm32-unknown-unknown` Clippy `-D warnings` PASS, zero `Cargo.lock` additions, and full workspace `dev-check.sh --tier full` PASS.

**M1 Target Exit States**:
- Headless mobile lifecycle prototype & adapter contracts: **`pass`**
- `android-native` (physical ARM64 device qualification): **`blocked` (`blocked-no-hardware`)**
- `ios-native` (physical ARM64 device qualification): **`blocked` (`blocked-no-hardware`)**

## 4. Physical-Device Sign-Off Checklist (Required Before Mobile Release)

Before `android-native` or `ios-native` can transition from `blocked-no-hardware` to `pass` (or `pass-with-approved-adapter`), the following checks must execute on representative target hardware from [`specs/m1-platforms.md`](../../specs/m1-platforms.md) (Pixel 6a / Moto G Power 5G / Galaxy A35 on Android 14–15; iPhone SE 3rd gen / iPhone 12 on iOS 17–18):

1. **Performance & Resource Budgets**:
   - Cold start to first render `<= 800 ms`; time to first usable interaction `<= 1,000 ms`.
   - Typing input latency `<= 25 ms` p95 / `<= 40 ms` p99 under the 80-turn streaming transcript workload.
   - Frame time `<= 16.7 ms` p95 / `<= 33.3 ms` p99 with `0` main-thread stalls `> 50 ms` per 15-second streaming window.
   - Citation panel open `<= 80 ms` (note) / `<= 120 ms` (5,000-row virtualized lab table).
   - Resident memory ceiling `<= 150 MiB` RSS.
   - Idle & background GPU submission strictly `0 frames/sec`, `< 1%` CPU (verified via Android GPU Inspector / Perfetto and Xcode Instruments Metal System Trace; no iOS background GPU termination).
2. **OS Lifecycle & Process Death**:
   - Trigger real OS process kill in background (`adb shell am kill` on Android; Jetsam / debugger stop + relaunch on iOS) and verify route, draft text, scroll position, and active record ID restore identically.
3. **Deep Links & Push Notifications**:
   - Verify Android App Links (`/.well-known/assetlinks.json`) and iOS Universal Links (`/.well-known/apple-app-site-association`) for `https://dabney.moe/elysianfields/sign-in?token=...` from both cold start and warm resume without browser fallback.
   - Deliver real FCM (Android) and APNs (iOS) notifications to a force-stopped/closed app and verify single-tap direct navigation to the target screen.
4. **OS Health Stores**:
   - Exercise real Health Connect (Android API 34+) and HealthKit (iOS 17/18) authorization dialogs for both **Grant** (reading a real synthetic workout/vitals test entry) and **Deny** (confirming graceful non-blocking UI and zero prompt loops).
5. **Hardware-Backed Secure Storage**:
   - Verify Android Keystore and iOS Keychain persist credentials across app force-stop and device reboot, return `DeviceLocked` cleanly before first unlock, and leak zero plaintext into `logcat` / Console.app logs.
6. **Display Geometry, IME & Screen Readers**:
   - Verify portrait/landscape rotation, Dynamic Island / punch-hole cutouts, Android split-screen multi-window resize, soft-keyboard open/close resize, and TalkBack / VoiceOver focus and landmark navigation without duplicate accessibility nodes.
