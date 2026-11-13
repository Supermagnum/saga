package org.saga.audio

import android.content.Context
import android.telecom.CallAudioState
import android.util.Log
import org.saga.handshake.SagaHandshakeState
import org.saga.iroh.IrohNativeBridge
import java.util.concurrent.ConcurrentHashMap

/**
 * Starts/stops voice I/O after security resolves; mute/hold/routing via Telecom.
 *
 * Encrypted → SRTP media; not-encrypted → plain RTP (after §5a continue when required).
 *
 * Incoming calls may start the native RoQ receive path at handshake Encrypted while
 * still RINGING so early RTP is not lost before answer; AudioRecord/AudioTrack attach
 * only when [startAudioIo] runs (typically on answer).
 */
object SagaCallMediaController {
    private const val TAG = "[Saga CallMedia]"

    private val engines = ConcurrentHashMap<String, SagaVoiceAudioEngine>()
    private val nativeStarted = ConcurrentHashMap<String, Boolean>()
    private val audioStarted = ConcurrentHashMap<String, Boolean>()
    private val rustSessions = ConcurrentHashMap<String, String>()

    fun onSecurityResolved(
        context: Context,
        telecomCallId: String,
        rustSessionId: String?,
        state: SagaHandshakeState,
        userContinuedPast5a: Boolean
    ) {
        if (!startNativeReceive(telecomCallId, rustSessionId, state, userContinuedPast5a)) {
            return
        }
        startAudioIo(context, telecomCallId)
    }

    /**
     * Open SRTP/RoQ ReceiveFlow + JB + tick without AudioRecord/AudioTrack.
     * Safe to call while RINGING so the callee can ingest before answer.
     */
    fun startNativeReceive(
        telecomCallId: String,
        rustSessionId: String?,
        state: SagaHandshakeState,
        userContinuedPast5a: Boolean = false
    ): Boolean {
        if (nativeStarted[telecomCallId] == true) return true
        val encrypted = state == SagaHandshakeState.Encrypted
        val allowPlain = when (state) {
            SagaHandshakeState.NeverEncrypted -> true
            SagaHandshakeState.Downgraded -> userContinuedPast5a
            SagaHandshakeState.Encrypted -> false
            else -> false
        }
        if (!encrypted && !allowPlain) {
            Log.i(TAG, "media declined call=[$telecomCallId] state=[$state] continued=$userContinuedPast5a")
            return false
        }
        if (rustSessionId.isNullOrBlank()) {
            Log.w(TAG, "no rust session for media call=[$telecomCallId]")
            return false
        }
        val mode = if (encrypted) 1 else 2 // 1=SRTP, 2=plain
        val started = IrohNativeBridge.startMedia(rustSessionId, mode)
        if (!started) {
            Log.e(TAG, "native startMedia failed call=[$telecomCallId]")
            return false
        }
        nativeStarted[telecomCallId] = true
        rustSessions[telecomCallId] = rustSessionId
        Log.i(
            TAG,
            "native receive started call=[$telecomCallId] encrypted=$encrypted (audio I/O deferred)"
        )
        return true
    }

    /** Attach mic/speaker once the call is answered / active. */
    fun startAudioIo(context: Context, telecomCallId: String) {
        if (audioStarted[telecomCallId] == true) return
        val rustSessionId = rustSessions[telecomCallId]
        if (rustSessionId.isNullOrBlank()) {
            Log.w(TAG, "startAudioIo: no native session call=[$telecomCallId]")
            return
        }
        if (nativeStarted[telecomCallId] != true) {
            Log.w(TAG, "startAudioIo: native receive not started call=[$telecomCallId]")
            return
        }
        val engine = SagaVoiceAudioEngine(
            context = context.applicationContext,
            onCaptureFrame = { buf ->
                // Debug inject replaces mic PCM when armed (RoQ metrics).
                if (SagaPcmDebug.injectActive()) {
                    SagaPcmDebug.fillInjectFrame(buf)
                }
                IrohNativeBridge.pushCapturePcm(rustSessionId, buf)
            },
            fillPlaybackFrame = { buf ->
                val n = IrohNativeBridge.pullPlaybackPcm(rustSessionId, buf)
                if (n > 0) {
                    SagaPcmDebug.dumpPlaybackFrame(buf, n)
                }
                n
            }
        )
        engines[telecomCallId] = engine
        audioStarted[telecomCallId] = true
        engine.start()
        Log.i(
            TAG,
            "audio I/O started call=[$telecomCallId] " +
                "aec=${engine.aecAttached} ns=${engine.nsAttached} agc=${engine.agcAttached}"
        )
    }

    fun setMuted(telecomCallId: String, rustSessionId: String?, muted: Boolean) {
        engines[telecomCallId]?.setMuted(muted)
        if (!rustSessionId.isNullOrBlank()) {
            IrohNativeBridge.setMediaMuted(rustSessionId, muted)
        }
    }

    /** Debug/E2E: mute every live engine + every RoQ media session. */
    fun setMutedAllForTest(muted: Boolean) {
        for ((_, engine) in engines) {
            engine.setMuted(muted)
        }
        IrohNativeBridge.setMediaMutedAll(muted)
        Log.i(TAG, "setMutedAllForTest muted=$muted engines=${engines.size}")
    }

    fun setHeld(telecomCallId: String, rustSessionId: String?, held: Boolean) {
        engines[telecomCallId]?.setHeld(held)
        if (!rustSessionId.isNullOrBlank()) {
            IrohNativeBridge.setMediaHeld(rustSessionId, held)
        }
    }

    fun stop(telecomCallId: String, rustSessionId: String?) {
        engines.remove(telecomCallId)?.stop()
        audioStarted.remove(telecomCallId)
        nativeStarted.remove(telecomCallId)
        rustSessions.remove(telecomCallId)
        if (!rustSessionId.isNullOrBlank()) {
            IrohNativeBridge.stopMedia(rustSessionId)
        }
    }

    fun logAudioRoute(state: CallAudioState?) {
        if (state == null) return
        Log.i(
            TAG,
            "CallAudioState route=${routeName(state.route)} muted=${state.isMuted} " +
                "supported=${state.supportedRouteMask}"
        )
    }

    private fun routeName(route: Int): String = when (route) {
        CallAudioState.ROUTE_EARPIECE -> "EARPIECE"
        CallAudioState.ROUTE_SPEAKER -> "SPEAKER"
        CallAudioState.ROUTE_BLUETOOTH -> "BLUETOOTH"
        CallAudioState.ROUTE_WIRED_HEADSET -> "WIRED_HEADSET"
        CallAudioState.ROUTE_WIRED_OR_EARPIECE -> "WIRED_OR_EARPIECE"
        else -> "OTHER($route)"
    }
}
