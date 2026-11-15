package org.saga.audio

import android.content.Context
import android.telecom.CallAudioState
import android.util.Log
import org.saga.handshake.SagaHandshakeState
import org.saga.iroh.IrohNativeBridge
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors
import java.util.concurrent.Future
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Starts/stops voice I/O after security resolves; mute/hold/routing via Telecom.
 *
 * Encrypted → SRTP media; not-encrypted → plain RTP (after §5a continue when required).
 *
 * Media is gated until the call is answered:
 * - Native RoQ receive may start at Encrypted while RINGING (callee) or at Encrypted
 *   (caller) so the ReceiveFlow exists, but send/playout stay disarmed.
 * - Callee [onLocalAnswered] flushes the JB, emits an answered signal, and arms I/O.
 * - Caller arms send/playout only after receiving that signal (no mic media before answer).
 */
object SagaCallMediaController {
    private const val TAG = "[Saga CallMedia]"
    private const val ANSWER_POLL_MS = 50L
    private const val ANSWER_WAIT_MS = 120_000L

    private val engines = ConcurrentHashMap<String, SagaVoiceAudioEngine>()
    private val nativeStarted = ConcurrentHashMap<String, Boolean>()
    private val audioStarted = ConcurrentHashMap<String, Boolean>()
    private val rustSessions = ConcurrentHashMap<String, String>()
    private val answerWaiters = ConcurrentHashMap<String, Future<*>>()
    private val scheduler = Executors.newSingleThreadScheduledExecutor { r ->
        Thread(r, "saga-media-answer-wait").apply { isDaemon = true }
    }

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
        val sid = rustSessions[telecomCallId] ?: return
        if (IrohNativeBridge.isSendArmed(sid)) {
            startAudioIo(context, telecomCallId)
        } else {
            awaitAnsweredThenStartAudio(context.applicationContext, telecomCallId, sid)
        }
    }

    /**
     * Open SRTP/RoQ ReceiveFlow + JB + tick without AudioRecord/AudioTrack.
     * Send/playout remain gated until [onLocalAnswered] or the peer answered signal.
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
            "native receive started call=[$telecomCallId] encrypted=$encrypted " +
                "(send/playout gated until answered)"
        )
        return true
    }

    /**
     * Callee answer path: flush pre-answer JB, signal peer, arm send/playout, start Audio I/O.
     */
    fun onLocalAnswered(context: Context, telecomCallId: String, rustSessionId: String?) {
        val sid = rustSessionId ?: rustSessions[telecomCallId]
        if (sid.isNullOrBlank()) {
            Log.w(TAG, "onLocalAnswered: no session call=[$telecomCallId]")
            return
        }
        if (nativeStarted[telecomCallId] != true) {
            Log.w(TAG, "onLocalAnswered: native not started call=[$telecomCallId]")
        }
        val ok = IrohNativeBridge.onLocalAnswered(sid)
        Log.i(TAG, "onLocalAnswered call=[$telecomCallId] nativeOk=$ok")
        cancelAnswerWait(telecomCallId)
        startAudioIo(context, telecomCallId)
    }

    /** Attach mic/speaker once send is armed (answered). */
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
        if (!IrohNativeBridge.isSendArmed(rustSessionId)) {
            Log.w(TAG, "startAudioIo: send not armed yet call=[$telecomCallId]")
            return
        }
        val engine = SagaVoiceAudioEngine(
            context = context.applicationContext,
            onCaptureFrame = { buf ->
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

    private fun awaitAnsweredThenStartAudio(
        appContext: Context,
        telecomCallId: String,
        rustSessionId: String
    ) {
        cancelAnswerWait(telecomCallId)
        val done = AtomicBoolean(false)
        val future = scheduler.scheduleWithFixedDelay({
            if (done.get()) return@scheduleWithFixedDelay
            if (audioStarted[telecomCallId] == true) {
                done.set(true)
                cancelAnswerWait(telecomCallId)
                return@scheduleWithFixedDelay
            }
            if (IrohNativeBridge.isSendArmed(rustSessionId)) {
                if (done.compareAndSet(false, true)) {
                    Log.i(TAG, "peer answered signal observed call=[$telecomCallId]")
                    startAudioIo(appContext, telecomCallId)
                    cancelAnswerWait(telecomCallId)
                }
            }
        }, ANSWER_POLL_MS, ANSWER_POLL_MS, TimeUnit.MILLISECONDS)
        answerWaiters[telecomCallId] = future
        scheduler.schedule({
            if (done.compareAndSet(false, true)) {
                Log.w(TAG, "answered wait timed out call=[$telecomCallId]")
                cancelAnswerWait(telecomCallId)
            }
        }, ANSWER_WAIT_MS, TimeUnit.MILLISECONDS)
    }

    private fun cancelAnswerWait(telecomCallId: String) {
        answerWaiters.remove(telecomCallId)?.cancel(false)
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
        cancelAnswerWait(telecomCallId)
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
