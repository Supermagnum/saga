package org.saga.iroh

import android.util.Log
import java.util.concurrent.ConcurrentHashMap

object IrohNativeBridge {
    private const val TAG = "[Iroh Native Bridge]"
    private const val LIB = "saga_iroh_core"

    const val HANDSHAKE_PENDING = 0
    const val HANDSHAKE_ENCRYPTED = 1
    const val HANDSHAKE_FAILED = 2

    @Volatile
    private var libraryLoaded = false

    private val listeners = ConcurrentHashMap<String, Listener>()

    interface Listener {
        fun onConnected()
        fun onFailed(reason: String)
    }

    fun isNativeAvailable(): Boolean {
        ensureLoaded()
        return libraryLoaded
    }

    fun setDevIdentity(peerLabel: String) {
        if (ensureLoaded()) nativeSetDevIdentity(peerLabel)
    }

    fun setForceHandshakeFail(force: Boolean) {
        if (ensureLoaded()) nativeSetForceHandshakeFail(force)
    }

    fun pollHandshake(sessionId: String): Int {
        if (!ensureLoaded()) return HANDSHAKE_ENCRYPTED
        return nativePollHandshake(sessionId)
    }

    fun connect(sessionId: String, peerId: String) {
        if (!ensureLoaded()) {
            listeners[sessionId]?.onConnected()
            return
        }
        nativeConnect(sessionId, peerId)
    }

    fun disconnect(sessionId: String) {
        if (libraryLoaded) nativeDisconnect(sessionId)
        unregister(sessionId)
    }

    fun register(sessionId: String, listener: Listener) {
        listeners[sessionId] = listener
    }

    fun unregister(sessionId: String) {
        listeners.remove(sessionId)
    }

    @JvmStatic
    fun notifyConnected(sessionId: String) {
        listeners.remove(sessionId)?.onConnected()
    }

    @JvmStatic
    fun notifyFailed(sessionId: String, reason: String) {
        listeners.remove(sessionId)?.onFailed(reason)
    }

    private fun ensureLoaded(): Boolean {
        if (libraryLoaded) return true
        return try {
            System.loadLibrary(LIB)
            libraryLoaded = nativeIsAvailable()
            libraryLoaded
        } catch (e: UnsatisfiedLinkError) {
            Log.w(TAG, "Native library unavailable: ${e.message}")
            false
        }
    }

    private external fun nativeConnect(sessionId: String, peerId: String)
    private external fun nativeDisconnect(sessionId: String)
    private external fun nativeIsAvailable(): Boolean
    private external fun nativeSetDevIdentity(peerLabel: String)
    private external fun nativeSetForceHandshakeFail(force: Boolean)
    private external fun nativePollHandshake(sessionId: String): Int
}
