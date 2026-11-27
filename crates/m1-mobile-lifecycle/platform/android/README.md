# Android Kotlin / JNI Platform Adapter Contract (`android-native`)

This document defines the narrow Kotlin / JNI bridge surface between the Android host shell (`aarch64-linux-android`, API 34–35) and the Rust domain layer in `m1-mobile-lifecycle`.

## Architectural Boundary

- **Rust owns all product logic and state**: lifecycle state machine, render-loop suspension gate (`should_draw()`), `SavedScreenState` binary codec, deep-link URI validation, notification routing, health permission state machine, and safe-area layout calculation.
- **Kotlin owns only OS integration callbacks**: forwarding typed values across JNI and executing Android framework APIs that require a `Context`, `Activity`, or `ActivityResultLauncher`.
- **Zero business logic or second UI store in Kotlin**: Kotlin never parses URLs for routing, never duplicates draft text or clinical records, and never logs credential bytes.

## Narrow JNI Bridge Surface

| # | Direction | Kotlin Entry Point / Android API | Rust Boundary Target | Data Transferred |
| --- | --- | --- | --- | --- |
| 1 | Kotlin -> Rust | `Activity.onResume` / `onStop` / `onDestroy` + `ProcessLifecycleOwner` | `nativeOnLifecycleEvent(eventTag: Int)` -> `AppLifecycleEvent` | Single enum discriminant (`u8`) |
| 2 | Rust <-> Kotlin | `Activity.onSaveInstanceState(outState)` / `onCreate(savedInstanceState)` | `nativeEncodeSavedState(): ByteArray` / `nativeRestoreSavedState(bytes: ByteArray)` | Versioned, checksummed `SavedScreenState` bytes (zero secrets) |
| 3 | Kotlin -> Rust | `Activity.onCreate(intent)` (ColdStart) / `onNewIntent(intent)` (WarmResume) for Android App Links (`https://dabney.moe/elysianfields/*`) and `curie://` | `nativeOnDeepLink(uri: String, launchModeTag: Int)` -> `DeepLinkRoute::parse` | Raw URI string + launch mode tag (`u8`) |
| 4 | Kotlin -> Rust | FCM `FirebaseMessagingService` + `PendingIntent` extras on notification tap | `nativeOnNotificationTap(id: String, kindTag: Int, targetRecordId: String?, launchModeTag: Int)` -> `NotificationPayload::parse_from_bridge` | Typed fields (`NotificationKind` tag + IDs) |
| 5 | Rust -> Kotlin | `HealthConnectClient.permissionController.getGrantedPermissions()` | `HealthAccessAdapter::permission_state(metricTag: Int): Int` | Metric tag (`u8`) -> `HealthPermissionState` tag (`u8`) |
| 6 | Rust -> Kotlin | `PermissionController.createRequestPermissionResultContract()` via `ActivityResultLauncher` | `HealthAccessAdapter::request_permission(metricTag: Int): Int` | Single-shot async callback -> `HealthPermissionState` tag (`u8`) |
| 7 | Rust -> Kotlin | `HealthConnectClient.readRecords(ReadRecordsRequest(...))` | `HealthAccessAdapter::read_latest_sample(metricTag: Int)` | Typed numeric sample struct (`metricTag`, value fields, epoch secs) |
| 8 | Rust -> Kotlin | Android Keystore (`AndroidKeyStore` AES-256-GCM key + encrypted app-private blob store) | `SecureStorageAdapter::{store, load, delete}(keyTag: Int, bytes: ByteArray?)` | Finite `SecureKey` tag (`u8`) + opaque byte array (never logged) |
| 9 | Kotlin -> Rust | `ViewCompat.setOnApplyWindowInsetsListener` (`systemBars()`, `displayCutout()`, `ime()`) + `Configuration.orientation` + `Activity.isInMultiWindowMode` | `nativeOnViewportChanged(...)` -> `ViewportSpec` -> `ViewportInsets::compute` | 11 integer metrics (logical px, milli scale factor, orientation/presentation/cutout tags) |

## Manifest and Platform Configuration Requirements

- **App Links verification**: Serve `/.well-known/assetlinks.json` on `dabney.moe` binding the signed Play Store SHA-256 certificate fingerprint; declare `android:autoVerify="true"` scoped strictly to `/elysianfields/`.
- **Health Connect**: Declare `<uses-permission android:name="android.permission.health.READ_HEART_RATE"/>` (and required metric permissions) plus the privacy rationale intent filter (`androidx.health.ACTION_SHOW_PERMISSIONS_RATIONALE` / `android.intent.action.VIEW_PERMISSION_USAGE`).
- **Graphics surface**: `SurfaceView` / `GameActivity` surface lifecycle tied strictly to `LifecycleController::should_draw()`, releasing swapchain frame pacing on `onStop` so background redraw rate is `0 frames/sec`.
- **Keystore accessibility**: Bind AES-256-GCM master key in `AndroidKeyStore` with `setUserAuthenticationRequired(false)` for background refresh compatibility where permitted or return typed `SecureStorageError::DeviceLocked` when locked.

## Estimated Maintenance Cost

| Component | Estimated Kotlin/JNI Footprint | Expected Maintenance Cadence |
| --- | --- | --- |
| Lifecycle & `Bundle` snapshot bridge | ~90 LOC | Annual Android API target bump |
| App Links & FCM intent bridge | ~110 LOC | Low (schema changes only when adding routes) |
| Health Connect adapter (`androidx.health.connect`) | ~180 LOC | Semi-annual Jetpack Health Connect SDK updates |
| Android Keystore AES-GCM wrapper | ~120 LOC | Low (stable Android Keystore API) |
| `WindowInsetsCompat` / multi-window bridge | ~80 LOC | Low (edge-to-edge behavior on new Android API levels) |
| **Total Android adapter shell** | **~580 LOC** | **Narrow, single-purpose shell; < 1 engineer-day/quarter** |
