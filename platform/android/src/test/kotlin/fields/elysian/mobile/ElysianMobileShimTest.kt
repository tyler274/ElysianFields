package fields.elysian.mobile

import java.security.GeneralSecurityException
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey

/**
 * Standalone executable verification harness for the Android Kotlin mobile OS shims.
 *
 * Requires zero external JUnit JARs: compiles with `kotlinc` and executes directly
 * under `java`, throwing `AssertionError` on any mismatch. Proves byte-for-byte
 * `EFSS` v1 parity with Rust `m1-mobile-lifecycle` and iOS `ElysianMobileShim`.
 */

private fun check(condition: Boolean, message: String) {
    if (!condition) {
        throw AssertionError("FAILED: $message")
    }
}

private fun unsignedBytes(vararg values: Int): ByteArray =
    ByteArray(values.size) { index -> values[index].toByte() }

private val CANONICAL_HOME_DEFAULT: ByteArray = unsignedBytes(
    0x45, 0x46, 0x53, 0x53, 0x01, 0x01, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x37,
    0x26, 0xfc, 0x82,
)

private val CANONICAL_CONVERSATION_REC42: ByteArray = unsignedBytes(
    0x45, 0x46, 0x53, 0x53, 0x01, 0x03, 0x00, 0x00,
    0x00, 0x06, 0x72, 0x65, 0x63, 0x2d, 0x34, 0x32,
    0x00, 0x00, 0x00, 0x08, 0x43, 0x68, 0x65, 0x63,
    0x6b, 0x20, 0x42, 0x50, 0x00, 0x00, 0x01, 0x40,
    0x01, 0x00, 0x00, 0x00, 0x06, 0x72, 0x65, 0x63,
    0x2d, 0x34, 0x32, 0x83, 0xc5, 0xf9, 0x68,
)

private val CANONICAL_SIGN_IN_COLD_DEEP_LINK: ByteArray = unsignedBytes(
    0x45, 0x46, 0x53, 0x53, 0x01, 0x02, 0x02, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0xbc, 0x9f, 0x11, 0xe9,
)

private class FakeNativeBridge : ElysianNativeBridge {
    val lifecycleEvents: MutableList<Int> = mutableListOf()
    val restoredPayloads: MutableList<ByteArray> = mutableListOf()
    val deepLinks: MutableList<Pair<String, Int>> = mutableListOf()
    val notificationTaps: MutableList<FcmNotificationTap> = mutableListOf()
    val viewports: MutableList<ViewportMetricsWire> = mutableListOf()
    var encodedStateToReturn: ByteArray = CANONICAL_CONVERSATION_REC42
    private var foreground: Boolean = true

    override fun nativeOnLifecycleEvent(eventTag: Int) {
        lifecycleEvents.add(eventTag)
        foreground = when (eventTag) {
            LifecycleEventTag.COLD_LAUNCH, LifecycleEventTag.RESUME_FOREGROUND -> true
            LifecycleEventTag.ENTER_BACKGROUND, LifecycleEventTag.OS_PROCESS_TERMINATE -> false
            else -> false
        }
    }

    override fun nativeShouldDraw(): Boolean = foreground

    override fun nativeEncodeSavedState(): ByteArray = encodedStateToReturn.copyOf()

    override fun nativeRestoreSavedState(bytes: ByteArray): Boolean {
        restoredPayloads.add(bytes.copyOf())
        return true
    }

    override fun nativeOnDeepLink(uri: String, launchModeTag: Int) {
        deepLinks.add(uri to launchModeTag)
    }

    override fun nativeOnNotificationTap(
        notificationId: String,
        kindTag: Int,
        targetRecordId: String?,
        launchModeTag: Int,
    ) {
        notificationTaps.add(FcmNotificationTap(notificationId, kindTag, targetRecordId, launchModeTag))
    }

    override fun nativeOnViewportChanged(metrics: ViewportMetricsWire) {
        viewports.add(metrics)
    }
}

private class FakeBundle : RestorableStateBundle {
    private val map: MutableMap<String, ByteArray> = mutableMapOf()
    override fun getByteArray(key: String): ByteArray? = map[key]?.copyOf()
    override fun putByteArray(key: String, value: ByteArray) {
        map[key] = value.copyOf()
    }
}

