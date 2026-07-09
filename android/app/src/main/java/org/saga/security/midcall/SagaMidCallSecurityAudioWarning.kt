package org.saga.security.midcall

import android.content.Context
import android.media.MediaPlayer
import android.util.Log
import org.saga.R

object SagaMidCallSecurityAudioWarning {
    const val PREFS_NAME = "saga_mid_call_security"
    const val KEY_PLAY_AUDIO_ON_SECURITY_LOSS = "play_audio_on_mid_call_security_loss"
    private const val TAG = "[Saga Mid-Call Security Warning]"

    fun isEnabled(context: Context): Boolean {
        return context.applicationContext
            .getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
            .getBoolean(KEY_PLAY_AUDIO_ON_SECURITY_LOSS, true)
    }

    fun playWarning(context: Context) {
        if (!isEnabled(context)) return
        Log.i(TAG, "Playing mid_call_security_warning exactly once (resId=${R.raw.mid_call_security_warning})")
        val player = MediaPlayer.create(context.applicationContext, R.raw.mid_call_security_warning)
            ?: return
        player.setOnCompletionListener { it.release() }
        player.start()
    }
}
