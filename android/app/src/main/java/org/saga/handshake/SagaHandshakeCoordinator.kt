package org.saga.handshake

import android.content.Context
import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import org.saga.iroh.ContactKeyStore
import org.saga.iroh.IrohNativeBridge

/**
 * Parallel handshake probe over Iroh using mock-token when native library is available.
 */
class SagaHandshakeCoordinator(
    private val appContext: Context,
    private val encryptionEstablishedStore: EncryptionEstablishedStore
) {
    companion object {
        private const val TAG = "[Saga Handshake Coordinator]"
    }

    private val scope = CoroutineScope(Dispatchers.Default + SupervisorJob())

    fun start(peerAddress: String, sessionId: String, onState: (SagaHandshakeState) -> Unit) {
        scope.launch {
            onState(SagaHandshakeState.Securing)
            IrohNativeBridge.setForceHandshakeFail(SagaTestFlags.isForceHandshakeFail(appContext))

            val poll = if (IrohNativeBridge.isNativeAvailable()) {
                IrohNativeBridge.pollHandshake(sessionId)
            } else {
                IrohNativeBridge.HANDSHAKE_FAILED
            }

            val finalState = when (poll) {
                IrohNativeBridge.HANDSHAKE_ENCRYPTED -> {
                    if (!ContactKeyStore.hasResolvableKey(appContext, peerAddress)) {
                        SagaHandshakeState.NeverEncrypted
                    } else {
                        encryptionEstablishedStore.markEncryptionEstablished(peerAddress)
                        SagaHandshakeState.Encrypted
                    }
                }
                IrohNativeBridge.HANDSHAKE_FAILED -> resolveFailure(peerAddress)
                else -> resolveFailure(peerAddress)
            }
            onState(finalState)
        }
    }

    fun resolveFailure(contactKey: String): SagaHandshakeState {
        return if (encryptionEstablishedStore.hasEncryptionHistory(contactKey)) {
            SagaHandshakeState.Downgraded
        } else {
            SagaHandshakeState.NeverEncrypted
        }
    }
}
