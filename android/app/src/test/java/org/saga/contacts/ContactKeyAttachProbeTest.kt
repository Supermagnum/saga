package org.saga.contacts

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ContactKeyAttachProbeTest {
    private val fpA =
        "80342e72c8388b728d66df256f8231258f250f760cd45613e34ab5d2ed9f4fa8"
    private val fpB =
        "ee52f46cbefba6e71b8e0a727a021f9cc99e85771114ff1c0b12fcc83418458b"

    @Test
    fun noExistingKeyIsReadyToSave() {
        val outcome = ContactKeyAttach.evaluateProbe(null, fpA, "bob")
        assertTrue(outcome is ContactKeyAttach.Outcome.Saved)
        assertEquals("bob", (outcome as ContactKeyAttach.Outcome.Saved).contactName)
    }

    @Test
    fun matchingKeyIsAlreadySaved() {
        val outcome = ContactKeyAttach.evaluateProbe(fpA, fpA, "bob")
        assertTrue(outcome is ContactKeyAttach.Outcome.AlreadySaved)
    }

    @Test
    fun differentKeyNeedsConfirm() {
        val outcome = ContactKeyAttach.evaluateProbe(fpA, fpB, "bob")
        assertTrue(outcome is ContactKeyAttach.Outcome.NeedsKeyChangeConfirm)
        val change = outcome as ContactKeyAttach.Outcome.NeedsKeyChangeConfirm
        assertEquals(fpA, change.existingFp)
        assertEquals(fpB, change.incomingFp)
        assertEquals("bob", change.contactName)
    }

    @Test
    fun badIncomingFingerprintErrors() {
        val outcome = ContactKeyAttach.evaluateProbe(null, "nope", "bob")
        assertTrue(outcome is ContactKeyAttach.Outcome.Error)
    }
}
