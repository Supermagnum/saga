package org.saga.iroh

import android.content.Context
import android.net.Uri
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.ContactsContract
import android.telecom.PhoneAccount
import android.telecom.TelecomManager
import android.util.Log
import org.saga.telecom.SagaPhoneAccountRegistrar

/**
 * Rust inbound accept -> TelecomManager.addNewIncomingCall -> SagaConnectionService.
 */
object IrohIncomingCallBridge {
    private const val TAG = "[Saga Incoming Call Bridge]"

    @Volatile
    private var appContext: Context? = null
    private val mainHandler = Handler(Looper.getMainLooper())

    fun init(context: Context) {
        appContext = context.applicationContext
    }

    fun onIncomingFromNative(sessionId: String, lookupKey: String, remoteEndpointId: String) {
        val ctx = appContext
        if (ctx == null) {
            Log.e(TAG, "CHECKPOINT FAIL appContext null — cannot addNewIncomingCall")
            return
        }
        mainHandler.post {
            deliverIncomingCall(ctx, sessionId, lookupKey, remoteEndpointId)
        }
    }

    private fun deliverIncomingCall(
        context: Context,
        sessionId: String,
        lookupKey: String,
        remoteEndpointId: String
    ) {
        val telecom = context.getSystemService(TelecomManager::class.java)
        if (telecom == null) {
            Log.e(TAG, "CHECKPOINT FAIL TelecomManager unavailable")
            return
        }

        if (!SagaPhoneAccountRegistrar.ensureEnabled(context)) {
            Log.e(TAG, "CHECKPOINT FAIL Saga PhoneAccount not enabled")
            return
        }

        val handle = SagaPhoneAccountRegistrar.phoneAccountHandle(context)
        val registered = telecom.getPhoneAccount(handle)
        if (registered == null) {
            Log.e(TAG, "CHECKPOINT FAIL PhoneAccount not registered handle=[$handle]")
            SagaPhoneAccountRegistrar.register(context)
            return
        }
        Log.i(
            TAG,
            "CHECKPOINT PhoneAccount verified enabled=[${registered.isEnabled}] handle=[$handle]"
        )

        val peerId = IrohNodeId.parse(lookupKey) ?: IrohNodeId(lookupKey)
        val displayName = resolveDisplayName(context, lookupKey)
        val telAddress = resolveTelAddress(context, lookupKey) ?: SagaDialer.IROH_PLACEHOLDER_TEL
        val address = Uri.fromParts(PhoneAccount.SCHEME_TEL, telAddress, null)

        val callExtras = Bundle().apply {
            putString(SagaDialer.EXTRA_PEER_ID, peerId.raw)
            putString(SagaDialer.EXTRA_SESSION_ID, sessionId)
            putString(SagaDialer.EXTRA_LOOKUP_KEY, lookupKey)
            putString(SagaDialer.EXTRA_REMOTE_ENDPOINT_ID, remoteEndpointId)
            putString(SagaDialer.EXTRA_CONTACT_NAME, displayName)
        }

        val extras = Bundle().apply {
            putParcelable(TelecomManager.EXTRA_INCOMING_CALL_ADDRESS, address)
            putParcelable(TelecomManager.EXTRA_PHONE_ACCOUNT_HANDLE, handle)
            putBundle(TelecomManager.EXTRA_INCOMING_CALL_EXTRAS, callExtras)
        }

        Log.i(
            TAG,
            "CHECKPOINT addNewIncomingCall session=[$sessionId] lookup=[$lookupKey] display=[$displayName]"
        )
        try {
            telecom.addNewIncomingCall(handle, extras)
            Log.i(TAG, "CHECKPOINT addNewIncomingCall returned (no throw)")
        } catch (e: SecurityException) {
            Log.e(TAG, "CHECKPOINT FAIL addNewIncomingCall SecurityException: ${e.message}")
        } catch (e: Exception) {
            Log.e(TAG, "CHECKPOINT FAIL addNewIncomingCall: ${e.message}")
        }
    }

    private fun resolveDisplayName(context: Context, lookupKey: String): String {
        val cursor = context.contentResolver.query(
            ContactsContract.CommonDataKinds.Phone.CONTENT_URI,
            arrayOf(
                ContactsContract.CommonDataKinds.Phone.DISPLAY_NAME,
                ContactsContract.CommonDataKinds.Phone.NUMBER
            ),
            null,
            null,
            null
        ) ?: return lookupKey
        cursor.use {
            while (it.moveToNext()) {
                val number = it.getString(1)?.filter { ch -> ch.isDigit() }.orEmpty()
                val keyDigits = lookupKey.filter { ch -> ch.isDigit() }
                if (number.endsWith(keyDigits) || number == keyDigits) {
                    return it.getString(0) ?: lookupKey
                }
            }
        }
        return lookupKey
    }

    private fun resolveTelAddress(context: Context, lookupKey: String): String? {
        val cursor = context.contentResolver.query(
            ContactsContract.CommonDataKinds.Phone.CONTENT_URI,
            arrayOf(ContactsContract.CommonDataKinds.Phone.NUMBER),
            null,
            null,
            null
        ) ?: return null
        cursor.use {
            while (it.moveToNext()) {
                val number = it.getString(0) ?: continue
                val digits = number.filter { ch -> ch.isDigit() }
                val keyDigits = lookupKey.filter { ch -> ch.isDigit() }
                if (digits.endsWith(keyDigits) || digits == keyDigits) {
                    return number
                }
            }
        }
        return null
    }
}
