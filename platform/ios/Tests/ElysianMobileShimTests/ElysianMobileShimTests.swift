import Foundation
import XCTest
@testable import ElysianMobileShim

private final class FakeIosBridge: ElysianIosNativeBridge {
    var lifecycleEvents: [LifecycleEventTag] = []
    var restoredPayloads: [Data] = []
    var deepLinks: [(uri: String, mode: LaunchModeTag)] = []
    var notificationTaps: [(id: String, kind: NotificationKindTag, target: String?, mode: LaunchModeTag)] = []
    var viewports: [EfViewportSpecC] = []
    var encodedStateToReturn: Data = Data()
    var foreground: Bool = true

    func onLifecycleEvent(_ event: LifecycleEventTag) {
        lifecycleEvents.append(event)
        switch event {
        case .coldLaunch, .resumeForeground:
            foreground = true
        case .enterBackground, .osProcessTerminate:
            foreground = false
        }
    }

    func shouldDraw() -> Bool {
        foreground
    }

    func encodeSavedState() -> Data {
        encodedStateToReturn
    }

    func restoreSavedState(_ bytes: Data) -> Bool {
        restoredPayloads.append(bytes)
        return true
    }

    func onDeepLink(uri: String, launchMode: LaunchModeTag) {
        deepLinks.append((uri, launchMode))
    }

    func onNotificationTap(
        notificationId: String,
        kind: NotificationKindTag,
        targetRecordId: String?,
        launchMode: LaunchModeTag
    ) {
        notificationTaps.append((notificationId, kind, targetRecordId, launchMode))
    }

    func onViewportChanged(_ spec: EfViewportSpecC) {
        viewports.append(spec)
    }
}

final class ElysianMobileShimTests: XCTestCase {
    // Canonical EFSS v1 binary frames produced by Rust `m1_mobile_lifecycle::SavedScreenState::encode`.
    private static let canonicalHomeDefault: [UInt8] = [
        0x45, 0x46, 0x53, 0x53, 0x01, 0x01, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x37,
        0x26, 0xfc, 0x82,
    ]

    private static let canonicalConversationRec42: [UInt8] = [
        0x45, 0x46, 0x53, 0x53, 0x01, 0x03, 0x00, 0x00,
        0x00, 0x06, 0x72, 0x65, 0x63, 0x2d, 0x34, 0x32,
        0x00, 0x00, 0x00, 0x08, 0x43, 0x68, 0x65, 0x63,
        0x6b, 0x20, 0x42, 0x50, 0x00, 0x00, 0x01, 0x40,
        0x01, 0x00, 0x00, 0x00, 0x06, 0x72, 0x65, 0x63,
        0x2d, 0x34, 0x32, 0x83, 0xc5, 0xf9, 0x68,
    ]

    private static let canonicalSignInColdDeepLink: [UInt8] = [
        0x45, 0x46, 0x53, 0x53, 0x01, 0x02, 0x02, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0xbc, 0x9f, 0x11, 0xe9,
    ]

    func testEfssV1CanonicalVectorsMatchRustByteForByte() {
        let homeData = Data(Self.canonicalHomeDefault)
        let convData = Data(Self.canonicalConversationRec42)
        let signInData = Data(Self.canonicalSignInColdDeepLink)

        XCTAssertTrue(EfssEnvelopeValidator.isValidV1Frame(homeData))
        XCTAssertTrue(EfssEnvelopeValidator.isValidV1Frame(convData))
        XCTAssertTrue(EfssEnvelopeValidator.isValidV1Frame(signInData))

        XCTAssertEqual(
            EfssEnvelopeValidator.fnv1a32(Self.canonicalHomeDefault[..<(Self.canonicalHomeDefault.count - 4)]),
            0x3726_fc82
        )
        XCTAssertEqual(
            EfssEnvelopeValidator.fnv1a32(Self.canonicalConversationRec42[..<(Self.canonicalConversationRec42.count - 4)]),
            0x83c5_f968
        )
        XCTAssertEqual(
            EfssEnvelopeValidator.fnv1a32(Self.canonicalSignInColdDeepLink[..<(Self.canonicalSignInColdDeepLink.count - 4)]),
            0xbc9f_11e9
        )
    }

    func testEfssV1RejectsCorruptedChecksumBadMagicWrongVersionAndTruncated() {
        var corrupted = Self.canonicalConversationRec42
        corrupted[10] ^= 0xff
        XCTAssertFalse(EfssEnvelopeValidator.isValidV1Frame(Data(corrupted)))

        var badMagic = Self.canonicalHomeDefault
        badMagic[0] = 0x00
        XCTAssertFalse(EfssEnvelopeValidator.isValidV1Frame(Data(badMagic)))

        var wrongVersion = Self.canonicalHomeDefault
        wrongVersion[4] = 0x02
        XCTAssertFalse(EfssEnvelopeValidator.isValidV1Frame(Data(wrongVersion)))

        XCTAssertFalse(EfssEnvelopeValidator.isValidV1Frame(Data(Self.canonicalHomeDefault.prefix(8))))
        XCTAssertFalse(EfssEnvelopeValidator.isValidV1Frame(Data()))
    }

