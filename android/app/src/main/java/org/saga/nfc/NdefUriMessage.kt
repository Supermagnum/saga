package org.saga.nfc

/**
 * Minimal NDEF URI message build/parse for saga://add-contact links.
 * URI identifier code 0x00 = no prefix (full URI in payload).
 */
object NdefUriMessage {
    private const val TNF_WELL_KNOWN: Byte = 0x01
    private val TYPE_URI = byteArrayOf('U'.code.toByte())

    fun build(uri: String): ByteArray {
        val uriBytes = uri.toByteArray(Charsets.UTF_8)
        require(uriBytes.size <= 255) { "URI too long for short NDEF record" }
        val payload = ByteArray(1 + uriBytes.size)
        payload[0] = 0x00 // no prefix
        System.arraycopy(uriBytes, 0, payload, 1, uriBytes.size)
        val record = ArrayList<Byte>()
        // MB=1 ME=1 SR=1 TNF=well-known
        record.add((0xD1).toByte())
        record.add(TYPE_URI.size.toByte())
        record.add(payload.size.toByte())
        record.addAll(TYPE_URI.toList())
        record.addAll(payload.toList())
        return record.toByteArray()
    }

    fun parseUri(ndefMessage: ByteArray): String? {
        if (ndefMessage.size < 3) return null
        var i = 0
        val header = ndefMessage[i++].toInt() and 0xFF
        val tnf = header and 0x07
        val sr = (header and 0x10) != 0
        if (tnf != TNF_WELL_KNOWN.toInt()) return null
        if (i >= ndefMessage.size) return null
        val typeLen = ndefMessage[i++].toInt() and 0xFF
        val payloadLen = if (sr) {
            if (i >= ndefMessage.size) return null
            ndefMessage[i++].toInt() and 0xFF
        } else {
            if (i + 3 >= ndefMessage.size) return null
            val b0 = ndefMessage[i++].toInt() and 0xFF
            val b1 = ndefMessage[i++].toInt() and 0xFF
            val b2 = ndefMessage[i++].toInt() and 0xFF
            val b3 = ndefMessage[i++].toInt() and 0xFF
            (b0 shl 24) or (b1 shl 16) or (b2 shl 8) or b3
        }
        if (i + typeLen + payloadLen > ndefMessage.size) return null
        val type = ndefMessage.copyOfRange(i, i + typeLen)
        i += typeLen
        if (!type.contentEquals(TYPE_URI)) return null
        val payload = ndefMessage.copyOfRange(i, i + payloadLen)
        if (payload.isEmpty()) return null
        // identifier code ignored for 0x00; for others we only accept 0x00
        if (payload[0] != 0.toByte()) return null
        return payload.copyOfRange(1, payload.size).toString(Charsets.UTF_8)
    }
}
