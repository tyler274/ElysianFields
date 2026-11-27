import Foundation
#if canImport(UIKit)
import UIKit
#endif

/// Stateless iOS lifecycle, `NSUserActivity` state-restoration, Universal Links / `curie://`,
/// and safe-area viewport shim matching `m1-mobile-lifecycle`.
///
/// Rust owns all product logic, routing, draft text, render-loop suspension (`should_draw()`),
/// and `EFSS` v1 state encoding/decoding. This Swift shell holds zero business state and never
/// logs credentials or patient data.

public enum LifecycleEventTag: UInt8 {
    case enterBackground = 1
    case resumeForeground = 2
    case osProcessTerminate = 3
    case coldLaunch = 4
}

public enum LaunchModeTag: UInt8 {
    case coldStart = 1
    case warmResume = 2
}

public enum NotificationKindTag: UInt8 {
    case conversation = 1
    case recordDetail = 2
    case vitalsOverview = 3

    public static func fromWireString(_ raw: String?) -> NotificationKindTag? {
        switch raw {
        case "conversation":
            return .conversation
        case "record_detail":
            return .recordDetail
        case "vitals_overview":
            return .vitalsOverview
        default:
            return nil
        }
    }
}

public enum HealthMetricTag: UInt8, Hashable, CaseIterable {
    case heartRateBpm = 1
    case stepsCount = 2
    case bloodPressureMmHg = 3
    case sleepDurationMinutes = 4
}

public enum HealthPermissionStateTag: UInt8 {
    case notDetermined = 0
    case granted = 1
    case denied = 2
    case unavailable = 3
}

public enum SecureKeyTag: UInt8, CaseIterable {
    case sessionRefreshToken = 1
    case signInExchangeToken = 2
    case deviceBindingKey = 3

    public var accountIdentifier: String {
        switch self {
        case .sessionRefreshToken:
            return "session_refresh_token"
        case .signInExchangeToken:
            return "sign_in_exchange_token"
        case .deviceBindingKey:
            return "device_binding_key"
        }
    }
}

/// Fixed-layout C-ABI viewport geometry matching Rust `ViewportSpec`.
public struct EfViewportSpecC: Equatable {
    public var widthPx: UInt32
    public var heightPx: UInt32
    public var scaleFactorMilli: UInt32
    public var orientationTag: UInt8
    public var presentationTag: UInt8
    public var paneFractionPercent: UInt8
    public var systemTopPx: UInt32
    public var systemRightPx: UInt32
    public var systemBottomPx: UInt32
    public var systemLeftPx: UInt32
    public var cutoutTag: UInt8
    public var cutoutPrimaryPx: UInt32
    public var cutoutSecondaryPx: UInt32
    public var softKeyboardHeightPx: UInt32

    public init(
        widthPx: UInt32,
        heightPx: UInt32,
        scaleFactorMilli: UInt32,
        orientationTag: UInt8,
        presentationTag: UInt8,
        paneFractionPercent: UInt8,
        systemTopPx: UInt32,
        systemRightPx: UInt32,
        systemBottomPx: UInt32,
        systemLeftPx: UInt32,
        cutoutTag: UInt8,
        cutoutPrimaryPx: UInt32,
        cutoutSecondaryPx: UInt32,
        softKeyboardHeightPx: UInt32
    ) {
        self.widthPx = widthPx
        self.heightPx = heightPx
        self.scaleFactorMilli = scaleFactorMilli
        self.orientationTag = orientationTag
        self.presentationTag = presentationTag
        self.paneFractionPercent = paneFractionPercent
        self.systemTopPx = systemTopPx
        self.systemRightPx = systemRightPx
        self.systemBottomPx = systemBottomPx
        self.systemLeftPx = systemLeftPx
        self.cutoutTag = cutoutTag
        self.cutoutPrimaryPx = cutoutPrimaryPx
        self.cutoutSecondaryPx = cutoutSecondaryPx
        self.softKeyboardHeightPx = softKeyboardHeightPx
    }
}

/// Validates the `EFSS` v1 binary envelope (magic + version + FNV-1a-32 checksum)
/// produced by Rust `SavedScreenState::encode` without decoding any business fields.
public enum EfssEnvelopeValidator {
    private static let magic: [UInt8] = [0x45, 0x46, 0x53, 0x53] // "EFSS"
    private static let formatVersionV1: UInt8 = 1
    private static let minFrameBytes: Int = 9
    private static let fnvOffsetBasis: UInt32 = 0x811c_9dc5
    private static let fnvPrime: UInt32 = 0x0100_0193

    public static func isValidV1Frame(_ data: Data) -> Bool {
        guard data.count >= minFrameBytes else {
            return false
        }
        let bytes = [UInt8](data)
        guard bytes[0] == magic[0],
              bytes[1] == magic[1],
              bytes[2] == magic[2],
              bytes[3] == magic[3],
              bytes[4] == formatVersionV1
        else {
            return false
        }

        let payloadEnd = bytes.count - 4
        let expectedChecksum =
            (UInt32(bytes[payloadEnd]) << 24)
            | (UInt32(bytes[payloadEnd + 1]) << 16)
            | (UInt32(bytes[payloadEnd + 2]) << 8)
            | UInt32(bytes[payloadEnd + 3])
        let actualChecksum = fnv1a32(bytes[..<payloadEnd])
        return expectedChecksum == actualChecksum
    }

