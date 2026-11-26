package org.saga.nfc

/**
 * NFC Forum Type 4 Tag APDU handling for a single NDEF URI message.
 * Pure logic for unit tests; used by [SagaNdefHceService].
 */
class Type4NdefApduHandler(
    private val ndefMessage: ByteArray
) {
    companion object {
        val NDEF_AID = hex("D2760000850101")
        private val CC_FILE_ID = hex("E103")
        private val NDEF_FILE_ID = hex("E104")
        private val SW_OK = hex("9000")
        private val SW_FILE_NOT_FOUND = hex("6A82")
        private val SW_INS_NOT_SUPPORTED = hex("6D00")
        private val SW_WRONG_P1P2 = hex("6A86")
        private val SW_WRONG_LENGTH = hex("6700")
        private val SW_CONDITIONS = hex("6985")

        fun hex(s: String): ByteArray {
            val clean = s.replace(" ", "")
            require(clean.length % 2 == 0)
            return ByteArray(clean.length / 2) { i ->
                clean.substring(i * 2, i * 2 + 2).toInt(16).toByte()
            }
        }
    }

    private enum class Selected { None, App, Cc, Ndef }

    private var selected = Selected.None
    private val ndefFile: ByteArray = run {
        val len = ndefMessage.size
        require(len <= 0xFFFF)
        byteArrayOf(((len shr 8) and 0xFF).toByte(), (len and 0xFF).toByte()) + ndefMessage
    }
    private val ccFile: ByteArray = run {
        val maxNdef = (ndefFile.size).coerceAtLeast(0x00FF)
        byteArrayOf(
            0x00, 0x0F, // CCLEN
            0x20, // mapping v2.0
            0x00, 0x3B.toByte(), // MLe
            0x00, 0x34, // MLc
            0x04, 0x06, // NDEF File Control TLV
            0xE1.toByte(), 0x04, // file id
            ((maxNdef shr 8) and 0xFF).toByte(), (maxNdef and 0xFF).toByte(),
            0x00, // read access
            0xFF.toByte() // write not allowed
        )
    }

    fun process(commandApdu: ByteArray): ByteArray {
        if (commandApdu.size < 4) return SW_WRONG_LENGTH
        val cla = commandApdu[0].toInt() and 0xFF
        val ins = commandApdu[1].toInt() and 0xFF
        val p1 = commandApdu[2].toInt() and 0xFF
        val p2 = commandApdu[3].toInt() and 0xFF
        if (cla != 0x00 && cla != 0x10) return SW_CONDITIONS
        return when (ins) {
            0xA4 -> select(p1, p2, commandApdu)
            0xB0 -> readBinary(p1, p2, commandApdu)
            else -> SW_INS_NOT_SUPPORTED
        }
    }

    private fun select(p1: Int, p2: Int, apdu: ByteArray): ByteArray {
        val data = dataField(apdu) ?: return SW_WRONG_LENGTH
        return when {
            p1 == 0x04 && (p2 == 0x00 || p2 == 0x0C) -> {
                if (data.contentEquals(NDEF_AID)) {
                    selected = Selected.App
                    SW_OK
                } else {
                    selected = Selected.None
                    SW_FILE_NOT_FOUND
                }
            }
            p1 == 0x00 && (p2 == 0x0C || p2 == 0x00) -> {
                if (selected == Selected.None) return SW_CONDITIONS
                when {
                    data.contentEquals(CC_FILE_ID) -> {
                        selected = Selected.Cc
                        SW_OK
                    }
                    data.contentEquals(NDEF_FILE_ID) -> {
                        selected = Selected.Ndef
                        SW_OK
                    }
                    else -> SW_FILE_NOT_FOUND
                }
            }
            else -> SW_WRONG_P1P2
        }
    }

    private fun readBinary(p1: Int, p2: Int, apdu: ByteArray): ByteArray {
        val file = when (selected) {
            Selected.Cc -> ccFile
            Selected.Ndef -> ndefFile
            else -> return SW_CONDITIONS
        }
        val offset = ((p1 and 0xFF) shl 8) or (p2 and 0xFF)
        if (offset > file.size) return SW_WRONG_P1P2
        val le = leField(apdu) ?: return SW_WRONG_LENGTH
        val end = (offset + le).coerceAtMost(file.size)
        val slice = file.copyOfRange(offset, end)
        return slice + SW_OK
    }

    private fun dataField(apdu: ByteArray): ByteArray? {
        if (apdu.size < 5) return byteArrayOf()
        val lc = apdu[4].toInt() and 0xFF
        if (apdu.size < 5 + lc) return null
        return apdu.copyOfRange(5, 5 + lc)
    }

    private fun leField(apdu: ByteArray): Int? {
        // Case 2 short: CLA INS P1 P2 Le
        if (apdu.size == 5) {
            val le = apdu[4].toInt() and 0xFF
            return if (le == 0) 256 else le
        }
        // Case 4: CLA INS P1 P2 Lc Data Le
        if (apdu.size >= 6) {
            val lc = apdu[4].toInt() and 0xFF
            if (apdu.size == 5 + lc + 1) {
                val le = apdu[apdu.size - 1].toInt() and 0xFF
                return if (le == 0) 256 else le
            }
            if (apdu.size == 5 + lc) {
                // no Le — default max
                return 256
            }
        }
        return 256
    }
}
