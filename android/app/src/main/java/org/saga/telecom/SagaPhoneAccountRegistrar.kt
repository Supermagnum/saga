package org.saga.telecom

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.telecom.PhoneAccount
import android.telecom.PhoneAccountHandle
import android.telecom.TelecomManager
import android.util.Log
import org.saga.iroh.SagaCallUri

object SagaPhoneAccountRegistrar {
    private const val TAG = "[Saga Phone Account]"
    const val ACCOUNT_ID = "saga_iroh"
    const val ACTION_SHOW_ONBOARDING = "org.saga.SHOW_PHONE_ACCOUNT_ONBOARDING"

    fun register(context: Context) {
        val telecomManager = context.getSystemService(TelecomManager::class.java) ?: return
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

    /**
     * Ensures the account is registered and returns whether it is enabled.
     * Returns true only when present AND enabled — disabled accounts must not place Iroh calls.
     */
    fun ensureEnabled(context: Context): Boolean {
        register(context)
        return isAccountEnabled(context)
    }

    fun isAccountEnabled(context: Context): Boolean {
        val telecomManager = context.getSystemService(TelecomManager::class.java) ?: return false
        val handle = phoneAccountHandle(context)
        var account = telecomManager.getPhoneAccount(handle)
        if (account == null) {
            Log.w(TAG, "PhoneAccount missing — re-registering")
            register(context)
            account = telecomManager.getPhoneAccount(handle)
        }
        if (account == null) {
            Log.e(TAG, "PhoneAccount still missing after register handle=[$handle]")
            return false
        }
        val enabled = account.isEnabled
        if (!enabled) {
            Log.w(
                TAG,
                "PhoneAccount registered but disabled — enable via Calling accounts settings"
            )
        }
        Log.i(TAG, "PhoneAccount present=[true] enabled=[$enabled] handle=[$handle]")
        return enabled
    }

    fun changePhoneAccountsIntent(): Intent {
        return Intent(TelecomManager.ACTION_CHANGE_PHONE_ACCOUNTS)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    }

    fun showOnboardingIntent(context: Context): Intent {
        return Intent(ACTION_SHOW_ONBOARDING).setPackage(context.packageName)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    }

    fun phoneAccountHandle(context: Context): PhoneAccountHandle {
        return PhoneAccountHandle(
            ComponentName(context, SagaConnectionService::class.java),
            ACCOUNT_ID
        )
    }

    fun handle(context: Context): PhoneAccountHandle = phoneAccountHandle(context)
}
