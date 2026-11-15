package org.saga.audio

import android.content.Context

/**
 * Release stub: TEST_* PCM / echo / rx-fault broadcast receivers are debug-only.
 * Intentionally omits the org.saga.TEST_* action string constants so they do not
 * appear in the release APK string table.
 */
object SagaPcmDebugReceiver {
    fun register(context: Context) {
        // no-op — release builds must not export test broadcast hooks
    }
}