private class EphemeralAesMasterKeyProvider : MasterKeyProvider {
    private val secretKey: SecretKey = KeyGenerator.getInstance("AES").apply { init(256) }.generateKey()
    override fun getOrCreateMasterKey(): SecretKey = secretKey
}

private class UserNotAuthenticatedException(msg: String) : GeneralSecurityException(msg)

private class LockedMasterKeyProvider : MasterKeyProvider {
    override fun getOrCreateMasterKey(): SecretKey {
        throw UserNotAuthenticatedException("device locked")
    }
}

private class FakeHealthConnectProvider(
    private val available: Boolean = true,
) : HealthConnectProvider {
    var state: Int = HealthPermissionStateTag.NOT_DETERMINED
    var promptDecision: Int = HealthPermissionStateTag.DENIED
    var promptInvocations: Int = 0
    var sampleToReturn: HealthSampleWire? = null

    override fun isSdkAvailable(): Boolean = available
    override fun queryPermissionState(metricTag: Int): Int = state
    override fun launchPermissionRequest(metricTag: Int): Int {
        promptInvocations += 1
        state = promptDecision
        return state
    }

    override fun readLatestRecord(metricTag: Int): HealthSampleWire? = sampleToReturn
}

private fun testEfssV1CanonicalParity() {
    check(EfssEnvelopeValidator.isValidV1Frame(CANONICAL_HOME_DEFAULT), "home_default frame must validate")
    check(EfssEnvelopeValidator.isValidV1Frame(CANONICAL_CONVERSATION_REC42), "conversation_rec42 frame must validate")
    check(EfssEnvelopeValidator.isValidV1Frame(CANONICAL_SIGN_IN_COLD_DEEP_LINK), "sign_in_cold_deep_link frame must validate")

    check(
        EfssEnvelopeValidator.fnv1a32(CANONICAL_HOME_DEFAULT, CANONICAL_HOME_DEFAULT.size - 4) == 0x3726fc82u,
        "home_default FNV-1a-32 checksum mismatch",
    )
    check(
        EfssEnvelopeValidator.fnv1a32(CANONICAL_CONVERSATION_REC42, CANONICAL_CONVERSATION_REC42.size - 4) == 0x83c5f968u,
        "conversation_rec42 FNV-1a-32 checksum mismatch",
    )
    check(
        EfssEnvelopeValidator.fnv1a32(CANONICAL_SIGN_IN_COLD_DEEP_LINK, CANONICAL_SIGN_IN_COLD_DEEP_LINK.size - 4) == 0xbc9f11e9u,
        "sign_in_cold_deep_link FNV-1a-32 checksum mismatch",
    )

    val corrupted = CANONICAL_CONVERSATION_REC42.copyOf()
    corrupted[10] = (corrupted[10].toInt() xor 0xff).toByte()
    check(!EfssEnvelopeValidator.isValidV1Frame(corrupted), "corrupted checksum must be rejected")

    val badMagic = CANONICAL_HOME_DEFAULT.copyOf()
    badMagic[0] = 0x00
    check(!EfssEnvelopeValidator.isValidV1Frame(badMagic), "bad magic must be rejected")

    val wrongVersion = CANONICAL_HOME_DEFAULT.copyOf()
    wrongVersion[4] = 0x02
    check(!EfssEnvelopeValidator.isValidV1Frame(wrongVersion), "unsupported version must be rejected")

    check(!EfssEnvelopeValidator.isValidV1Frame(CANONICAL_HOME_DEFAULT.copyOf(8)), "truncated frame must be rejected")
}