    public static func fnv1a32(_ bytes: ArraySlice<UInt8>) -> UInt32 {
        var hash = fnvOffsetBasis
        for byte in bytes {
            hash ^= UInt32(byte)
            hash = hash &* fnvPrime
        }
        return hash
    }
}

/// Narrow C-ABI FFI surface implemented by the Rust static library (`libelysian_mobile.a`).
public protocol ElysianIosNativeBridge: AnyObject {
    func onLifecycleEvent(_ event: LifecycleEventTag)
    func shouldDraw() -> Bool
    func encodeSavedState() -> Data
    func restoreSavedState(_ bytes: Data) -> Bool
    func onDeepLink(uri: String, launchMode: LaunchModeTag)
    func onNotificationTap(
        notificationId: String,
        kind: NotificationKindTag,
        targetRecordId: String?,
        launchMode: LaunchModeTag
    )
    func onViewportChanged(_ spec: EfViewportSpecC)
}

/// Stateless iOS scene lifecycle forwarder bridging UIKit events directly into Rust.
public final class ElysianSceneLifecycleBridge {
    public static let stateRestorationActivityType = "fields.elysian.mobile.stateRestoration"
    public static let savedStateUserInfoKey = "elysian.saved_screen_state.v1"

    private let bridge: ElysianIosNativeBridge

    public init(bridge: ElysianIosNativeBridge) {
        self.bridge = bridge
    }

    public func sceneWillConnect(restorationActivity: NSUserActivity?, launchURL: URL?) {
        bridge.onLifecycleEvent(.coldLaunch)
        if let rawData = restorationActivity?.userInfo?[Self.savedStateUserInfoKey] as? Data,
           EfssEnvelopeValidator.isValidV1Frame(rawData)
        {
            _ = bridge.restoreSavedState(rawData)
        }
        if let url = launchURL {
            bridge.onDeepLink(uri: url.absoluteString, launchMode: .coldStart)
        }
    }

    public func sceneWillEnterForeground() {
        bridge.onLifecycleEvent(.resumeForeground)
    }

    public func sceneDidEnterBackground() {
        bridge.onLifecycleEvent(.enterBackground)
    }

    public func sceneDidDisconnect() {
        bridge.onLifecycleEvent(.osProcessTerminate)
    }

    public func sceneDidOpenURL(_ url: URL) {
        bridge.onDeepLink(uri: url.absoluteString, launchMode: .warmResume)
    }

    public func sceneDidContinueUserActivity(_ activity: NSUserActivity) {
        guard activity.activityType == NSUserActivityTypeBrowsingWeb,
              let url = activity.webpageURL
        else {
            return
        }
        bridge.onDeepLink(uri: url.absoluteString, launchMode: .warmResume)
    }

    public func buildStateRestorationActivity() -> NSUserActivity? {
        let encoded = bridge.encodeSavedState()
        guard EfssEnvelopeValidator.isValidV1Frame(encoded) else {
            return nil
        }
        let activity = NSUserActivity(activityType: Self.stateRestorationActivityType)
        activity.addUserInfoEntries(from: [Self.savedStateUserInfoKey: encoded])
        return activity
    }

    public func viewportDidChange(_ spec: EfViewportSpecC) {
        bridge.onViewportChanged(spec)
    }

    /// Checked before submitting any Metal drawable so background GPU usage is strictly `0`.
    public func shouldDraw() -> Bool {
        bridge.shouldDraw()
    }
}

#if canImport(UIKit)
@available(iOS 15.0, *)
public final class ElysianAppDelegate: UIResponder, UIApplicationDelegate, UISceneDelegate {
    public var lifecycleBridge: ElysianSceneLifecycleBridge?
    public var apnsHandler: ApnsHandler?

    public func application(
        _ application: UIApplication,
        didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data
    ) {
        apnsHandler?.didRegisterForRemoteNotifications(deviceToken: deviceToken)
    }

    public func scene(
        _ scene: UIScene,
        willConnectTo session: UISceneSession,
        options connectionOptions: UIScene.ConnectionOptions
    ) {
        let url = connectionOptions.urlContexts.first?.url
            ?? connectionOptions.userActivities.first?.webpageURL
        lifecycleBridge?.sceneWillConnect(
            restorationActivity: session.stateRestorationActivity,
            launchURL: url
        )
    }

    public func sceneWillEnterForeground(_ scene: UIScene) {
        lifecycleBridge?.sceneWillEnterForeground()
    }

    public func sceneDidEnterBackground(_ scene: UIScene) {
        lifecycleBridge?.sceneDidEnterBackground()
    }

    public func sceneDidDisconnect(_ scene: UIScene) {
        lifecycleBridge?.sceneDidDisconnect()
    }

    public func scene(_ scene: UIScene, openURLContexts URLContexts: Set<UIOpenURLContext>) {
        if let url = URLContexts.first?.url {
            lifecycleBridge?.sceneDidOpenURL(url)
        }
    }

    public func scene(_ scene: UIScene, continue userActivity: NSUserActivity) {
        lifecycleBridge?.sceneDidContinueUserActivity(userActivity)
    }

    public func stateRestorationActivity(for scene: UIScene) -> NSUserActivity? {
        lifecycleBridge?.buildStateRestorationActivity()
    }
}
#endif
