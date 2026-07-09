package org.saga.iroh

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import androidx.core.content.ContextCompat

object IrohRelayStatus {
    const val ACTION_QUERY = "org.saga.TEST_RELAY_QUERY"
    const val STATUS_FILE = "relay_status.txt"

    const val RELAY_PENDING = 0
    const val RELAY_READY = 1
    const val RELAY_FAILED = 2

    fun registerQueryReceiver(context: Context) {
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(ctx: Context, intent: Intent) {
                if (intent.action != ACTION_QUERY) return
                writeStatus(ctx.applicationContext, pollReady())
            }
        }
        ContextCompat.registerReceiver(
            context.applicationContext,
            receiver,
            IntentFilter(ACTION_QUERY),
            ContextCompat.RECEIVER_NOT_EXPORTED
        )
    }

    fun pollReady(): Int {
        if (!IrohNativeBridge.isNativeAvailable()) return RELAY_READY
        return IrohNativeBridge.pollRelayReady()
    }

    fun writeStatus(context: Context, status: Int) {
        context.openFileOutput(STATUS_FILE, Context.MODE_PRIVATE).use { out ->
            out.write("$status\n".toByteArray())
        }
    }
}