private fun testActivityLifecycleAndRestore() {
    val bridge = FakeNativeBridge()
    val activity = ElysianMobileActivity(bridge)
    val bundle = FakeBundle()

    check(activity.onSaveInstanceState(bundle), "valid EFSS v1 snapshot must save")
    val saved = bundle.getByteArray(ElysianMobileActivity.SAVED_STATE_BUNDLE_KEY)
    check(saved != null && saved.contentEquals(CANONICAL_CONVERSATION_REC42), "saved bundle bytes must match Rust EFSS v1")

    activity.onCreate(bundle, "curie://sign-in?token=syn_cold_tok")
    check(bridge.lifecycleEvents == listOf(LifecycleEventTag.COLD_LAUNCH), "cold launch event expected")
    check(bridge.restoredPayloads.size == 1 && bridge.restoredPayloads[0].contentEquals(CANONICAL_CONVERSATION_REC42), "restored bytes mismatch")
    check(bridge.deepLinks == listOf("curie://sign-in?token=syn_cold_tok" to LaunchModeTag.COLD_START), "cold deep link mismatch")
    check(activity.shouldDraw(), "foreground shouldDraw must be true")

    activity.onStop()
    check(!activity.shouldDraw(), "background shouldDraw must be false (0 frames hidden)")
    activity.onResume()
    check(activity.shouldDraw(), "resumed shouldDraw must be true")

    activity.onNewIntent("https://dabney.moe/elysianfields/sign-in?token=syn_warm_tok")
    check(bridge.deepLinks.size == 2 && bridge.deepLinks[1].second == LaunchModeTag.WARM_RESUME, "warm deep link mismatch")

    // Corrupted bundle must be ignored safely.
    val badBundle = FakeBundle()
    val tampered = CANONICAL_CONVERSATION_REC42.copyOf()
    tampered[12] = (tampered[12].toInt() xor 0x44).toByte()
    badBundle.putByteArray(ElysianMobileActivity.SAVED_STATE_BUNDLE_KEY, tampered)
    activity.onCreate(badBundle, null)
    check(bridge.restoredPayloads.size == 1, "corrupted EFSS frame must not be passed to Rust")
}

private fun testKeystoreVault() {
    val blobStore = InMemoryVaultBlobStore()
    val vault = KeystoreVault(blobStore, EphemeralAesMasterKeyProvider())

    val secretText = "syn_refresh_token_secret_payload_012345"
    val secretBytes = secretText.encodeToByteArray()

    check(vault.store(SecureKeyTag.SESSION_REFRESH_TOKEN, ByteArray(0)) == KeystoreVaultResult.EmptySecret, "empty secret must be refused")
    check(vault.store(SecureKeyTag.SESSION_REFRESH_TOKEN, secretBytes) == KeystoreVaultResult.Ok(Unit), "AES-GCM store must succeed")

    // Ciphertext stored in blob store must not contain plaintext secret bytes.
    val rawCiphertext = blobStore.readBlob(SecureKeyTag.SESSION_REFRESH_TOKEN.slotName)
    check(rawCiphertext != null && !rawCiphertext.decodeToString().contains(secretText), "ciphertext must not leak plaintext")

    val loaded = vault.load(SecureKeyTag.SESSION_REFRESH_TOKEN)
    check(loaded is KeystoreVaultResult.Ok && loaded.value != null && loaded.value.contentEquals(secretBytes), "AES-GCM load must round-trip exact bytes")

    check(vault.delete(SecureKeyTag.SESSION_REFRESH_TOKEN) == KeystoreVaultResult.Ok(Unit), "delete must succeed")
    val afterDelete = vault.load(SecureKeyTag.SESSION_REFRESH_TOKEN)
    check(afterDelete is KeystoreVaultResult.Ok && afterDelete.value == null, "load after delete must return null")

    val lockedVault = KeystoreVault(blobStore, LockedMasterKeyProvider())
    check(lockedVault.store(SecureKeyTag.DEVICE_BINDING_KEY, secretBytes) == KeystoreVaultResult.DeviceLocked, "locked keystore must map to DeviceLocked")
}

private fun testHealthConnectBridge() {
    val unavailableBridge = HealthConnectBridge(null)
    check(unavailableBridge.permissionState(HealthMetricTag.HEART_RATE_BPM) == HealthPermissionStateTag.UNAVAILABLE, "null provider must be UNAVAILABLE")
    check(unavailableBridge.readLatestSample(HealthMetricTag.HEART_RATE_BPM) == HealthReadOutcome.PlatformUnavailable, "null provider read must be PlatformUnavailable")

    val provider = FakeHealthConnectProvider(available = true)
    val bridge = HealthConnectBridge(provider)
    check(bridge.permissionState(HealthMetricTag.HEART_RATE_BPM) == HealthPermissionStateTag.NOT_DETERMINED, "initial state NOT_DETERMINED")

    // Denied permission must prompt once and NEVER loop.
    check(bridge.requestPermission(HealthMetricTag.HEART_RATE_BPM) == HealthPermissionStateTag.DENIED, "first prompt returns DENIED")
    check(bridge.requestPermission(HealthMetricTag.HEART_RATE_BPM) == HealthPermissionStateTag.DENIED, "second request returns DENIED without re-prompt")
    check(provider.promptInvocations == 1, "denied permission must never loop prompts")
    check(
        bridge.readLatestSample(HealthMetricTag.HEART_RATE_BPM) ==
            HealthReadOutcome.PermissionRequired(HealthMetricTag.HEART_RATE_BPM, HealthPermissionStateTag.DENIED),
        "read while denied must fail closed",
    )

    // Granted permission reads sample cleanly.
    val sample = HealthSampleWire(HealthMetricTag.HEART_RATE_BPM, 68L, 0L, 1_760_000_050L)
    provider.state = HealthPermissionStateTag.GRANTED
    provider.sampleToReturn = sample
    check(bridge.readLatestSample(HealthMetricTag.HEART_RATE_BPM) == HealthReadOutcome.Sample(sample), "granted read must return sample")
}

