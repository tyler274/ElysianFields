import Foundation
#if canImport(UserNotifications)
import UserNotifications
#endif

/// Validated wire payload extracted from an APNs notification tap.
public struct ApnsNotificationWire: Equatable {
    public let notificationId: String
    public let kind: NotificationKindTag
    public let targetRecordId: String?
    public let launchMode: LaunchModeTag

    public init(
        notificationId: String,
        kind: NotificationKindTag,
        targetRecordId: String?,
        launchMode: LaunchModeTag
    ) {
        self.notificationId = notificationId
        self.kind = kind
        self.targetRecordId = targetRecordId
        self.launchMode = launchMode
    }
}

/// Stateless APNs token & notification-tap forwarder matching `m1-mobile-lifecycle::notifications`.
///
/// Performs zero routing decisions, stores zero clinical payloads, and never logs APNs tokens.
public final class ApnsHandler {
    public static let notificationIdKey = "ef_notification_id"
    public static let notificationKindKey = "ef_notification_kind"
    public static let targetRecordIdKey = "ef_target_record_id"

    private let bridge: ElysianIosNativeBridge
    private let onPushTokenRotated: (String) -> Void

    public init(
        bridge: ElysianIosNativeBridge,
        onPushTokenRotated: @escaping (String) -> Void
    ) {
        self.bridge = bridge
        self.onPushTokenRotated = onPushTokenRotated
    }

    public func didRegisterForRemoteNotifications(deviceToken: Data) {
        guard !deviceToken.isEmpty else {
            return
        }
        let hexToken = deviceToken.map { String(format: "%02x", $0) }.joined()
        onPushTokenRotated(hexToken)
    }

    public func didReceiveNotificationTap(_ wire: ApnsNotificationWire) {
        bridge.onNotificationTap(
            notificationId: wire.notificationId,
            kind: wire.kind,
            targetRecordId: wire.targetRecordId,
            launchMode: wire.launchMode
        )
    }

    public static func parseUserInfo(
        _ userInfo: [AnyHashable: Any],
        launchMode: LaunchModeTag
    ) -> ApnsNotificationWire? {
        guard let rawId = userInfo[notificationIdKey] as? String else {
            return nil
        }
        let trimmedId = rawId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedId.isEmpty,
              let kind = NotificationKindTag.fromWireString(userInfo[notificationKindKey] as? String)
        else {
            return nil
        }
        let rawTarget = (userInfo[targetRecordIdKey] as? String)?
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let targetRecordId = (rawTarget?.isEmpty == false) ? rawTarget : nil
        return ApnsNotificationWire(
            notificationId: trimmedId,
            kind: kind,
            targetRecordId: targetRecordId,
            launchMode: launchMode
        )
    }
}

#if canImport(UserNotifications)
@available(iOS 15.0, macOS 12.0, *)
public final class ElysianNotificationCenterDelegate: NSObject, UNUserNotificationCenterDelegate {
    private let handler: ApnsHandler

    public init(handler: ApnsHandler) {
        self.handler = handler
        super.init()
    }

    public func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        let userInfo = response.notification.request.content.userInfo
        if let wire = ApnsHandler.parseUserInfo(userInfo, launchMode: .warmResume) {
            handler.didReceiveNotificationTap(wire)
        }
        completionHandler()
    }
}
#endif
