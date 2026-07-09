package org.saga.iroh

data class IrohNodeId(val raw: String) {
    companion object {
        fun parse(raw: String): IrohNodeId? {
            val trimmed = raw.trim()
            if (trimmed.length < 8) return null
            if (!trimmed.all { it.isLetterOrDigit() || it == '-' || it == '_' }) return null
            return IrohNodeId(trimmed)
        }
    }
}
