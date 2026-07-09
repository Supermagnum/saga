package org.saga.handshake

import android.content.Context

object SagaTestFlags {
    private const val PREFS = "saga_test"
    private const val KEY_FORCE_FAIL = "force_fail"

    fun isForceHandshakeFail(context: Context): Boolean {
        return context.applicationContext
            .getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .getBoolean(KEY_FORCE_FAIL, false)
    }
}
