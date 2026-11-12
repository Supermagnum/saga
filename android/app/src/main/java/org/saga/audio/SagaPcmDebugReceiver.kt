package org.saga.audio

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.util.Log
import androidx.core.content.ContextCompat

/**
 * Mid-call PCM inject/dump without bringing MainActivity to the foreground
 * (which would cover the in-call UI and can disrupt Telecom audio focus).
 */
object SagaPcmDebugReceiver {
    private const val TAG = "[Saga PcmDebug]"

    const val ACTION_INJECT = "org.saga.TEST_PCM_INJECT"
    const val ACTION_DUMP = "org.saga.TEST_PCM_DUMP"
    const val ACTION_DUMP_FINISH = "org.saga.TEST_PCM_DUMP_FINISH"

    fun register(context: Context) {
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(ctx: Context, intent: Intent) {
                when (intent.action) {
                    ACTION_INJECT -> {
                        val path = intent.getStringExtra("path")
                        SagaPcmDebug.setInjectWav(path)
                        Log.i(TAG, "broadcast INJECT path=[$path]")
                    }
                    ACTION_DUMP -> {
                        val path = intent.getStringExtra("path")
                        SagaPcmDebug.setDumpPath(path)
                        Log.i(TAG, "broadcast DUMP path=[$path]")
                    }
                    ACTION_DUMP_FINISH -> {
                        SagaPcmDebug.finishDump()
                        Log.i(TAG, "broadcast DUMP_FINISH")
                    }
                }
            }
        }
        val filter = IntentFilter().apply {
            addAction(ACTION_INJECT)
            addAction(ACTION_DUMP)
            addAction(ACTION_DUMP_FINISH)
        }
        ContextCompat.registerReceiver(
            context.applicationContext,
            receiver,
            filter,
            ContextCompat.RECEIVER_EXPORTED
        )
        Log.i(TAG, "broadcast receiver registered")
    }
}
