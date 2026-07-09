package org.saga.iroh

object IrohSessionFactory {
    fun create(peerId: IrohNodeId): IrohCallSession {
        return if (IrohNativeBridge.isNativeAvailable()) {
            NativeIrohCallSession(peerId)
        } else {
            StubIrohCallSession(peerId)
        }
    }
}
