package fields.elysian.mobile

/**
 * Stateless Android FCM push token & notification tap bridge matching
 * `m1-mobile-lifecycle::notifications`.
 *
 * Performs zero routing decisions, stores zero clinical payloads, and never logs
 * push registration tokens.
 */

object NotificationKindTag {
    const val CONVERSATION: Int = 1
    const val RECORD_DETAIL: Int = 2
    const val VITALS_OVERVIEW: Int = 3

    fun fromWireString(raw: String?): Int? = when (raw) {
        "conversation" -> CONVERSATION
        "record_detail" -> RECORD_DETAIL
        "vitals_overview" -> VITALS_OVERVIEW
        else -> null
    }
}

/**
 * Validated wire envelope extracted from FCM `RemoteMessage.data` / `PendingIntent` extras.
 */
data class FcmNotificationTap(
    val notificationId: String,
    val kindTag: Int,
    val targetRecordId: String?,
    val launchModeTag: Int,
)

/**
 * Callback delivering rotated opaque FCM device tokens to Rust for registration
 * with `/elysianfields/mobile-api/push/register`.
 */
fun interface PushTokenSink {
    fun onPushTokenRotated(token: String)
}

/**
 * Stateless bridge invoked by the Android `FirebaseMessagingService` and launch `Intent` handler.
 */
class ElysianFcmService(
    private val bridge: ElysianNativeBridge,
    private val tokenSink: PushTokenSink,
) {
    fun onNewToken(token: String) {
        val trimmed = token.trim()
        if (trimmed.isEmpty()) {
            return
        }
        tokenSink.onPushTokenRotated(trimmed)
    }

    fun onNotificationTap(tap: FcmNotificationTap) {
        bridge.nativeOnNotificationTap(
            notificationId = tap.notificationId,
            kindTag = tap.kindTag,
            targetRecordId = tap.targetRecordId,
            launchModeTag = tap.launchModeTag,
        )
    }

    fun onNotificationExtras(
        extras: Map<String, String>,
        launchModeTag: Int,
    ): Boolean {
        val id = extras[EXTRA_NOTIFICATION_ID]?.trim()
        val kindTag = NotificationKindTag.fromWireString(extras[EXTRA_NOTIFICATION_KIND])
        if (id.isNullOrEmpty() || kindTag == null) {
            return false
        }
        val targetRecordId = extras[EXTRA_TARGET_RECORD_ID]?.trim()?.ifEmpty { null }
        onNotificationTap(
            FcmNotificationTap(
                notificationId = id,
                kindTag = kindTag,
                targetRecordId = targetRecordId,
                launchModeTag = launchModeTag,
            ),
        )
        return true
    }

    companion object {
        const val EXTRA_NOTIFICATION_ID: String = "ef_notification_id"
        const val EXTRA_NOTIFICATION_KIND: String = "ef_notification_kind"
        const val EXTRA_TARGET_RECORD_ID: String = "ef_target_record_id"
    }
}
