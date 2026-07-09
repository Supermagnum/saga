package org.saga.iroh

import android.content.Context

object IrohRelayStore {
    private const val PREFS = "saga_iroh_relay"
    private const val KEY_URL = "relay_url"

    fun getUrl(context: Context): String? {
        return context.applicationContext
            .getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .getString(KEY_URL, null)
            ?.trim()
            ?.takeIf { it.isNotEmpty() }
    }
}
