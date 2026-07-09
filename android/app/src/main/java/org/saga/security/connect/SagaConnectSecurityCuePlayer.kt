/*
 * Connect-time security audio cue (saga-spec.md section 5 step 4).
 * Distinct from org.saga.security.midcall.SagaMidCallSecurityAudioWarning (section 5b).
 */
package org.saga.security.connect

import android.content.Context
import android.media.MediaPlayer
import android.util.Log
import org.saga.R

object SagaConnectSecurityCuePlayer {
    private const val TAG = "[Saga Connect Security Cue]"

    fun playSecure(context: Context) = playOnce(context, R.raw.call_secure, "call_secure")

    fun playUnsecure(context: Context) = playOnce(context, R.raw.call_unsecure, "call_unsecure")

    private fun playOnce(context: Context, resId: Int, resourceName: String) {
        Log.i(TAG, "Playing $resourceName exactly once")
        val player = MediaPlayer.create(context.applicationContext, resId) ?: run {
            Log.e(TAG, "Failed to load raw resource [$resourceName]")
            return
        }
        player.setOnCompletionListener { it.release() }
        player.start()
    }
}
