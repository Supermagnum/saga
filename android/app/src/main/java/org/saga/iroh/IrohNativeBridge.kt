package org.saga.iroh

import android.util.Log
import java.nio.ByteBuffer
import java.util.concurrent.ConcurrentHashMap

object IrohNativeBridge {
    private const val TAG = "[Iroh Native Bridge]"
    private const val LIB = "saga_iroh_core"

    const val HANDSHAKE_PENDING = 0
    const val HANDSHAKE_ENCRYPTED = 1
    const val HANDSHAKE_FAILED = 2

    const val RELAY_PENDING = 0
    const val RELAY_READY = 1
    const val RELAY_FAILED = 2

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

    fun setRelayUrl(relayUrl: String) {
        if (ensureLoaded()) nativeSetRelayUrl(relayUrl)
    }

    fun pollRelayReady(): Int {
        if (!ensureLoaded()) return RELAY_READY
        return nativePollRelayReady()
    }

    fun setForceHandshakeFail(force: Boolean) {
        if (ensureLoaded()) nativeSetForceHandshakeFail(force)
    }

    fun setForceBindingPerturb(force: Boolean) {
        if (ensureLoaded()) nativeSetForceBindingPerturb(force)
    }

    fun pollHandshake(sessionId: String): Int {
        if (!ensureLoaded()) return HANDSHAKE_ENCRYPTED
        return nativePollHandshake(sessionId)
    }

    /** mode: 1 = SRTP (encrypted), 2 = plain RTP (not-encrypted Iroh). */
    fun startMedia(sessionId: String, mode: Int): Boolean {
        if (!ensureLoaded()) return false
        return nativeStartMedia(sessionId, mode)
    }

    fun stopMedia(sessionId: String) {
        if (libraryLoaded) nativeStopMedia(sessionId)
    }

    fun pushCapturePcm(sessionId: String, frame: ByteBuffer) {
        if (libraryLoaded) nativePushCapturePcm(sessionId, frame)
    }

    fun pullPlaybackPcm(sessionId: String, frame: ByteBuffer): Int {
        if (!libraryLoaded) return 0
        return nativePullPlaybackPcm(sessionId, frame)
    }

    fun setMediaMuted(sessionId: String, muted: Boolean) {
        if (libraryLoaded) nativeSetMediaMuted(sessionId, muted)
    }

    fun setMediaHeld(sessionId: String, held: Boolean) {
        if (libraryLoaded) nativeSetMediaHeld(sessionId, held)
    }

    /** §5b re-key; returns swap duration in microseconds, or -1 on failure. */
    fun rekeyMedia(sessionId: String): Long {
        if (!ensureLoaded()) return -1L
        return nativeRekeyMedia(sessionId)
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

    @JvmStatic
    fun notifyIncomingCall(sessionId: String, lookupKey: String, remoteEndpointId: String) {
        Log.i(
            TAG,
            "notifyIncomingCall session=[$sessionId] lookup=[$lookupKey] remote=[$remoteEndpointId]"
        )
        IrohIncomingCallBridge.onIncomingFromNative(sessionId, lookupKey, remoteEndpointId)
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
    private external fun nativeSetRelayUrl(relayUrl: String)
    private external fun nativePollRelayReady(): Int
    private external fun nativeSetForceHandshakeFail(force: Boolean)
    private external fun nativeSetForceBindingPerturb(force: Boolean)
    private external fun nativePollHandshake(sessionId: String): Int
    private external fun nativeStartMedia(sessionId: String, mode: Int): Boolean
    private external fun nativeStopMedia(sessionId: String)
    private external fun nativePushCapturePcm(sessionId: String, frame: ByteBuffer)
    private external fun nativePullPlaybackPcm(sessionId: String, frame: ByteBuffer): Int
    private external fun nativeSetMediaMuted(sessionId: String, muted: Boolean)
    private external fun nativeSetMediaHeld(sessionId: String, held: Boolean)
    private external fun nativeRekeyMedia(sessionId: String): Long
}
