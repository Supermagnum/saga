package org.saga.iroh

import android.util.Log

class NativeIrohCallSession(
    override val peerId: IrohNodeId,
    override val sessionId: String = "native-${System.currentTimeMillis()}"
) : IrohCallSession {
    override fun connect(onConnected: () -> Unit, onFailed: (String) -> Unit) {
        IrohNativeBridge.register(
            sessionId,
            object : IrohNativeBridge.Listener {
                override fun onConnected() = onConnected()
                override fun onFailed(reason: String) = onFailed(reason)
            }
        )
        IrohNativeBridge.connect(sessionId, peerId.raw)
    }

    override fun disconnect() {
        IrohNativeBridge.disconnect(sessionId)
    }
}
