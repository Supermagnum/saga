package org.saga.iroh

import android.content.Context
import android.net.Uri
import org.saga.contacts.ContactKeyRepository

/**
 * Call-time key resolution bridge: Contacts MIME rows, not fixtures.
 *
 * Contact DATA1 holds the ephemeral-session long-term fingerprint as 64
 * lowercase hex ASCII (Base64-wrapped in the MIME row). Iroh EndpointIds are
 * separate: dial peers use phone-digit labels (`15550100010`), not the token
 * fingerprint.
 */
object ContactKeyStore {
    /** 64 lowercase hex chars — ephemeral-session wire fingerprint format. */
    private val FINGERPRINT_HEX = Regex("^[0-9a-f]{64}$")

    fun hasResolvableKey(peerOrLookupId: String): Boolean {
        if (peerOrLookupId.endsWith("nokey", ignoreCase = true)) return false
        if (IrohNodeId.parse(peerOrLookupId) != null) return true
        return false
    }

    fun hasResolvableKey(context: Context, peerOrLookupId: String): Boolean {
        if (peerOrLookupId.endsWith("nokey", ignoreCase = true)) return false
        if (peerOrLookupId.startsWith("content://")) {
            return ContactKeyRepository.readPublicKeyBytes(context, Uri.parse(peerOrLookupId)) != null
        }
        val fromPhone = ContactKeyRepository.readPublicKeyBytesForPhone(context, peerOrLookupId)
        if (fromPhone != null) return true
        return IrohNodeId.parse(peerOrLookupId) != null
    }

    /**
     * Decode stored key material. Returns the 64-char fingerprint hex when valid.
     * Legacy phone-label rows are rejected (reseed via TestContactSeeder).
     */
    fun fingerprintHexFromKeyBytes(keyBytes: ByteArray): String? {
        val asText = keyBytes.toString(Charsets.UTF_8).trim().lowercase()
        if (FINGERPRINT_HEX.matches(asText)) return asText
        return null
    }

    /**
     * @deprecated Token fingerprint is not an Iroh peer id. Prefer phone-digit labels.
     * Returns a peer id only for legacy UTF-8 label rows still in Contacts.
     */
    fun peerIdFromKeyBytes(keyBytes: ByteArray): IrohNodeId? {
        if (fingerprintHexFromKeyBytes(keyBytes) != null) return null
        val asText = keyBytes.toString(Charsets.UTF_8)
        if (IrohNodeId.parse(asText) != null) return IrohNodeId(asText)
        return null
    }

    fun isFingerprintHex(raw: String): Boolean = FINGERPRINT_HEX.matches(raw.trim().lowercase())
}
