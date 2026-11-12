package org.saga.audio

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioRecord
import android.media.AudioTrack
import android.media.MediaRecorder
import android.media.audiofx.AcousticEchoCanceler
import android.media.audiofx.AutomaticGainControl
import android.media.audiofx.NoiseSuppressor
import android.os.Process
import android.util.Log
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Capture/playback for Iroh voice using platform VOICE_COMMUNICATION so AEC/NS/AGC apply.
 *
 * Chosen over Oboe/cpal: AcousticEchoCanceler/NoiseSuppressor attach to
 * [AudioRecord] session IDs; Telecom managed calls expect this path; cpal is
 * reserved for the Linux peer (spec §10) and does not expose the voice-comm
 * preset cleanly on Android.
 *
 * No per-frame heap allocations on the audio threads — direct [ByteBuffer]s reused.
 */
class SagaVoiceAudioEngine(
    private val context: Context,
    private val onCaptureFrame: (ByteBuffer) -> Unit,
    private val fillPlaybackFrame: (ByteBuffer) -> Int
) {
    companion object {
        private const val TAG = "[Saga VoiceAudio]"
        const val SAMPLE_RATE = 48_000
        const val CHANNELS = 1
        const val FRAME_SAMPLES = 960 // 20 ms @ 48 kHz
        private const val BYTES_PER_SAMPLE = 2
        const val FRAME_BYTES = FRAME_SAMPLES * CHANNELS * BYTES_PER_SAMPLE
    }

    private val running = AtomicBoolean(false)
    private val muted = AtomicBoolean(false)
    private val held = AtomicBoolean(false)

    private var record: AudioRecord? = null
    private var track: AudioTrack? = null
    private var aec: AcousticEchoCanceler? = null
    private var ns: NoiseSuppressor? = null
    private var agc: AutomaticGainControl? = null

    private var captureThread: Thread? = null
    private var playbackThread: Thread? = null

    // Pre-allocated direct buffers — never replaced on the audio path.
    // nativeOrder matches AudioRecord short[] / AudioTrack byte semantics on-device;
    // x86_64/arm64 emulators are little-endian (same as Rust from_le_bytes).
    private val captureBuf: ByteBuffer =
        ByteBuffer.allocateDirect(FRAME_BYTES).order(ByteOrder.nativeOrder())
    private val playbackBuf: ByteBuffer =
        ByteBuffer.allocateDirect(FRAME_BYTES).order(ByteOrder.nativeOrder())
    private val silence = ByteArray(FRAME_BYTES)
    private val playbackScratch = ByteArray(FRAME_BYTES)
    private val captureShorts = ShortArray(FRAME_SAMPLES)

    var aecAttached: Boolean = false
        private set
    var nsAttached: Boolean = false
        private set
    var agcAttached: Boolean = false
        private set

    fun start() {
        if (!running.compareAndSet(false, true)) return
        val inBuf = AudioRecord.getMinBufferSize(
            SAMPLE_RATE,
            AudioFormat.CHANNEL_IN_MONO,
            AudioFormat.ENCODING_PCM_16BIT
        ).coerceAtLeast(FRAME_BYTES * 4)
        val outBuf = AudioTrack.getMinBufferSize(
            SAMPLE_RATE,
            AudioFormat.CHANNEL_OUT_MONO,
            AudioFormat.ENCODING_PCM_16BIT
        ).coerceAtLeast(FRAME_BYTES * 4)

        val rec = AudioRecord(
            MediaRecorder.AudioSource.VOICE_COMMUNICATION,
            SAMPLE_RATE,
            AudioFormat.CHANNEL_IN_MONO,
            AudioFormat.ENCODING_PCM_16BIT,
            inBuf
        )
        if (rec.state != AudioRecord.STATE_INITIALIZED) {
            Log.e(TAG, "AudioRecord init failed")
            running.set(false)
            rec.release()
            return
        }
        Log.i(
            TAG,
            "AudioRecord sr=${rec.sampleRate} ch=${rec.channelCount} fmt=${rec.audioFormat} " +
                "want sr=$SAMPLE_RATE ch=$CHANNELS enc=PCM16 frameSamples=$FRAME_SAMPLES " +
                "byteOrder=${captureBuf.order()}"
        )

        attachEffects(rec.audioSessionId)

        val attrs = AudioAttributes.Builder()
            .setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION)
            .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
            .build()
        val format = AudioFormat.Builder()
            .setSampleRate(SAMPLE_RATE)
            .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
            .setChannelMask(AudioFormat.CHANNEL_OUT_MONO)
            .build()
        val trk = AudioTrack(
            attrs,
            format,
            outBuf,
            AudioTrack.MODE_STREAM,
            AudioManager.AUDIO_SESSION_ID_GENERATE
        )
        if (trk.state != AudioTrack.STATE_INITIALIZED) {
            Log.e(TAG, "AudioTrack init failed")
            running.set(false)
            releaseEffects()
            rec.release()
            trk.release()
            return
        }
        Log.i(
            TAG,
            "AudioTrack sr=${trk.sampleRate} ch=${trk.channelCount} fmt=${trk.audioFormat} " +
                "want sr=$SAMPLE_RATE ch=$CHANNELS enc=PCM16 frameSamples=$FRAME_SAMPLES " +
                "byteOrder=${playbackBuf.order()}"
        )

        record = rec
        track = trk
        rec.startRecording()
        trk.play()

        captureThread = Thread({
            Process.setThreadPriority(Process.THREAD_PRIORITY_URGENT_AUDIO)
            captureLoop()
        }, "saga-capture").also { it.start() }

        playbackThread = Thread({
            Process.setThreadPriority(Process.THREAD_PRIORITY_URGENT_AUDIO)
            playbackLoop()
        }, "saga-playback").also { it.start() }

        Log.i(
            TAG,
            "started aecAttached=$aecAttached nsAttached=$nsAttached agcAttached=$agcAttached " +
                "sessionId=${rec.audioSessionId}"
        )
    }

    fun setMuted(value: Boolean) {
        muted.set(value)
        Log.i(TAG, "muted=$value (capture continues; send path gated upstream)")
    }

    fun isMuted(): Boolean = muted.get()

    fun setHeld(value: Boolean) {
        held.set(value)
        Log.i(TAG, "held=$value")
    }

    fun isHeld(): Boolean = held.get()

    fun stop() {
        if (!running.compareAndSet(true, false)) return
        captureThread?.join(500)
        playbackThread?.join(500)
        captureThread = null
        playbackThread = null
        try {
            record?.stop()
        } catch (_: Exception) {
        }
        try {
            track?.stop()
        } catch (_: Exception) {
        }
        releaseEffects()
        record?.release()
        track?.release()
        record = null
        track = null
        Log.i(TAG, "stopped")
    }

    private fun attachEffects(sessionId: Int) {
        aecAttached = false
        nsAttached = false
        agcAttached = false
        if (AcousticEchoCanceler.isAvailable()) {
            aec = AcousticEchoCanceler.create(sessionId)?.also {
                it.enabled = true
                aecAttached = it.enabled
            }
        }
        if (NoiseSuppressor.isAvailable()) {
            ns = NoiseSuppressor.create(sessionId)?.also {
                it.enabled = true
                nsAttached = it.enabled
            }
        }
        if (AutomaticGainControl.isAvailable()) {
            agc = AutomaticGainControl.create(sessionId)?.also {
                it.enabled = true
                agcAttached = it.enabled
            }
        }
        Log.i(
            TAG,
            "effects aecAvailable=${AcousticEchoCanceler.isAvailable()} aecAttached=$aecAttached " +
                "nsAvailable=${NoiseSuppressor.isAvailable()} nsAttached=$nsAttached " +
                "agcAvailable=${AutomaticGainControl.isAvailable()} agcAttached=$agcAttached"
        )
    }

    private fun releaseEffects() {
        aec?.release(); aec = null
        ns?.release(); ns = null
        agc?.release(); agc = null
    }

    private fun captureLoop() {
        val rec = record ?: return
        // Pace to one frame per 20 ms. Emulator AudioRecord can return
        // immediately; without pacing, debug PCM inject burns the whole WAV
        // into a small Rust capture queue and most speech is dropped.
        val frameNs = 1_000_000_000L * FRAME_SAMPLES / SAMPLE_RATE
        try {
            while (running.get()) {
                val t0 = System.nanoTime()
                if (held.get()) {
                    sleepFrameRemainder(t0, frameNs)
                    continue
                }
                val n = rec.read(captureShorts, 0, FRAME_SAMPLES)
                if (n <= 0) {
                    sleepFrameRemainder(t0, frameNs)
                    continue
                }
                captureBuf.clear()
                for (i in 0 until n) {
                    captureBuf.putShort(captureShorts[i])
                }
                while (captureBuf.position() < FRAME_BYTES) {
                    captureBuf.putShort(0)
                }
                captureBuf.flip()
                if (!muted.get()) {
                    // Inject (if armed) overwrites mic PCM inside onCaptureFrame.
                    onCaptureFrame(captureBuf)
                }
                sleepFrameRemainder(t0, frameNs)
            }
        } catch (t: Throwable) {
            Log.e(TAG, "captureLoop crashed: ${t.message}", t)
            running.set(false)
        }
    }

    private fun sleepFrameRemainder(t0: Long, frameNs: Long) {
        val elapsed = System.nanoTime() - t0
        val remain = frameNs - elapsed
        if (remain >= 1_000_000L) {
            try {
                Thread.sleep(remain / 1_000_000L, (remain % 1_000_000L).toInt())
            } catch (_: InterruptedException) {
                Thread.currentThread().interrupt()
            }
        }
    }

    private fun playbackLoop() {
        val trk = track ?: return
        try {
            while (running.get()) {
                if (held.get()) {
                    trk.write(silence, 0, silence.size)
                    continue
                }
                playbackBuf.clear()
                val wrote = fillPlaybackFrame(playbackBuf).coerceIn(0, FRAME_BYTES)
                if (wrote <= 0) {
                    trk.write(silence, 0, silence.size)
                } else {
                    // JNI writes into the direct buffer without advancing position.
                    playbackBuf.position(0)
                    playbackBuf.limit(wrote)
                    playbackBuf.get(playbackScratch, 0, wrote)
                    trk.write(playbackScratch, 0, wrote)
                }
            }
        } catch (t: Throwable) {
            Log.e(TAG, "playbackLoop crashed: ${t.message}", t)
            running.set(false)
        }
    }
}
