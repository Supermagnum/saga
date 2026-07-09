package org.saga.handshake

import android.content.Context

class EncryptionEstablishedStore(context: Context) {
    private val prefs = context.applicationContext.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

    fun hasEncryptionHistory(contactKey: String): Boolean = prefs.getBoolean(contactKey, false)

    fun markEncryptionEstablished(contactKey: String) {
        prefs.edit().putBoolean(contactKey, true).apply()
    }

    companion object {
        private const val PREFS_NAME = "saga_encryption_established"
    }
}
