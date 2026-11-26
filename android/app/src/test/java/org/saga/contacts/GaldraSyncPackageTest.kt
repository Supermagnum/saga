package org.saga.contacts

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class GaldraSyncPackageTest {
    @Test
    fun mapPubkey_emptyRejectedWithoutNative() {
        val (fp, reason) = GaldraSyncPackage.mapPubkey(null)
        assertNull(fp)
        assertEquals("no public key", reason)
        val empty = GaldraSyncPackage.mapPubkey(ByteArray(0))
        assertNull(empty.first)
        assertEquals("no public key", empty.second)
    }

    @Test
    fun schemaVersionBounds() {
        assertEquals(1, GaldraSyncPackage.MIN_SCHEMA_VERSION)
        assertEquals(8, GaldraSyncPackage.MAX_SCHEMA_VERSION)
    }
}
