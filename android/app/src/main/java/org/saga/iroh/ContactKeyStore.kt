package org.saga.iroh

import android.content.Context
import android.net.Uri
import org.saga.contacts.ContactKeyRepository

/**
 * Call-time key resolution bridge: Contacts MIME rows, not fixtures.
 */
object ContactKeyStore {
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

    fun peerIdFromKeyBytes(keyBytes: ByteArray): IrohNodeId? {
        val asText = keyBytes.toString(Charsets.UTF_8)
        if (IrohNodeId.parse(asText) != null) return IrohNodeId(asText)
        if (keyBytes.size == 32) {
            val hex = keyBytes.joinToString("") { "%02x".format(it) }
            return IrohNodeId(hex)
        }
        return null
    }
}
