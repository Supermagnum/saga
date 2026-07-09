package org.saga.dial

import org.saga.iroh.IrohNodeId

sealed class DialTarget {
    abstract val lookupKey: String

    data class Cellular(val number: String) : DialTarget() {
        override val lookupKey: String = number
    }

    data class Iroh(
        val peerId: IrohNodeId,
        override val lookupKey: String,
        val contactName: String? = null
    ) : DialTarget()
}
