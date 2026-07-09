package org.saga.iroh

import android.content.Context
import android.net.Uri
import android.os.Bundle
import android.telecom.TelecomManager
import android.util.Log
import org.saga.dial.DialTarget
import org.saga.telecom.SagaPhoneAccountRegistrar

object SagaDialer {
    private const val TAG = "[Saga Dialer]"
    /** Non-routable placeholder; real peer is in [EXTRA_PEER_ID] for Telecom. */
    const val IROH_PLACEHOLDER_TEL = "+15550100999"
    const val EXTRA_PEER_ID = "org.saga.EXTRA_PEER_ID"

    fun parseIrohUri(uri: Uri?): IrohNodeId? = SagaCallUri.parsePeer(uri)

    fun peerIdFromConnectionRequest(uri: Uri?, extras: Bundle?): IrohNodeId? {
        val outgoing = extras?.getBundle(TelecomManager.EXTRA_OUTGOING_CALL_EXTRAS)
        outgoing?.getString(EXTRA_PEER_ID)?.let { raw ->
            IrohNodeId.parse(raw)?.let { return it }
        }
        extras?.getString(EXTRA_PEER_ID)?.let { raw ->
            IrohNodeId.parse(raw)?.let { return it }
        }
        return parseIrohUri(uri)
    }

    fun placeCall(context: Context, target: DialTarget) {
        when (target) {
            is DialTarget.Cellular -> placeCellularCall(context, target.number)
            is DialTarget.Iroh -> placeIrohCall(
                context,
                target.peerId,
                target.lookupKey,
                target.contactName ?: target.lookupKey
            )
        }
    }

    fun placeIrohCall(
        context: Context,
        peerId: IrohNodeId,
        lookupKey: String = peerId.raw,
        contactName: String = lookupKey
    ) {
        PendingDialLookup.put(peerId.raw, lookupKey, contactName)
        Log.i(TAG, "Placing Iroh call to contact [$contactName] via Saga PhoneAccount")
        placeIrohViaTelecom(context, peerId)
    }

    fun placeCellularCall(context: Context, number: String) {
        val uri = Uri.fromParts("tel", number, null)
        Log.i(TAG, "Placing cellular call to [$number]")
        context.getSystemService(TelecomManager::class.java).placeCall(uri, Bundle())
    }

    private fun placeIrohViaTelecom(context: Context, peerId: IrohNodeId) {
        val telecom = context.getSystemService(TelecomManager::class.java)
        val uri = Uri.fromParts("tel", IROH_PLACEHOLDER_TEL, null)
        val peerExtras = Bundle().apply {
            putString(EXTRA_PEER_ID, peerId.raw)
        }
        val extras = Bundle().apply {
            putParcelable(
                TelecomManager.EXTRA_PHONE_ACCOUNT_HANDLE,
                SagaPhoneAccountRegistrar.phoneAccountHandle(context)
            )
            putBundle(TelecomManager.EXTRA_OUTGOING_CALL_EXTRAS, peerExtras)
        }
        telecom.placeCall(uri, extras)
    }
}
