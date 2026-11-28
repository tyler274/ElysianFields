package fields.elysian.mobile

/**
 * Pure Kotlin application configuration and route builder for Curie Mobile on Android.
 *
 * Holds zero Android SDK imports so CI compiles all `src/main/kotlin` sources cleanly
 * with `kotlinc` without requiring `android.jar`. Enforces the same security invariants
 * declared in `AndroidManifest.xml` (`allowBackup = false`, `usesCleartextTraffic = false`,
 * `/elysianfields/mobile-api` subpath mount, and explicit crisis `tel:` URIs).
 */
data class MobileAppConfig(
    val applicationId: String = DEFAULT_APPLICATION_ID,
    val apiBasePath: String = DEFAULT_API_BASE_PATH,
    val allowBackup: Boolean = false,
    val usesCleartextTraffic: Boolean = false,
    val crisisHelplineUris: List<String> = DEFAULT_CRISIS_HELPLINE_URIS,
) {
    /**
     * Resolves an API route under the canonical `/elysianfields/mobile-api` subpath.
     * Refuses cleartext `http://` origins unless targeting loopback (`127.0.0.1` / `localhost`)
     * when `usesCleartextTraffic` is disabled.
     */
    fun endpointUrl(origin: String, relativePath: String): String {
        val trimmedOrigin = origin.trim().trimEnd('/')
        require(trimmedOrigin.isNotEmpty()) { "origin must not be empty" }
        val isHttps = trimmedOrigin.startsWith("https://")
        val isLoopbackHttp =
            trimmedOrigin.startsWith("http://127.0.0.1") ||
                trimmedOrigin.startsWith("http://localhost")
        require(isHttps || usesCleartextTraffic || isLoopbackHttp) {
            "cleartext HTTP origin is forbidden for non-loopback endpoints"
        }

        val normalizedBase = "/" + apiBasePath.trim('/')
        val normalizedRel = relativePath.trimStart('/')
        return if (normalizedRel.isEmpty()) {
            "$trimmedOrigin$normalizedBase"
        } else {
            "$trimmedOrigin$normalizedBase/$normalizedRel"
        }
    }

    /**
     * Checks whether a URI is an allowed emergency/crisis `tel:` dial URI.
     */
    fun isAllowedCrisisUri(uri: String): Boolean {
        val normalized = uri.trim()
        return normalized.startsWith("tel:") && crisisHelplineUris.contains(normalized)
    }

    companion object {
        const val DEFAULT_APPLICATION_ID: String = "fields.elysian.mobile"
        const val DEFAULT_API_BASE_PATH: String = "/elysianfields/mobile-api"
        val DEFAULT_CRISIS_HELPLINE_URIS: List<String> = listOf(
            "tel:988",
            "tel:911",
            "tel:18006624357",
        )

        fun productionDefaults(): MobileAppConfig = MobileAppConfig()
    }
}
