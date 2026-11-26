package org.saga.nfc

import android.nfc.Tag
import android.nfc.tech.IsoDep
import android.util.Log

/** Read an NDEF URI from a Type 4 tag / HCE peer over ISO-DEP. */
object NfcType4Reader {
    private const val TAG = "[Saga NFC Reader]"

    fun readUri(tag: Tag): String? {
        val iso = IsoDep.get(tag) ?: return null
        return try {
            iso.connect()
            iso.timeout = 5000
            val selectAid = apdu(0x00, 0xA4, 0x04, 0x00, Type4NdefApduHandler.NDEF_AID)
            if (!ok(transceive(iso, selectAid))) return null
            val selectCc = apdu(0x00, 0xA4, 0x00, 0x0C, Type4NdefApduHandler.hex("E103"))
            if (!ok(transceive(iso, selectCc))) return null
            val cc = stripSw(transceive(iso, apdu(0x00, 0xB0, 0x00, 0x00, le = 15))) ?: return null
            if (cc.size < 15) return null
            val ndefFileId = cc.copyOfRange(9, 11)
            val selectNdef = apdu(0x00, 0xA4, 0x00, 0x0C, ndefFileId)
            if (!ok(transceive(iso, selectNdef))) return null
            val lenBytes = stripSw(transceive(iso, apdu(0x00, 0xB0, 0x00, 0x00, le = 2))) ?: return null
            if (lenBytes.size < 2) return null
            val ndefLen = ((lenBytes[0].toInt() and 0xFF) shl 8) or (lenBytes[1].toInt() and 0xFF)
            if (ndefLen <= 0 || ndefLen > 4096) return null
            val body = stripSw(
                transceive(iso, apdu(0x00, 0xB0, 0x00, 0x02, le = ndefLen.coerceAtMost(256)))
            ) ?: return null
            NdefUriMessage.parseUri(body)
        } catch (e: Exception) {
            Log.w(TAG, "ISO-DEP read failed: ${e.message}")
            null
        } finally {
            try {
                iso.close()
            } catch (_: Exception) {
            }
        }
    }

    private fun transceive(iso: IsoDep, cmd: ByteArray): ByteArray {
        return iso.transceive(cmd)
    }

    private fun ok(resp: ByteArray): Boolean {
        if (resp.size < 2) return false
        return resp[resp.size - 2] == 0x90.toByte() && resp[resp.size - 1] == 0x00.toByte()
    }

    private fun stripSw(resp: ByteArray?): ByteArray? {
        if (resp == null || resp.size < 2) return null
        if (!ok(resp)) return null
        return resp.copyOfRange(0, resp.size - 2)
    }

    private fun apdu(cla: Int, ins: Int, p1: Int, p2: Int, data: ByteArray? = null, le: Int? = null): ByteArray {
        val out = ArrayList<Byte>()
        out.add(cla.toByte())
        out.add(ins.toByte())
        out.add(p1.toByte())
        out.add(p2.toByte())
        if (data != null) {
            out.add(data.size.toByte())
            out.addAll(data.toList())
        }
        if (le != null) {
            out.add(if (le >= 256) 0x00 else le.toByte())
        }
        return out.toByteArray()
    }
}
