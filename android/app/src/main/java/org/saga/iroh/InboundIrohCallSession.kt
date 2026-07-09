package org.saga.iroh

/**
 * Callee-side session: Rust already accepted the Iroh connection and registered [sessionId].
 */
class InboundIrohCallSession(
    override val peerId: IrohNodeId,
    override val sessionId: String
) : IrohCallSession {
    override fun connect(onConnected: () -> Unit, onFailed: (String) -> Unit) {
        onConnected()
    }

    override fun disconnect() {
        IrohNativeBridge.disconnect(sessionId)
    }
}
