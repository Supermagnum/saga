package org.saga.security

import android.content.Context
import android.util.Log

/**
 * Records explicit user confirmations when a contact's Saga fingerprint changes.
 * Same prefs style as [DowngradeEventLog].
 */
object KeyChangeEventLog {
    private const val PREFS = "saga_key_change_events"
    private const val KEY_LOG = "events"

    fun record(context: Context, contactKey: String, oldFp: String, newFp: String) {
        val prefs = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val entry = "${System.currentTimeMillis()}|$contactKey|${oldFp.take(16)}|${newFp.take(16)}"
        val existing = prefs.getString(KEY_LOG, "").orEmpty()
        val updated = if (existing.isEmpty()) entry else "$existing\n$entry"
        prefs.edit().putString(KEY_LOG, updated).apply()
        Log.i("[Saga Key Change Event]", "recorded key-change contact=[$contactKey]")
    }
}
