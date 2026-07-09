package org.saga.security

import android.content.Context
import android.util.Log

object DowngradeEventLog {
    private const val PREFS = "saga_downgrade_events"
    private const val KEY_LOG = "events"

    fun record(context: Context, contactKey: String) {
        val prefs = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val entry = "${System.currentTimeMillis()}|$contactKey"
        val existing = prefs.getString(KEY_LOG, "").orEmpty()
        val updated = if (existing.isEmpty()) entry else "$existing\n$entry"
        prefs.edit().putString(KEY_LOG, updated).apply()
        Log.i("[Saga Downgrade Event]", "recorded downgrade contact=[$contactKey] at [$entry]")
    }

    fun hasEventFor(context: Context, contactKey: String): Boolean {
        val prefs = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        return prefs.getString(KEY_LOG, "").orEmpty().contains(contactKey)
    }
}
