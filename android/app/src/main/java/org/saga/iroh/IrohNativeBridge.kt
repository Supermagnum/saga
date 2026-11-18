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

    /** Mesh link states reported by native via [notifyMeshPeerState]. */
    const val MESH_PEER_PENDING = 0
    const val MESH_PEER_ENCRYPTED = 1
    const val MESH_PEER_FAILED = 2
    const val MESH_PEER_LEFT = 3

    @Volatile
    private var libraryLoaded = false

    /** null = not probed yet. */
    @Volatile
    private var meshAvailable: Boolean? = null

    private val listeners = ConcurrentHashMap<String, Listener>()

    interface Listener {
        fun onConnected()
        fun onFailed(reason: String)
    }

    fun isNativeAvailable(): Boolean {
        ensureLoaded()
        return libraryLoaded
    }

    /** Must run once from Application.onCreate before binding the Iroh endpoint. */
    fun installAndroidContext(context: android.content.Context) {
        if (ensureLoaded()) nativeInstallAndroidContext(context.applicationContext)
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

    /** Debug-only: peer re-injects decoded PCM into send (echo RTT). No-op in release. */
    fun setEchoMode(enabled: Boolean) {
        if (ensureLoaded()) nativeSetEchoMode(enabled)
    }

    /** Debug-only: receive-side drop % + optional jitter ms. No-op in release. */
    fun setRxFault(dropPct: Int, jitterMs: Int) {
        if (ensureLoaded()) nativeSetRxFault(dropPct, jitterMs)
    }

    /**
     * §16 voice disguise settings. mode: 0=off 1=pitch 2=formant 3=formant+pitch;
     * formantStrength: 0=mild 1=medium 2=strong.
     */
    fun setDisguiseSettings(
        mode: Int,
        pitchSemitones: Float,
        formantStrength: Int,
        randomEachCall: Boolean,
        callEnabled: Boolean
    ) {
        if (ensureLoaded()) {
            nativeSetDisguiseSettings(
                mode,
                pitchSemitones,
                formantStrength,
                randomEachCall,
                callEnabled
            )
        }
    }

    fun localDisguiseActive(): Boolean {
        if (!ensureLoaded()) return false
        return nativeLocalDisguiseActive()
    }

    fun peerDisguiseActive(sessionId: String): Boolean {
        if (!ensureLoaded()) return false
        return nativePeerDisguiseActive(sessionId)
    }

    /** In-place s16le mono PCM through the current disguise settings (Test my voice). */
    fun processDisguisePcm(frame: ByteBuffer) {
        if (ensureLoaded()) nativeProcessDisguisePcm(frame)
    }

    /** Native -> Kotlin: peer disguise chip (does not affect padlock / security). */
    @JvmStatic
    fun notifyPeerDisguise(sessionId: String, active: Boolean) {
        Log.i(TAG, "notifyPeerDisguise session=[$sessionId] active=$active")
        org.saga.audio.VoiceDisguiseStore.onPeerDisguise(sessionId, active)
    }

    fun pollHandshake(sessionId: String): Int {
        if (!ensureLoaded()) return HANDSHAKE_ENCRYPTED
        return nativePollHandshake(sessionId)
    }

    /**
     * 64-char lowercase hex fingerprint authenticated by a pairwise session handshake.
     * Null when unknown — never treat null as verified (G4/G5).
     */
    fun sessionPeerFingerprint(sessionId: String): String? {
        if (!ensureLoaded()) return null
        return try {
            nativeSessionPeerFingerprint(sessionId)
        } catch (e: UnsatisfiedLinkError) {
            Log.w(TAG, "sessionPeerFingerprint unavailable: ${e.message}")
            null
        }
    }

    /**
     * Push a signaling-only introduction over an existing encrypted pairwise session so the
     * peer can show "introduced by" without a media mesh link (G4a; G3 stays PENDING_MESH).
     */
    fun announceIntroducedPeer(
        sessionId: String,
        groupId: String,
        lookupKey: String,
        endpointId: String,
        fingerprintHex: String,
        introducerName: String
    ): Boolean {
        if (!ensureLoaded()) return false
        return try {
            nativeAnnounceIntroducedPeer(
                sessionId, groupId, lookupKey, endpointId, fingerprintHex, introducerName
            )
        } catch (e: UnsatisfiedLinkError) {
            Log.w(TAG, "announceIntroducedPeer unavailable: ${e.message}")
            false
        }
    }

    /** Native -> Kotlin: roster introduction on an encrypted pairwise link. */
    @JvmStatic
    fun notifyRosterIntroduction(
        groupId: String,
        lookupKey: String,
        endpointId: String,
        fingerprintHex: String,
        introducerName: String
    ) {
        Log.i(
            TAG,
            "notifyRosterIntroduction group=[$groupId] peer=[$lookupKey] by=[$introducerName]"
        )
        org.saga.telecom.GroupCallCoordinator.onRosterIntroductionNative(
            groupId, lookupKey, endpointId, fingerprintHex, introducerName
        )
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

    fun setMediaMutedAll(muted: Boolean) {
        if (libraryLoaded) nativeSetMediaMutedAll(muted)
    }

    fun setMediaHeld(sessionId: String, held: Boolean) {
        if (libraryLoaded) nativeSetMediaHeld(sessionId, held)
    }

    /**
     * Callee answer: flush pre-answer JB, arm send/playout, emit answered signal to peer.
     */
    fun onLocalAnswered(sessionId: String): Boolean {
        if (!ensureLoaded()) return false
        return nativeOnLocalAnswered(sessionId)
    }

    /** True once local answer (callee) or remote answered signal (caller) armed send. */
    fun isSendArmed(sessionId: String): Boolean {
        if (!ensureLoaded()) return false
        return nativeIsSendArmed(sessionId)
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

    /**
     * True when the loaded native library exports the mesh JNI (nativeMesh*).
     *
     * Dual path (see GroupCallCoordinator): when true, a group is one native mesh with one
     * link id per peer; otherwise the Kotlin scaffold runs one 1:1 native session per peer.
     * TODO(mesh-jni): signatures below are the Kotlin-side contract; reconcile with the Rust
     * agent's final JNI names when it lands, then remove the pairwise scaffold.
     */
    fun isMeshNativeAvailable(): Boolean {
        meshAvailable?.let { return it }
        if (!ensureLoaded()) return false
        val available = try {
            nativeMeshIsAvailable()
        } catch (e: UnsatisfiedLinkError) {
            Log.i(TAG, "Mesh JNI not present: ${e.message}")
            false
        }
        meshAvailable = available
        return available
    }

    /** Local Iroh endpoint id (64 hex), or null when the endpoint is not up. */
    fun localEndpointId(): String? {
        if (!ensureLoaded()) return null
        return try {
            nativeLocalEndpointId()
        } catch (e: UnsatisfiedLinkError) {
            Log.w(TAG, "localEndpointId unavailable: ${e.message}")
            null
        }
    }

    fun meshOpen(groupId: String): Boolean {
        if (!isMeshNativeAvailable()) return false
        return nativeMeshOpen(groupId)
    }

    /** Add (or start dialling) one peer link inside the mesh group. */
    fun meshAddPeer(groupId: String, linkId: String, peerId: String): Boolean {
        if (!isMeshNativeAvailable()) return false
        return nativeMeshAddPeer(groupId, linkId, peerId)
    }

    /** Close one link only; other links of the mesh keep running. */
    fun meshRemovePeer(groupId: String, linkId: String) {
        if (isMeshNativeAvailable()) nativeMeshRemovePeer(groupId, linkId)
    }

    /** Local user leaves the mesh. Other participants keep their mutual links (no host). */
    fun meshClose(groupId: String) {
        if (isMeshNativeAvailable()) nativeMeshClose(groupId)
    }

    /**
     * 64-char lowercase hex fingerprint authenticated by this link's handshake, or null when
     * unknown. Null must never be treated as verified.
     */
    fun meshPeerFingerprint(groupId: String, linkId: String): String? {
        if (!isMeshNativeAvailable()) return null
        return nativeMeshPeerFingerprint(groupId, linkId)
    }

    /** Native -> Kotlin: one mesh link changed state (MESH_PEER_*). */
    @JvmStatic
    fun notifyMeshPeerState(groupId: String, linkId: String, state: Int) {
        Log.i(TAG, "notifyMeshPeerState group=[$groupId] link=[$linkId] state=[$state]")
        org.saga.telecom.GroupCallCoordinator.onMeshPeerState(groupId, linkId, state)
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
    private external fun nativeInstallAndroidContext(context: android.content.Context)
    private external fun nativeSetDevIdentity(peerLabel: String)
    private external fun nativeSetRelayUrl(relayUrl: String)
    private external fun nativePollRelayReady(): Int
    private external fun nativeSetForceHandshakeFail(force: Boolean)
    private external fun nativeSetForceBindingPerturb(force: Boolean)
    private external fun nativeSetEchoMode(enabled: Boolean)
    private external fun nativeSetRxFault(dropPct: Int, jitterMs: Int)
    private external fun nativeSetDisguiseSettings(
        mode: Int,
        pitchSemitones: Float,
        formantStrength: Int,
        randomEachCall: Boolean,
        callEnabled: Boolean
    )
    private external fun nativeLocalDisguiseActive(): Boolean
    private external fun nativePeerDisguiseActive(sessionId: String): Boolean
    private external fun nativeProcessDisguisePcm(pcm: ByteBuffer)
    private external fun nativePollHandshake(sessionId: String): Int
    private external fun nativeSessionPeerFingerprint(sessionId: String): String?
    private external fun nativeAnnounceIntroducedPeer(
        sessionId: String,
        groupId: String,
        lookupKey: String,
        endpointId: String,
        fingerprintHex: String,
        introducerName: String
    ): Boolean
    private external fun nativeStartMedia(sessionId: String, mode: Int): Boolean
    private external fun nativeStopMedia(sessionId: String)
    private external fun nativePushCapturePcm(sessionId: String, frame: ByteBuffer)
    private external fun nativePullPlaybackPcm(sessionId: String, frame: ByteBuffer): Int
    private external fun nativeSetMediaMuted(sessionId: String, muted: Boolean)
    private external fun nativeSetMediaMutedAll(muted: Boolean)
    private external fun nativeSetMediaHeld(sessionId: String, held: Boolean)
    private external fun nativeOnLocalAnswered(sessionId: String): Boolean
    private external fun nativeIsSendArmed(sessionId: String): Boolean
    private external fun nativeRekeyMedia(sessionId: String): Long
    private external fun nativeLocalEndpointId(): String?
    private external fun nativeMeshIsAvailable(): Boolean
    private external fun nativeMeshOpen(groupId: String): Boolean
    private external fun nativeMeshAddPeer(groupId: String, linkId: String, peerId: String): Boolean
    private external fun nativeMeshRemovePeer(groupId: String, linkId: String)
    private external fun nativeMeshClose(groupId: String)
    private external fun nativeMeshPeerFingerprint(groupId: String, linkId: String): String?
}
