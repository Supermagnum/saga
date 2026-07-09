package org.saga.iroh

import java.util.concurrent.ConcurrentHashMap

/** Carries contact lookup key and display name from dial UI to ConnectionService. */
object PendingDialLookup {
    data class Pending(val lookupKey: String, val contactName: String)

    private val pending = ConcurrentHashMap<String, Pending>()

    fun put(peerEndpoint: String, lookupKey: String, contactName: String) {
        pending[peerEndpoint] = Pending(lookupKey, contactName)
    }

    fun take(peerEndpoint: String): Pending? = pending.remove(peerEndpoint)
}
