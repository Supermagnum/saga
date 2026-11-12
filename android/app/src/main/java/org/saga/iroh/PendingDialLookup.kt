package org.saga.iroh

import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicLong

/** Carries contact lookup key and display name from dial UI to ConnectionService. */
object PendingDialLookup {
    data class Pending(
        val lookupKey: String,
        val contactName: String,
        val createdAtMs: Long = System.currentTimeMillis()
    )

    private val pending = ConcurrentHashMap<String, Pending>()
    private val lastIrohDialAtMs = AtomicLong(0L)

    /** Window for detecting Telecom SIM substitution after an Iroh placeCall attempt. */
    private const val RECENT_IROH_DIAL_MS = 15_000L

    fun put(peerEndpoint: String, lookupKey: String, contactName: String) {
        pending[peerEndpoint] = Pending(lookupKey, contactName)
        lastIrohDialAtMs.set(System.currentTimeMillis())
    }

    fun take(peerEndpoint: String): Pending? = pending.remove(peerEndpoint)

    fun clear(peerEndpoint: String) {
        pending.remove(peerEndpoint)
    }

    fun clearAll() {
        pending.clear()
    }

    /** True if an Iroh dial was requested recently (pending map may already be consumed). */
    fun hasRecentIrohDial(withinMs: Long = RECENT_IROH_DIAL_MS): Boolean {
        val last = lastIrohDialAtMs.get()
        if (last == 0L) return false
        return System.currentTimeMillis() - last <= withinMs
    }

    fun markIrohDialConsumed() {
        lastIrohDialAtMs.set(0L)
    }
}
