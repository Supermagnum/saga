package org.saga.contacts

import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ContactKeyDirectoryTest {
    @Test
    fun evaluateDuplicate_nullWhenSameAsSourceOrTarget() {
        assertNull(ContactKeyDirectory.evaluateDuplicate(1L, 2L, 1L, "a"))
        assertNull(ContactKeyDirectory.evaluateDuplicate(1L, 2L, 2L, "b"))
        assertNull(ContactKeyDirectory.evaluateDuplicate(1L, 2L, null, null))
    }

    @Test
    fun evaluateDuplicate_reportsOtherOwner() {
        val outcome = ContactKeyDirectory.evaluateDuplicate(1L, 2L, 9L, "alice")
        assertTrue(outcome is ContactKeyDirectory.MoveOutcome.DuplicateElsewhere)
        val dup = outcome as ContactKeyDirectory.MoveOutcome.DuplicateElsewhere
        assertTrue(dup.otherRawId == 9L)
        assertTrue(dup.otherName == "alice")
    }
}
