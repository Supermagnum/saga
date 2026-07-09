package org.saga.telecom

import android.content.ComponentName
import android.content.Context
import android.net.Uri
import android.telecom.PhoneAccount
import android.telecom.PhoneAccountHandle
import android.telecom.TelecomManager
import android.util.Log
import org.saga.iroh.SagaCallUri

object SagaPhoneAccountRegistrar {
    private const val TAG = "[Saga Phone Account]"
    const val ACCOUNT_ID = "saga_iroh"

    fun register(context: Context) {
        val telecomManager = context.getSystemService(TelecomManager::class.java)
        val handle = phoneAccountHandle(context)
        val account = PhoneAccount.builder(handle, "Saga")
            .setCapabilities(PhoneAccount.CAPABILITY_CALL_PROVIDER)
            .setShortDescription("Saga encrypted calls")
            .setAddress(Uri.fromParts(SagaCallUri.SCHEME, "saga", null))
            .addSupportedUriScheme(PhoneAccount.SCHEME_TEL)
            .addSupportedUriScheme(SagaCallUri.SCHEME)
            .build()
        telecomManager.registerPhoneAccount(account)
        Log.i(TAG, "Registered Saga phone account (tel + saga schemes)")
    }

    fun phoneAccountHandle(context: Context): PhoneAccountHandle {
        return PhoneAccountHandle(
            ComponentName(context, SagaConnectionService::class.java),
            ACCOUNT_ID
        )
    }

    fun handle(context: Context): PhoneAccountHandle = phoneAccountHandle(context)
}
