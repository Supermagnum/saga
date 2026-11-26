package org.saga.nfc

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class Type4NdefApduHandlerTest {
    private val link = "saga://add-contact?v=1&fp=" + "cd".repeat(32)
    private val ndef = NdefUriMessage.build(link)
    private val handler = Type4NdefApduHandler(ndef)

    @Test
    fun selectAidCcNdefAndRead() {
        val selectAid = cmd(0x00, 0xA4, 0x04, 0x00, Type4NdefApduHandler.NDEF_AID)
        assertSwOk(handler.process(selectAid))

        val selectCc = cmd(0x00, 0xA4, 0x00, 0x0C, Type4NdefApduHandler.hex("E103"))
        assertSwOk(handler.process(selectCc))

        val cc = handler.process(cmd(0x00, 0xB0, 0x00, 0x00, le = 15))
        assertTrue(cc.size >= 17)
        assertSwOk(cc)

        val selectNdef = cmd(0x00, 0xA4, 0x00, 0x0C, Type4NdefApduHandler.hex("E104"))
        assertSwOk(handler.process(selectNdef))

        val lenResp = handler.process(cmd(0x00, 0xB0, 0x00, 0x00, le = 2))
        assertSwOk(lenResp)
        val len = ((lenResp[0].toInt() and 0xFF) shl 8) or (lenResp[1].toInt() and 0xFF)
        assertEquals(ndef.size, len)

        val body = handler.process(cmd(0x00, 0xB0, 0x00, 0x02, le = len))
        assertSwOk(body)
        val payload = body.copyOfRange(0, body.size - 2)
        assertEquals(link, NdefUriMessage.parseUri(payload))
    }

    @Test
    fun unknownInsRejected() {
        val r = handler.process(byteArrayOf(0x00, 0xCA.toByte(), 0x00, 0x00, 0x00))
        assertArrayEquals(Type4NdefApduHandler.hex("6D00"), r)
    }

    @Test
    fun wrongAidRejected() {
        val r = handler.process(cmd(0x00, 0xA4, 0x04, 0x00, Type4NdefApduHandler.hex("A000000003")))
        assertArrayEquals(Type4NdefApduHandler.hex("6A82"), r)
    }

    private fun assertSwOk(resp: ByteArray) {
        assertTrue(resp.size >= 2)
        assertEquals(0x90.toByte(), resp[resp.size - 2])
        assertEquals(0x00.toByte(), resp[resp.size - 1])
    }

    private fun cmd(
        cla: Int,
        ins: Int,
        p1: Int,
        p2: Int,
        data: ByteArray? = null,
        le: Int? = null
    ): ByteArray {
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
            out.add(if (le >= 256) 0 else le.toByte())
        }
        return out.toByteArray()
    }
}
