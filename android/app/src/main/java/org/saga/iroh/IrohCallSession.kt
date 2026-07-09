package org.saga.iroh

interface IrohCallSession {
    val peerId: IrohNodeId
    val sessionId: String
    fun connect(onConnected: () -> Unit, onFailed: (String) -> Unit)
    fun disconnect()
}
