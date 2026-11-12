package org.saga.call

import android.telecom.Call
import android.telecom.TelecomManager
import org.saga.iroh.ContactKeyStore
import org.saga.iroh.SagaCallUri
import org.saga.iroh.SagaDialer
import org.saga.telecom.SagaPhoneAccountRegistrar
import java.util.concurrent.ConcurrentHashMap

data class ActiveCallContext(
    val telecomCallId: String,
    val displayName: String,
    val origin: CallOrigin,
    val snapshot: CallSecuritySnapshot
)

object SagaCallRegistry {
    private val calls = ConcurrentHashMap<String, ActiveCallContext>()

    fun put(context: ActiveCallContext) {
        calls[context.telecomCallId] = context
    }

    fun updateSnapshot(telecomCallId: String, snapshot: CallSecuritySnapshot) {
        calls[telecomCallId]?.let { existing ->
            calls[telecomCallId] = existing.copy(snapshot = snapshot)
        }
    }

    fun get(callId: String): ActiveCallContext? = calls[callId]

    fun remove(callId: String) {
        calls.remove(callId)
    }

    fun originForTelecomCall(call: Call): CallOrigin {
        val handle = call.details.handle
        if (handle?.scheme == SagaCallUri.SCHEME) return CallOrigin.IROH
        if (call.details.accountHandle?.id == SagaPhoneAccountRegistrar.ACCOUNT_ID) {
            return CallOrigin.IROH
        }
        return CallOrigin.CELLULAR
    }

    fun peerIdFromCall(call: Call): String {
        val intentExtras = call.details.intentExtras
        val outgoing = intentExtras?.getBundle(TelecomManager.EXTRA_OUTGOING_CALL_EXTRAS)
        outgoing?.getString(SagaDialer.EXTRA_LOOKUP_KEY)?.takeIf { it.isNotBlank() }?.let { return it }
        outgoing?.getString(SagaDialer.EXTRA_PEER_ID)?.takeIf { it.isNotBlank() }?.let { return it }
        val incoming = intentExtras?.getBundle(TelecomManager.EXTRA_INCOMING_CALL_EXTRAS)
        incoming?.getString(SagaDialer.EXTRA_LOOKUP_KEY)?.takeIf { it.isNotBlank() }?.let { return it }
        incoming?.getString(SagaDialer.EXTRA_PEER_ID)?.takeIf { it.isNotBlank() }?.let { return it }
        intentExtras?.getString(SagaDialer.EXTRA_LOOKUP_KEY)?.takeIf { it.isNotBlank() }?.let { return it }
        intentExtras?.getString(SagaDialer.EXTRA_PEER_ID)?.takeIf { it.isNotBlank() }?.let { return it }
        val handle = call.details.handle ?: return "Unknown"
        if (handle.scheme == SagaCallUri.SCHEME) {
            return handle.schemeSpecificPart ?: handle.toString()
        }
        // Non-routable Iroh placeholder (tel:, or legacy tel:iroh) — never treat as peer id.
        val ssp = handle.schemeSpecificPart.orEmpty()
        if (ssp == SagaCallUri.IROH_TELECOM_SSP ||
            ssp.equals(SagaCallUri.LEGACY_UNSAFE_IROH_SSP, ignoreCase = true)
        ) {
            return "Unknown"
        }
        if (call.details.accountHandle?.id == SagaPhoneAccountRegistrar.ACCOUNT_ID) {
            return handle.schemeSpecificPart ?: handle.toString()
        }
        return handle.schemeSpecificPart ?: handle.toString()
    }

    fun displayNameFromCall(call: Call): String {
        val intentExtras = call.details.intentExtras
        val outgoing = intentExtras?.getBundle(TelecomManager.EXTRA_OUTGOING_CALL_EXTRAS)
        outgoing?.getString(SagaDialer.EXTRA_CONTACT_NAME)?.takeIf { it.isNotBlank() }?.let { return it }
        val incoming = intentExtras?.getBundle(TelecomManager.EXTRA_INCOMING_CALL_EXTRAS)
        incoming?.getString(SagaDialer.EXTRA_CONTACT_NAME)?.takeIf { it.isNotBlank() }?.let { return it }
        intentExtras?.getString(SagaDialer.EXTRA_CONTACT_NAME)?.takeIf { it.isNotBlank() }?.let { return it }
        return peerIdFromCall(call)
    }

    fun telecomCallId(call: Call): String {
        if (originForTelecomCall(call) == CallOrigin.IROH) {
            val peer = peerIdFromCall(call)
            return normalizeIrohTelecomCallId(peer)
        }
        return call.details.handle?.toString() ?: peerIdFromCall(call)
    }

    /** Align with ConnectionService/IrohDialManager lookup keys (+E.164 for phone-digit peer labels). */
    private fun normalizeIrohTelecomCallId(peer: String): String {
        if (peer.startsWith("+")) return peer
        if (peer.all { it.isDigit() } && peer.length >= 10) return "+$peer"
        return peer
    }

    fun initialSnapshot(
        origin: CallOrigin,
        peerId: String = "",
        context: android.content.Context? = null
    ): CallSecuritySnapshot {
        if (origin == CallOrigin.CELLULAR) {
            return CallSecurityStateResolver.resolve(CallOrigin.CELLULAR)
        }
        // Prefer Contacts-aware lookup; the no-Context overload only accepts IrohNodeId shape
        // and would treat phone lookup keys as KEY_NOT_FOUND (premature call_unsecure).
        val hasKey = if (context != null) {
            ContactKeyStore.hasResolvableKey(context, peerId)
        } else {
            ContactKeyStore.hasResolvableKey(peerId)
        }
        return if (hasKey) {
            CallSecurityStateResolver.resolve(
                origin = CallOrigin.IROH,
                handshakeSecuring = true,
                contactKeyResolvable = true
            )
        } else {
            CallSecurityStateResolver.resolve(
                origin = CallOrigin.IROH,
                contactKeyResolvable = false
            )
        }
    }
}
