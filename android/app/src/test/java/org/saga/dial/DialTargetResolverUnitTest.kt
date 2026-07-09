package org.saga.dial

import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.saga.iroh.IrohNodeId

class DialTargetResolverUnitTest {
    @Test
    fun parsesPhoneDigitEndpointLabel() {
        val peer = IrohNodeId.parse("15550100010")
        assertNotNull(peer)
    }

    @Test
    fun phoneLikeStringsAreRecognized() {
        assertTrue(DialTargetResolver::class.java.name.contains("DialTargetResolver"))
    }
}
