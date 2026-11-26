package org.saga.contacts

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class SagaAddContactLinkTest {
    private val fp =
        "80342e72c8388b728d66df256f8231258f250f760cd45613e34ab5d2ed9f4fa8"

    @Test
    fun buildAndParseRoundTrip() {
        val payload = SagaAddContactLink.Payload(
            fingerprintHex = fp.uppercase(),
            endpointId = "15550100010",
            suggestedName = "Bob Peer"
        )
        val link = SagaAddContactLink.build(payload)
        assertTrue(link.startsWith("saga://add-contact?"))
        val parsed = SagaAddContactLink.parse(link).getOrThrow()
        assertEquals(fp, parsed.fingerprintHex)
        assertEquals("15550100010", parsed.endpointId)
        assertEquals("Bob Peer", parsed.suggestedName)
    }

    @Test
    fun parseMinimalFingerprintOnly() {
        val link = "saga://add-contact?v=1&fp=$fp"
        val parsed = SagaAddContactLink.parse(link).getOrThrow()
        assertEquals(fp, parsed.fingerprintHex)
        assertEquals(null, parsed.endpointId)
        assertEquals(null, parsed.suggestedName)
    }

    @Test
    fun rejectBadFingerprint() {
        val link = "saga://add-contact?v=1&fp=nothex"
        assertTrue(SagaAddContactLink.parse(link).isFailure)
    }

    @Test
    fun rejectUnknownVersion() {
        val link = "saga://add-contact?v=99&fp=$fp"
        assertTrue(SagaAddContactLink.parse(link).isFailure)
    }

    @Test
    fun rejectExtraParam() {
        val link = "saga://add-contact?v=1&fp=$fp&evil=1"
        assertTrue(SagaAddContactLink.parse(link).isFailure)
    }

    @Test
    fun rejectWrongHost() {
        val link = "saga://call?v=1&fp=$fp"
        assertTrue(SagaAddContactLink.parse(link).isFailure)
    }

    @Test
    fun rejectDuplicateParam() {
        val link = "saga://add-contact?v=1&fp=$fp&fp=$fp"
        assertTrue(SagaAddContactLink.parse(link).isFailure)
    }

    @Test
    fun buildRejectsInvalidFingerprint() {
        try {
            SagaAddContactLink.build(SagaAddContactLink.Payload("abc"))
            fail("expected IllegalArgumentException")
        } catch (_: IllegalArgumentException) {
            // expected
        }
    }

    @Test
    fun plausibleEndpointAcceptsHexAndDigits() {
        assertTrue(SagaAddContactLink.isPlausibleEndpointId(fp))
        assertTrue(SagaAddContactLink.isPlausibleEndpointId("15550100010"))
    }

    @Test
    fun buildAndParseWithEndpointHex() {
        val ep = "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899"
        val link = SagaAddContactLink.build(
            SagaAddContactLink.Payload(fingerprintHex = fp, endpointId = ep)
        )
        val parsed = SagaAddContactLink.parse(link).getOrThrow()
        assertEquals(fp, parsed.fingerprintHex)
        assertEquals(ep, parsed.endpointId)
    }

    @Test
    fun rejectBadEndpoint() {
        val link = "saga://add-contact?v=1&fp=$fp&ep=!!!"
        assertTrue(SagaAddContactLink.parse(link).isFailure)
    }
}
