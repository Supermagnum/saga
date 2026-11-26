package org.saga.contacts

/** Display helpers for 64-hex ephemeral-session fingerprints. */
object FingerprintFormat {
    /** Groups of 4 hex chars separated by spaces (lowercase). */
    fun grouped(fingerprintHex: String): String {
        val hex = fingerprintHex.trim().lowercase()
        require(hex.length == 64) { "expected 64 hex chars" }
        return hex.chunked(4).joinToString(" ")
    }
}
