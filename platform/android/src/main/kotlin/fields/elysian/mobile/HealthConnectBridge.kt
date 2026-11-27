package fields.elysian.mobile

/**
 * Stateless Android Health Connect adapter shim matching `m1-mobile-lifecycle::health`.
 *
 * Enforces single-shot permission prompting (no prompt loops when denied) and
 * fail-closed sample reads (no read attempted unless permission is `GRANTED`).
 * Holds zero clinical records or PHI.
 */

object HealthMetricTag {
    const val HEART_RATE_BPM: Int = 1
    const val STEPS_COUNT: Int = 2
    const val BLOOD_PRESSURE_MMHG: Int = 3
    const val SLEEP_DURATION_MINUTES: Int = 4
}

object HealthPermissionStateTag {
    const val NOT_DETERMINED: Int = 0
    const val GRANTED: Int = 1
    const val DENIED: Int = 2
    const val UNAVAILABLE: Int = 3
}

/**
 * Typed numeric sample payload forwarded from Jetpack Health Connect into Rust.
 */
data class HealthSampleWire(
    val metricTag: Int,
    val primaryValue: Long,
    val secondaryValue: Long = 0L,
    val recordedAtEpochSecs: Long,
)

sealed class HealthReadOutcome {
    data class Sample(val sample: HealthSampleWire) : HealthReadOutcome()
    data object NoData : HealthReadOutcome()
    data class PermissionRequired(val metricTag: Int, val stateTag: Int) : HealthReadOutcome()
    data object PlatformUnavailable : HealthReadOutcome()
}

/**
 * Narrow OS host delegate bound to Jetpack `androidx.health.connect.client.HealthConnectClient`
 * by the Android Gradle application module.
 */
interface HealthConnectProvider {
    fun isSdkAvailable(): Boolean
    fun queryPermissionState(metricTag: Int): Int
    fun launchPermissionRequest(metricTag: Int): Int
    fun readLatestRecord(metricTag: Int): HealthSampleWire?
}

/**
 * Stateless Health Connect shim invoked exclusively by the Rust background sync worker
 * (never directly from UI rendering code).
 */
class HealthConnectBridge(
    private val provider: HealthConnectProvider?,
) {
    fun permissionState(metricTag: Int): Int {
        val host = provider ?: return HealthPermissionStateTag.UNAVAILABLE
        if (!host.isSdkAvailable()) {
            return HealthPermissionStateTag.UNAVAILABLE
        }
        return host.queryPermissionState(metricTag)
    }

    /**
     * Requests Health Connect permission once when `NOT_DETERMINED`.
     * Never re-prompts in a loop when already `DENIED`, `GRANTED`, or `UNAVAILABLE`.
     */
    fun requestPermission(metricTag: Int): Int {
        val host = provider ?: return HealthPermissionStateTag.UNAVAILABLE
        if (!host.isSdkAvailable()) {
            return HealthPermissionStateTag.UNAVAILABLE
        }
        return when (val current = host.queryPermissionState(metricTag)) {
            HealthPermissionStateTag.NOT_DETERMINED -> host.launchPermissionRequest(metricTag)
            HealthPermissionStateTag.GRANTED,
            HealthPermissionStateTag.DENIED,
            HealthPermissionStateTag.UNAVAILABLE,
            -> current
            else -> HealthPermissionStateTag.UNAVAILABLE
        }
    }

    /**
     * Reads the latest record for `metricTag`, failing closed if permission is not `GRANTED`.
     */
    fun readLatestSample(metricTag: Int): HealthReadOutcome {
        val host = provider ?: return HealthReadOutcome.PlatformUnavailable
        if (!host.isSdkAvailable()) {
            return HealthReadOutcome.PlatformUnavailable
        }
        val state = host.queryPermissionState(metricTag)
        if (state == HealthPermissionStateTag.UNAVAILABLE) {
            return HealthReadOutcome.PlatformUnavailable
        }
        if (state != HealthPermissionStateTag.GRANTED) {
            return HealthReadOutcome.PermissionRequired(metricTag, state)
        }
        val sample = host.readLatestRecord(metricTag) ?: return HealthReadOutcome.NoData
        return HealthReadOutcome.Sample(sample)
    }
}
