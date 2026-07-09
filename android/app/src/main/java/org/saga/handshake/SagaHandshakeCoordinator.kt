package org.saga.handshake

import android.content.Context
import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
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
        private const val POLL_INTERVAL_MS = 200L
        private const val POLL_MAX_MS = 120_000L
    }

    private val scope = CoroutineScope(Dispatchers.Default + SupervisorJob())

    fun start(peerAddress: String, sessionId: String, onState: (SagaHandshakeState) -> Unit) {
        scope.launch {
            onState(SagaHandshakeState.Securing)
            IrohNativeBridge.setForceHandshakeFail(SagaTestFlags.isForceHandshakeFail(appContext))

            val poll = if (IrohNativeBridge.isNativeAvailable()) {
                pollWithRetry(sessionId)
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

    private suspend fun pollWithRetry(sessionId: String): Int {
        val deadline = System.currentTimeMillis() + POLL_MAX_MS
        while (System.currentTimeMillis() < deadline) {
            when (IrohNativeBridge.pollHandshake(sessionId)) {
                IrohNativeBridge.HANDSHAKE_ENCRYPTED -> return IrohNativeBridge.HANDSHAKE_ENCRYPTED
                IrohNativeBridge.HANDSHAKE_FAILED -> return IrohNativeBridge.HANDSHAKE_FAILED
                else -> delay(POLL_INTERVAL_MS)
            }
        }
        return IrohNativeBridge.HANDSHAKE_FAILED
    }
}
