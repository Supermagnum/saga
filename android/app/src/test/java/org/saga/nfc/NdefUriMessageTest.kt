package org.saga.nfc

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class NdefUriMessageTest {
    @Test
    fun roundTrip() {
        val uri = "saga://add-contact?v=1&fp=" + "ab".repeat(32)
        val msg = NdefUriMessage.build(uri)
        assertEquals(uri, NdefUriMessage.parseUri(msg))
    }

    @Test
    fun rejectGarbage() {
        assertNull(NdefUriMessage.parseUri(byteArrayOf(0x01, 0x02)))
    }
}
