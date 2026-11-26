package org.saga.nfc

import android.content.Context
import android.content.SharedPreferences
import android.nfc.cardemulation.HostApduService
import android.os.Bundle
import android.util.Log

/**
 * HCE Type 4 tag serving the current add-contact NDEF message.
 * Enabled only while Share-via-NFC is open (component + prefs gate).
 */
class SagaNdefHceService : HostApduService() {
    companion object {
        private const val TAG = "[Saga NFC HCE]"
        private const val PREFS = "saga_nfc_hce"
        private const val KEY_LINK = "active_link"
        private const val KEY_ENABLED = "enabled"

        fun setActiveLink(context: Context, link: String?) {
            prefs(context).edit()
                .putString(KEY_LINK, link)
                .putBoolean(KEY_ENABLED, !link.isNullOrBlank())
                .apply()
        }

        fun clear(context: Context) {
            prefs(context).edit().clear().apply()
        }

        fun isArmed(context: Context): Boolean {
            return prefs(context).getBoolean(KEY_ENABLED, false) &&
                !prefs(context).getString(KEY_LINK, null).isNullOrBlank()
        }

        private fun prefs(context: Context): SharedPreferences {
            return context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        }
    }

    private var handler: Type4NdefApduHandler? = null

    override fun processCommandApdu(commandApdu: ByteArray?, extras: Bundle?): ByteArray {
        if (commandApdu == null) return Type4NdefApduHandler.hex("6985")
        if (!isArmed(this)) {
            Log.i(TAG, "HCE not armed — rejecting APDU")
            return Type4NdefApduHandler.hex("6985")
        }
        val h = handler ?: run {
            val link = prefs(this).getString(KEY_LINK, null)
            if (link.isNullOrBlank()) return Type4NdefApduHandler.hex("6985")
            Type4NdefApduHandler(NdefUriMessage.build(link)).also { handler = it }
        }
        return h.process(commandApdu)
    }

    override fun onDeactivated(reason: Int) {
        handler = null
        Log.i(TAG, "HCE deactivated reason=[$reason]")
    }
}
