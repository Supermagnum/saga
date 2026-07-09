package org.saga.iroh

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class IrohNodeIdTest {
    @Test
    fun parse_validId() {
        val id = IrohNodeId.parse("abc12345")
        assertNotNull(id)
        assertEquals("abc12345", id?.raw)
    }

    @Test
    fun parse_tooShort() {
        assertNull(IrohNodeId.parse("abc"))
    }

    @Test
    fun contactKeyStore_nokeySuffix() {
        assertFalse(ContactKeyStore.hasResolvableKey("peer-nokey"))
        assertTrue(ContactKeyStore.hasResolvableKey("peer-valid12"))
    }
}
