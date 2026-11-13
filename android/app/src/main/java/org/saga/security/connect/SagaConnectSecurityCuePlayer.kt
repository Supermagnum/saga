/*
 * Connect-time security audio cue (saga-spec.md section 5 step 4).
 * Distinct from org.saga.security.midcall.SagaMidCallSecurityAudioWarning (section 5b).
 */
package org.saga.security.connect

import android.content.Context
import android.media.MediaPlayer
import android.os.SystemClock
import android.util.Log
import org.saga.R
import java.util.concurrent.atomic.AtomicLong

object SagaConnectSecurityCuePlayer {
    private const val TAG = "[Saga Connect Security Cue]"
    /** Ignore duplicate play requests within this window (API 29 can double-fire connect). */
    private const val DEDUPE_MS = 2500L
    private val lastPlayElapsedMs = AtomicLong(0L)
    private val lastResource = AtomicLong(0L)

    fun playSecure(context: Context) = playOnce(context, R.raw.call_secure, "call_secure")

    fun playUnsecure(context: Context) = playOnce(context, R.raw.call_unsecure, "call_unsecure")

    private fun playOnce(context: Context, resId: Int, resourceName: String) {
        val now = SystemClock.elapsedRealtime()
        val prevT = lastPlayElapsedMs.get()
        val prevR = lastResource.get()
        if (prevR == resId.toLong() && now - prevT < DEDUPE_MS) {
            Log.i(TAG, "Skipping duplicate $resourceName within ${DEDUPE_MS}ms")
            return
        }
        lastPlayElapsedMs.set(now)
        lastResource.set(resId.toLong())
        Log.i(TAG, "Playing $resourceName exactly once")
        val player = MediaPlayer.create(context.applicationContext, resId) ?: run {
            Log.e(TAG, "Failed to load raw resource [$resourceName]")
            return
        }
        player.setOnCompletionListener { it.release() }
        player.start()
    }
}
