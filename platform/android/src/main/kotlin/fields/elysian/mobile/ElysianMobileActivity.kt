package fields.elysian.mobile

/**
 * Stateless Android lifecycle, state-restoration, deep-link, and viewport shim.
 *
 * Rust (`m1-mobile-lifecycle` / `ef-mobile-client`) owns all product logic,
 * routing, draft text, render suspension (`should_draw()`), and `EFSS` v1 state
 * encoding/decoding. This Kotlin shell holds zero business state and never logs
 * credentials or patient data.
 */

object LifecycleEventTag {
    const val ENTER_BACKGROUND: Int = 1
    const val RESUME_FOREGROUND: Int = 2
    const val OS_PROCESS_TERMINATE: Int = 3
    const val COLD_LAUNCH: Int = 4
}

object LaunchModeTag {
    const val COLD_START: Int = 1
    const val WARM_RESUME: Int = 2
}

object OrientationTag {
    const val PORTRAIT: Int = 1
    const val LANDSCAPE_LEFT: Int = 2
    const val LANDSCAPE_RIGHT: Int = 3
    const val PORTRAIT_UPSIDE_DOWN: Int = 4
}

object PresentationTag {
    const val FULL_SCREEN: Int = 1
    const val SPLIT_SCREEN: Int = 2
}

object CutoutTag {
    const val NONE: Int = 0
    const val TOP_NOTCH: Int = 1
    const val DYNAMIC_ISLAND: Int = 2
    const val LANDSCAPE_SIDE: Int = 3
}

/**
 * Fixed 11-integer viewport geometry payload forwarded to Rust `ViewportInsets::compute`.
 */
data class ViewportMetricsWire(
    val widthPx: Int,
    val heightPx: Int,
    val scaleFactorMilli: Int,
    val orientationTag: Int,
    val presentationTag: Int,
    val paneFractionPercent: Int,
    val systemTopPx: Int,
    val systemRightPx: Int,
    val systemBottomPx: Int,
    val systemLeftPx: Int,
    val cutoutTag: Int,
    val cutoutPrimaryPx: Int,
    val cutoutSecondaryPx: Int,
    val softKeyboardHeightPx: Int,
)

/**
 * Validates the `EFSS` v1 binary envelope (magic + version + FNV-1a-32 checksum)
 * produced by Rust `SavedScreenState::encode` without decoding any business fields.
 */
object EfssEnvelopeValidator {
    private const val MAGIC_0: Byte = 0x45 // 'E'
    private const val MAGIC_1: Byte = 0x46 // 'F'
    private const val MAGIC_2: Byte = 0x53 // 'S'
    private const val MAGIC_3: Byte = 0x53 // 'S'
    private const val FORMAT_VERSION_V1: Byte = 1
    private const val MIN_FRAME_BYTES: Int = 9 // 4 magic + 1 version + 4 checksum
    private const val FNV1A_OFFSET_BASIS: UInt = 0x811c9dc5u
    private const val FNV1A_PRIME: UInt = 0x01000193u

    fun isValidV1Frame(bytes: ByteArray): Boolean {
        if (bytes.size < MIN_FRAME_BYTES) {
            return false
        }
        if (bytes[0] != MAGIC_0 ||
            bytes[1] != MAGIC_1 ||
            bytes[2] != MAGIC_2 ||
            bytes[3] != MAGIC_3
        ) {
            return false
        }
        if (bytes[4] != FORMAT_VERSION_V1) {
            return false
        }
        val payloadEnd = bytes.size - 4
        val expectedChecksum = readBeU32(bytes, payloadEnd)
        val actualChecksum = fnv1a32(bytes, payloadEnd)
        return expectedChecksum == actualChecksum
    }

    fun fnv1a32(bytes: ByteArray, length: Int = bytes.size): UInt {
        var hash = FNV1A_OFFSET_BASIS
        for (i in 0 until length) {
            hash = hash xor (bytes[i].toInt() and 0xff).toUInt()
            hash *= FNV1A_PRIME
        }
        return hash
    }

    private fun readBeU32(bytes: ByteArray, offset: Int): UInt {
        val b0 = (bytes[offset].toInt() and 0xff).toUInt() shl 24
        val b1 = (bytes[offset + 1].toInt() and 0xff).toUInt() shl 16
        val b2 = (bytes[offset + 2].toInt() and 0xff).toUInt() shl 8
        val b3 = (bytes[offset + 3].toInt() and 0xff).toUInt()
        return b0 or b1 or b2 or b3
    }
}

/**
 * Narrow JNI boundary contract implemented by the Rust native library (`libelysian_mobile.so`).
 */
interface ElysianNativeBridge {
    fun nativeOnLifecycleEvent(eventTag: Int)
    fun nativeShouldDraw(): Boolean
    fun nativeEncodeSavedState(): ByteArray
    fun nativeRestoreSavedState(bytes: ByteArray): Boolean
    fun nativeOnDeepLink(uri: String, launchModeTag: Int)
    fun nativeOnNotificationTap(
        notificationId: String,
        kindTag: Int,
        targetRecordId: String?,
        launchModeTag: Int,
    )

