package org.saga.iroh

import android.content.Context

object DevIdentityStore {
    private const val PREFS = "saga_dev_identity"
    private const val KEY_LABEL = "peer_label"

    fun getLabel(context: Context): String? {
        return context.applicationContext
            .getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .getString(KEY_LABEL, null)
            ?.takeIf { it.isNotBlank() }
    }
}
