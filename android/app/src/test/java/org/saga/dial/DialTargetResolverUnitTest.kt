package org.saga.dial

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Test
import org.saga.iroh.IrohNodeId

class DialTargetResolverUnitTest {
    private val fp =
        "80342e72c8388b728d66df256f8231258f250f760cd45613e34ab5d2ed9f4fa8"

    @Test
    fun parsesPhoneDigitEndpointLabel() {
        val peer = IrohNodeId.parse("15550100010")
        assertNotNull(peer)
        assertEquals("15550100010", peer!!.raw)
    }

    @Test
    fun prefersData2EndpointOverPhoneDigits() {
        val keyBytes = fp.toByteArray(Charsets.UTF_8)
        val peer = DialTargetResolver.irohPeerForContactKey(
            keyBytes,
            phone = "+15550100010",
            endpointId = "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899"
        )
        assertNotNull(peer)
        assertEquals(
            "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899",
            peer!!.raw
        )
    }

    @Test
    fun fallsBackToPhoneDigitsWhenData2Missing() {
        val keyBytes = fp.toByteArray(Charsets.UTF_8)
        val peer = DialTargetResolver.irohPeerForContactKey(
            keyBytes,
            phone = "+15550100010",
            endpointId = null
        )
        assertNotNull(peer)
        assertEquals("15550100010", peer!!.raw)
    }

    @Test
    fun ignoresUnparseableData2AndFallsBack() {
        val keyBytes = fp.toByteArray(Charsets.UTF_8)
        val peer = DialTargetResolver.irohPeerForContactKey(
            keyBytes,
            phone = "15550100011",
            endpointId = "!!!"
        )
        assertNotNull(peer)
        assertEquals("15550100011", peer!!.raw)
    }

    @Test
    fun fingerprintWithoutPhoneCannotDial() {
        val keyBytes = fp.toByteArray(Charsets.UTF_8)
        val peer = DialTargetResolver.irohPeerForContactKey(
            keyBytes,
            phone = null,
            endpointId = null
        )
        assertNull(peer)
    }
}