    fun nativeOnViewportChanged(metrics: ViewportMetricsWire)
}

/**
 * Direct JNI implementation binding to `libelysian_mobile.so`.
 */
class JniNativeBridge : ElysianNativeBridge {
    override external fun nativeOnLifecycleEvent(eventTag: Int)
    override external fun nativeShouldDraw(): Boolean
    override external fun nativeEncodeSavedState(): ByteArray
    override external fun nativeRestoreSavedState(bytes: ByteArray): Boolean
    override external fun nativeOnDeepLink(uri: String, launchModeTag: Int)
    override external fun nativeOnNotificationTap(
        notificationId: String,
        kindTag: Int,
        targetRecordId: String?,
        launchModeTag: Int,
    )

    override fun nativeOnViewportChanged(metrics: ViewportMetricsWire) {
        nativeOnViewportChangedRaw(
            metrics.widthPx,
            metrics.heightPx,
            metrics.scaleFactorMilli,
            metrics.orientationTag,
            metrics.presentationTag,
            metrics.paneFractionPercent,
            metrics.systemTopPx,
            metrics.systemRightPx,
            metrics.systemBottomPx,
            metrics.systemLeftPx,
            metrics.cutoutTag,
            metrics.cutoutPrimaryPx,
            metrics.cutoutSecondaryPx,
            metrics.softKeyboardHeightPx,
        )
    }

    private external fun nativeOnViewportChangedRaw(
        widthPx: Int,
        heightPx: Int,
        scaleFactorMilli: Int,
        orientationTag: Int,
        presentationTag: Int,
        paneFractionPercent: Int,
        systemTopPx: Int,
        systemRightPx: Int,
        systemBottomPx: Int,
        systemLeftPx: Int,
        cutoutTag: Int,
        cutoutPrimaryPx: Int,
        cutoutSecondaryPx: Int,
        softKeyboardHeightPx: Int,
    )

    companion object {
        fun loadNativeLibrary() {
            System.loadLibrary("elysian_mobile")
        }
    }
}

/**
 * Minimal OS restoration bundle abstraction mapped to Android `android.os.Bundle`
 * by the Gradle application wrapper.
 */
interface RestorableStateBundle {
    fun getByteArray(key: String): ByteArray?
    fun putByteArray(key: String, value: ByteArray)
}

/**
 * Stateless Android Activity host shim forwarding OS lifecycle, `EFSS` v1 state
 * restoration, App Links / `curie://` intents, and window insets strictly into Rust.
 */
class ElysianMobileActivity(
    private val bridge: ElysianNativeBridge,
) {
    fun onCreate(savedInstanceState: RestorableStateBundle?, launchUri: String?) {
        bridge.nativeOnLifecycleEvent(LifecycleEventTag.COLD_LAUNCH)
        val rawSnapshot = savedInstanceState?.getByteArray(SAVED_STATE_BUNDLE_KEY)
        if (rawSnapshot != null && EfssEnvelopeValidator.isValidV1Frame(rawSnapshot)) {
            bridge.nativeRestoreSavedState(rawSnapshot)
        }
        if (!launchUri.isNullOrEmpty()) {
            bridge.nativeOnDeepLink(launchUri, LaunchModeTag.COLD_START)
        }
    }

    fun onResume() {
        bridge.nativeOnLifecycleEvent(LifecycleEventTag.RESUME_FOREGROUND)
    }

    fun onStop() {
        bridge.nativeOnLifecycleEvent(LifecycleEventTag.ENTER_BACKGROUND)
    }

    fun onDestroy() {
        bridge.nativeOnLifecycleEvent(LifecycleEventTag.OS_PROCESS_TERMINATE)
    }

    fun onNewIntent(uri: String?) {
        if (!uri.isNullOrEmpty()) {
            bridge.nativeOnDeepLink(uri, LaunchModeTag.WARM_RESUME)
        }
    }

    fun onSaveInstanceState(outState: RestorableStateBundle): Boolean {
        val encoded = bridge.nativeEncodeSavedState()
        if (!EfssEnvelopeValidator.isValidV1Frame(encoded)) {
            return false
        }
        outState.putByteArray(SAVED_STATE_BUNDLE_KEY, encoded)
        return true
    }

    fun onWindowInsetsChanged(metrics: ViewportMetricsWire) {
        bridge.nativeOnViewportChanged(metrics)
    }

    /**
     * Gate checked by `SurfaceView` / `GameActivity` before submitting any GPU frame.
     * Guarantees `0` frames rendered while backgrounded or terminated.
     */
    fun shouldDraw(): Boolean = bridge.nativeShouldDraw()

    companion object {
        const val SAVED_STATE_BUNDLE_KEY: String = "elysian.saved_screen_state.v1"
    }
}
