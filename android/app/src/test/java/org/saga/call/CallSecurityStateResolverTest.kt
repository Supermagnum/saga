package org.saga.call

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CallSecurityStateResolverTest {
    @Test
    fun cellular_alwaysKeyNotFoundAndUnsecureCue() {
        val snapshot = CallSecurityStateResolver.resolve(
            origin = CallOrigin.CELLULAR,
            contactKeyResolvable = true,
            handshakeEncrypted = false
        )
        assertEquals(PadlockDisplayState.KEY_NOT_FOUND, snapshot.padlockState)
        assertTrue(snapshot.playUnsecureCue)
        assertFalse(snapshot.playSecureCue)
        assertFalse(snapshot.showDowngradeModal)
    }

    @Test
    fun iroh_encrypted_playsSecureCue() {
        val snapshot = CallSecurityStateResolver.resolve(
            origin = CallOrigin.IROH,
            handshakeEncrypted = true,
            contactKeyResolvable = true
        )
        assertEquals(PadlockDisplayState.ENCRYPTED, snapshot.padlockState)
        assertTrue(snapshot.playSecureCue)
        assertFalse(snapshot.playUnsecureCue)
    }

    @Test
    fun iroh_downgrade_playsUnsecureCueAndModal() {
        val snapshot = CallSecurityStateResolver.resolve(
            origin = CallOrigin.IROH,
            handshakeFailed = true,
            hasEncryptionHistory = true,
            contactKeyResolvable = true
        )
        assertEquals(PadlockDisplayState.DOWNGRADED, snapshot.padlockState)
        assertTrue(snapshot.playUnsecureCue)
        assertTrue(snapshot.showDowngradeModal)
    }
}