private fun testFcmService() {
    val bridge = FakeNativeBridge()
    val rotated = mutableListOf<String>()
    val fcm = ElysianFcmService(bridge) { token -> rotated.add(token) }

    fcm.onNewToken("   ")
    fcm.onNewToken(" fcm-opaque-token-1 ")
    check(rotated == listOf("fcm-opaque-token-1"), "FCM token rotation forwarding mismatch")

    val ok = fcm.onNotificationExtras(
        mapOf(
            ElysianFcmService.EXTRA_NOTIFICATION_ID to "notif-fcm-1",
            ElysianFcmService.EXTRA_NOTIFICATION_KIND to "conversation",
            ElysianFcmService.EXTRA_TARGET_RECORD_ID to "rec-42",
        ),
        LaunchModeTag.COLD_START,
    )
    check(ok, "valid FCM extras must forward")
    check(bridge.notificationTaps.size == 1 && bridge.notificationTaps[0].targetRecordId == "rec-42", "FCM tap forwarding mismatch")

    val invalid = fcm.onNotificationExtras(
        mapOf(
            ElysianFcmService.EXTRA_NOTIFICATION_ID to "notif-bad",
            ElysianFcmService.EXTRA_NOTIFICATION_KIND to "unknown",
        ),
        LaunchModeTag.WARM_RESUME,
    )
    check(!invalid, "unknown FCM kind must return false")
}

private fun testMobileAppConfig() {
    val cfg = MobileAppConfig.productionDefaults()
    check(cfg.applicationId == "fields.elysian.mobile", "applicationId mismatch")
    check(cfg.apiBasePath == "/elysianfields/mobile-api", "apiBasePath mismatch")
    check(!cfg.allowBackup, "allowBackup must be false")
    check(!cfg.usesCleartextTraffic, "usesCleartextTraffic must be false")
    check(
        cfg.endpointUrl("https://app.example.invalid/", "readyz") ==
            "https://app.example.invalid/elysianfields/mobile-api/readyz",
        "HTTPS endpoint resolution mismatch",
    )
    check(
        cfg.endpointUrl("http://127.0.0.1:8089", "/metrics") ==
            "http://127.0.0.1:8089/elysianfields/mobile-api/metrics",
        "loopback dev endpoint resolution mismatch",
    )
    var rejectedCleartext = false
    try {
        cfg.endpointUrl("http://public.example.invalid", "readyz")
    } catch (_: IllegalArgumentException) {
        rejectedCleartext = true
    }
    check(rejectedCleartext, "non-loopback cleartext HTTP origin must be rejected")
    check(cfg.isAllowedCrisisUri("tel:988"), "tel:988 must be allowed")
    check(cfg.isAllowedCrisisUri("tel:911"), "tel:911 must be allowed")
    check(!cfg.isAllowedCrisisUri("http://988"), "non-tel crisis URI must be rejected")
}

fun main() {
    testEfssV1CanonicalParity()
    println("PASS: testEfssV1CanonicalParity")
    testActivityLifecycleAndRestore()
    println("PASS: testActivityLifecycleAndRestore")
    testKeystoreVault()
    println("PASS: testKeystoreVault")
    testHealthConnectBridge()
    println("PASS: testHealthConnectBridge")
    testFcmService()
    println("PASS: testFcmService")
    testMobileAppConfig()
    println("PASS: testMobileAppConfig")
    println("ALL ANDROID KOTLIN SHIM TESTS PASSED (6/6)")
}