    func testSceneLifecycleBridgeRestoresValidEfssAndEnforcesRenderGate() {
        let fake = FakeIosBridge()
        let canonicalBytes = Data(Self.canonicalConversationRec42)
        fake.encodedStateToReturn = canonicalBytes
        let sceneBridge = ElysianSceneLifecycleBridge(bridge: fake)

        // Build NSUserActivity containing validated EFSS v1 bytes.
        let activity = sceneBridge.buildStateRestorationActivity()
        XCTAssertNotNil(activity)
        XCTAssertEqual(
            activity?.userInfo?[ElysianSceneLifecycleBridge.savedStateUserInfoKey] as? Data,
            canonicalBytes
        )

        // Cold connect restores valid EFSS frame and forwards cold-start URL.
        let coldURL = URL(string: "https://dabney.moe/elysianfields/sign-in?token=syn_tok")!
        sceneBridge.sceneWillConnect(restorationActivity: activity, launchURL: coldURL)
        XCTAssertEqual(fake.lifecycleEvents, [.coldLaunch])
        XCTAssertEqual(fake.restoredPayloads, [canonicalBytes])
        XCTAssertEqual(fake.deepLinks.count, 1)
        XCTAssertEqual(fake.deepLinks[0].uri, coldURL.absoluteString)
        XCTAssertEqual(fake.deepLinks[0].mode, .coldStart)
        XCTAssertTrue(sceneBridge.shouldDraw())

        // Background pause enforces 0 frames rendered while hidden.
        sceneBridge.sceneDidEnterBackground()
        XCTAssertFalse(sceneBridge.shouldDraw())
        sceneBridge.sceneWillEnterForeground()
        XCTAssertTrue(sceneBridge.shouldDraw())

        // Corrupted restoration activity bytes must be ignored (never passed to Rust restore).
        var tampered = Self.canonicalConversationRec42
        tampered[12] ^= 0x55
        let badActivity = NSUserActivity(activityType: ElysianSceneLifecycleBridge.stateRestorationActivityType)
        badActivity.addUserInfoEntries(from: [ElysianSceneLifecycleBridge.savedStateUserInfoKey: Data(tampered)])
        sceneBridge.sceneWillConnect(restorationActivity: badActivity, launchURL: nil)
        XCTAssertEqual(fake.restoredPayloads.count, 1)

        // Invalid Rust snapshot returns nil activity instead of persisting garbage.
        fake.encodedStateToReturn = Data([0x01, 0x02, 0x03])
        XCTAssertNil(sceneBridge.buildStateRestorationActivity())
    }

    func testKeychainVaultRejectsEmptySecretAndUsesFiniteSlots() {
        let vault = KeychainVault(serviceIdentifier: "fields.elysian.mobile.test.vault.v1")
        XCTAssertThrowsError(try vault.store(key: .sessionRefreshToken, secret: [])) { error in
            XCTAssertEqual(error as? KeychainVaultError, .emptySecret)
        }
        XCTAssertEqual(SecureKeyTag.sessionRefreshToken.accountIdentifier, "session_refresh_token")
        XCTAssertEqual(SecureKeyTag.signInExchangeToken.accountIdentifier, "sign_in_exchange_token")
        XCTAssertEqual(SecureKeyTag.deviceBindingKey.accountIdentifier, "device_binding_key")
        XCTAssertEqual(SecureKeyTag.allCases.count, 3)
    }

    func testHealthKitBridgeHandlesUnavailablePlatformGracefullyWithoutLooping() {
        let bridge = HealthKitBridge()
        #if !(canImport(HealthKit) && os(iOS))
        XCTAssertFalse(bridge.isAvailable())
        XCTAssertEqual(bridge.permissionState(for: .heartRateBpm), .unavailable)

        let permExpectation = expectation(description: "permission unavailable")
        bridge.requestPermission(for: .heartRateBpm) { result in
            XCTAssertEqual(result, .success(.unavailable))
            permExpectation.fulfill()
        }

        let readExpectation = expectation(description: "read unavailable")
        bridge.readLatestSample(for: .heartRateBpm) { result in
            XCTAssertEqual(result, .failure(.platformUnavailable))
            readExpectation.fulfill()
        }
        wait(for: [permExpectation, readExpectation], timeout: 2.0)
        #else
        _ = bridge.permissionState(for: .heartRateBpm)
        #endif
    }

    func testApnsHandlerFormatsTokenAndForwardsValidatedTaps() {
        let fake = FakeIosBridge()
        var rotatedTokens: [String] = []
        let handler = ApnsHandler(bridge: fake) { token in
            rotatedTokens.append(token)
        }

        // Empty device token ignored; non-empty token formatted as lowercase hex without logging.
        handler.didRegisterForRemoteNotifications(deviceToken: Data())
        handler.didRegisterForRemoteNotifications(deviceToken: Data([0xde, 0xad, 0xbe, 0xef]))
        XCTAssertEqual(rotatedTokens, ["deadbeef"])

        let validInfo: [AnyHashable: Any] = [
            ApnsHandler.notificationIdKey: "notif-apns-1",
            ApnsHandler.notificationKindKey: "conversation",
            ApnsHandler.targetRecordIdKey: "rec-42",
        ]
        let parsed = ApnsHandler.parseUserInfo(validInfo, launchMode: .coldStart)
        XCTAssertEqual(
            parsed,
            ApnsNotificationWire(
                notificationId: "notif-apns-1",
                kind: .conversation,
                targetRecordId: "rec-42",
                launchMode: .coldStart
            )
        )
        if let wire = parsed {
            handler.didReceiveNotificationTap(wire)
        }
        XCTAssertEqual(fake.notificationTaps.count, 1)
        XCTAssertEqual(fake.notificationTaps[0].id, "notif-apns-1")
        XCTAssertEqual(fake.notificationTaps[0].kind, .conversation)
        XCTAssertEqual(fake.notificationTaps[0].target, "rec-42")
        XCTAssertEqual(fake.notificationTaps[0].mode, .coldStart)

        // Unknown notification kind or blank ID is rejected cleanly.
        let invalidInfo: [AnyHashable: Any] = [
            ApnsHandler.notificationIdKey: "notif-bad",
            ApnsHandler.notificationKindKey: "unknown_kind",
        ]
        XCTAssertNil(ApnsHandler.parseUserInfo(invalidInfo, launchMode: .warmResume))
    }
}
