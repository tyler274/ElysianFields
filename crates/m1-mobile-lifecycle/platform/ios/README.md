# iOS Swift / C-ABI FFI Platform Adapter Contract (`ios-native`)

This document defines the narrow Swift / C-ABI (`extern "C"`) bridge surface between the iOS host shell (`aarch64-apple-ios`, iOS 15.0+ minimum / qualified on iOS 17–18) and the Rust domain layer in `m1-mobile-lifecycle`.

## Architectural Boundary

- **Rust owns all product logic and state**: lifecycle state machine, render-loop suspension gate (`should_draw()`), `SavedScreenState` binary codec, deep-link URI validation, notification routing, health permission state machine, and safe-area layout calculation.
- **Swift owns only OS integration callbacks**: bridging UIKit `UISceneDelegate`, `UNUserNotificationCenterDelegate`, `HKHealthStore`, Keychain Services (`Security.framework`), and `UIWindow` safe-area metrics over a fixed C-ABI surface.
- **Zero business logic or second UI store in Swift**: Swift never duplicates editable drafts, never constructs custom URL routing tables, and never prints secrets into `os_log` / `NSLog`.

## Narrow C-ABI FFI Surface

| # | Direction | Swift Entry Point / Apple Framework API | Rust FFI Target (`extern "C"`) | Data Transferred |
| --- | --- | --- | --- | --- |
| 1 | Swift -> Rust | `UISceneDelegate` (`sceneDidEnterBackground`, `sceneWillEnterForeground`, `sceneDidDisconnect`) | `ef_ios_on_lifecycle_event(event_tag: u8)` -> `AppLifecycleEvent` | Single enum discriminant (`u8`) |
| 2 | Rust <-> Swift | `stateRestorationActivity(for:)` / `scene(_:willConnectTo:options:)` `NSUserActivity` userInfo | `ef_ios_encode_saved_state(out_ptr, out_len)` / `ef_ios_restore_saved_state(ptr, len)` | Versioned, checksummed `SavedScreenState` bytes (zero secrets) |
| 3 | Swift -> Rust | Universal Links (`scene(_:continue:)` / `NSUserActivityTypeBrowsingWeb` for `https://dabney.moe/elysianfields/*`) and `curie://` (`scene(_:openURLContexts:)`) | `ef_ios_on_deep_link(uri_utf8, uri_len, launch_mode_tag: u8)` -> `DeepLinkRoute::parse` | UTF-8 URI slice + launch mode tag (`u8`) |
| 4 | Swift -> Rust | APNs + `UNUserNotificationCenterDelegate.userNotificationCenter(_:didReceive:withCompletionHandler:)` | `ef_ios_on_notification_tap(id_ptr, id_len, kind_tag, target_ptr, target_len, launch_mode_tag)` -> `NotificationPayload::parse_from_bridge` | Typed fields (`NotificationKind` tag + IDs) |
| 5 | Rust -> Swift | `HKHealthStore.isHealthDataAvailable()` + stored per-metric request status | `ef_ios_health_permission_state(metric_tag: u8) -> u8` | Metric tag (`u8`) -> `HealthPermissionState` tag (`u8`) |
| 6 | Rust -> Swift | `HKHealthStore.requestAuthorization(toShare:read:)` | `ef_ios_health_request_permission(metric_tag: u8, cb)` | Single-shot async callback -> `HealthPermissionState` tag (`u8`) |
| 7 | Rust -> Swift | `HKSampleQuery` (sorted by `HKSampleSortIdentifierEndDate`, limit 1) | `ef_ios_health_read_latest_sample(metric_tag: u8, out_sample)` | Typed C struct (`metric_tag`, value fields, epoch secs) |
| 8 | Rust -> Swift | Keychain Services (`SecItemAdd`, `SecItemCopyMatching`, `SecItemUpdate`, `SecItemDelete` with `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`) | `ef_ios_keychain_{store,load,delete}(key_tag: u8, ...)` | Finite `SecureKey` tag (`u8`) + opaque buffer (zeroed on Swift side after copy) |
| 9 | Swift -> Rust | `UIWindow.safeAreaInsets`, `UIScreen.main.scale`, `UIWindowScene.interfaceOrientation`, `UIResponder.keyboardWillChangeFrameNotification`, iPadOS split-view bounds | `ef_ios_on_viewport_changed(spec: EfViewportSpecC)` -> `ViewportInsets::compute` | Fixed-layout `#[repr(C)]` struct of 11 integers |

## Platform-Specific Nuances

- **HealthKit read-privacy semantics**: Apple's `HKHealthStore` intentionally hides read-permission denial from apps to protect user privacy (returning `0` samples on `HKSampleQuery` when read access is denied). The Swift adapter reports `Unavailable` when `HKHealthStore.isHealthDataAvailable()` is false, records whether `requestAuthorization` has already been presented once per metric (preventing prompt loops), and maps empty query results to `HealthUiOutcome::AuthorizedNoData` while surfacing a non-blocking Settings link so denied users can adjust Health permissions without crashing or looping.
- **Universal Links verification**: Serve `/.well-known/apple-app-site-association` on `dabney.moe` scoping `components` strictly to `/elysianfields/*` and matching the Apple Team ID + Bundle ID.
- **Metal render suspension**: Pause the `CADisplayLink` / Metal drawable submission immediately on `sceneDidEnterBackground` so `LifecycleController::should_draw()` enforces `0 frames/sec` while hidden and complies with iOS background GPU termination rules (iOS terminates apps that submit Metal work in the background).
- **Keychain persistence across reinstall/restart**: Use `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`; if Keychain returns `errSecInteractionNotAllowed`, map strictly to `SecureStorageError::DeviceLocked` rather than reporting an empty secret.

## Estimated Maintenance Cost

| Component | Estimated Swift/C-FFI Footprint | Expected Maintenance Cadence |
| --- | --- | --- |
| `UISceneDelegate` lifecycle & `NSUserActivity` snapshot bridge | ~100 LOC | Annual iOS SDK release validation |
| Universal Links & APNs delegate bridge | ~100 LOC | Low (stable UIKit / UserNotifications APIs) |
| HealthKit (`HKHealthStore`) adapter | ~170 LOC | Low (new metric types added incrementally) |
| Keychain Services (`Security.framework`) wrapper | ~110 LOC | Low (stable C Keychain API) |
| Safe-area, Dynamic Island, keyboard & iPad split-view bridge | ~90 LOC | Low (new device geometry verification) |
| **Total iOS adapter shell** | **~570 LOC** | **Narrow, single-purpose shell; < 1 engineer-day/quarter** |
