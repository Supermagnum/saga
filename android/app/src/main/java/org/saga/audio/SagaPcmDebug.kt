package org.saga.audio

import android.util.Log
import java.io.File
import java.io.FileOutputStream
import java.io.RandomAccessFile
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

/**
 * Debug-only PCM inject/dump for RoQ media metrics (emulator E2E).
 *
 * Inject: feed WAV/PCM16 LE mono 48 kHz frames into the capture path (skip mic).
 * Dump: append decoded playback frames to a raw PCM file for host scoring.
 */
object SagaPcmDebug {
    private const val TAG = "[Saga PcmDebug]"

    private val injectPath = AtomicReference<String?>(null)
    private val dumpPath = AtomicReference<String?>(null)
    private val injectCursor = AtomicReference(0)
    private val injectSamples = AtomicReference<ShortArray?>(null)
    private val dumping = AtomicBoolean(false)
    private var dumpStream: FileOutputStream? = null
    private var injectVerifyStream: FileOutputStream? = null
    private val injectFrames = java.util.concurrent.atomic.AtomicLong(0)

    /** Configure inject WAV path (48 kHz mono PCM16 WAV). Cleared when null/blank. */
    @JvmStatic
    fun setInjectWav(path: String?) {
        if (path.isNullOrBlank()) {
            injectPath.set(null)
            injectSamples.set(null)
            injectCursor.set(0)
            Log.i(TAG, "inject cleared")
            return
        }
        val samples = loadWavMono48k(File(path))
        if (samples == null) {
            Log.e(TAG, "inject load failed path=[$path]")
            return
        }
        injectPath.set(path)
        injectSamples.set(samples)
        injectCursor.set(0)
        injectFrames.set(0)
        try {
            injectVerifyStream?.close()
        } catch (_: Exception) {
        }
        // Side-channel: raw PCM actually fed into capture (pre-network) for scoring.
        injectVerifyStream = try {
            FileOutputStream(File("/data/data/org.saga/files/inject_verify.pcm"), false)
        } catch (e: Exception) {
            Log.w(TAG, "inject verify open failed: ${e.message}")
            null
        }
        Log.i(TAG, "inject armed path=[$path] samples=${samples.size}")
    }

    /** Configure playback dump path (raw s16le mono 48 kHz). */
    @JvmStatic
    fun setDumpPath(path: String?) {
        closeDumpLocked()
        if (path.isNullOrBlank()) {
            dumpPath.set(null)
            dumping.set(false)
            Log.i(TAG, "dump cleared")
            return
        }
        try {
            val f = File(path)
            f.parentFile?.mkdirs()
            dumpStream = FileOutputStream(f, false)
            dumpPath.set(path)
            dumping.set(true)
            Log.i(TAG, "dump armed path=[$path]")
        } catch (e: Exception) {
            Log.e(TAG, "dump open failed: ${e.message}")
            dumpPath.set(null)
            dumping.set(false)
        }
    }

    @JvmStatic
    fun injectActive(): Boolean = injectSamples.get() != null

    /**
     * If inject is armed, fill [buf] from the WAV (silence after EOF) and return true
     * so the mic frame can be skipped. Otherwise return false.
     */
    fun fillInjectFrame(buf: ByteBuffer): Boolean {
        val samples = injectSamples.get() ?: return false
        buf.clear()
        var cursor = injectCursor.get()
        val frame = ShortArray(SagaVoiceAudioEngine.FRAME_SAMPLES)
        for (i in frame.indices) {
            if (cursor < samples.size) {
                frame[i] = samples[cursor]
                cursor++
            } else {
                frame[i] = 0
            }
        }
        injectCursor.set(cursor)
        buf.asShortBuffer().put(frame)
        buf.position(0)
        buf.limit(SagaVoiceAudioEngine.FRAME_BYTES)
        val n = injectFrames.incrementAndGet()
        try {
            val bytes = ByteArray(SagaVoiceAudioEngine.FRAME_BYTES)
            val dup = buf.duplicate().order(ByteOrder.nativeOrder())
            dup.position(0)
            dup.limit(SagaVoiceAudioEngine.FRAME_BYTES)
            dup.get(bytes)
            injectVerifyStream?.write(bytes)
        } catch (_: Exception) {
        }
        if (n == 1L || n % 50L == 0L) {
            Log.i(TAG, "inject frames=$n cursor=$cursor")
        }
        return true
    }

    /** Append [len] bytes of a playback frame (direct buffer, little-endian s16) to the dump. */
    fun dumpPlaybackFrame(buf: ByteBuffer, len: Int) {
        if (!dumping.get() || len <= 0) return
        val stream = dumpStream ?: return
        try {
            val dup = buf.duplicate()
            dup.order(ByteOrder.nativeOrder())
            dup.position(0)
            dup.limit(len.coerceAtMost(dup.capacity()))
            val bytes = ByteArray(dup.remaining())
            dup.get(bytes)
            stream.write(bytes)
        } catch (e: Exception) {
            Log.w(TAG, "dump write failed: ${e.message}")
        }
    }

    @JvmStatic
    fun finishDump() {
        closeDumpLocked()
        Log.i(TAG, "dump finished path=[${dumpPath.get()}]")
    }

    private fun closeDumpLocked() {
        dumping.set(false)
        try {
            dumpStream?.flush()
            dumpStream?.close()
        } catch (_: Exception) {
        }
        dumpStream = null
    }

    /** Minimal WAV reader: PCM16 mono 48 kHz. Returns null on mismatch. */
    private fun loadWavMono48k(file: File): ShortArray? {
        if (!file.isFile) return null
        return try {
            RandomAccessFile(file, "r").use { raf ->
                val hdr = ByteArray(44)
                if (raf.read(hdr) < 44) return null
                if (String(hdr, 0, 4) != "RIFF" || String(hdr, 8, 4) != "WAVE") return null
                // Scan for fmt / data chunks (simple).
                raf.seek(12)
                var dataOffset = -1L
                var dataSize = 0
                var sampleRate = 0
                var channels = 0
                var bits = 0
                while (raf.filePointer + 8 <= raf.length()) {
                    val id = ByteArray(4)
                    raf.readFully(id)
                    val sizeBytes = ByteArray(4)
                    raf.readFully(sizeBytes)
                    val size = (sizeBytes[0].toInt() and 0xff) or
                        ((sizeBytes[1].toInt() and 0xff) shl 8) or
                        ((sizeBytes[2].toInt() and 0xff) shl 16) or
                        ((sizeBytes[3].toInt() and 0xff) shl 24)
                    val chunk = String(id)
                    when (chunk) {
                        "fmt " -> {
                            val fmt = ByteArray(size.coerceAtMost(16))
                            raf.readFully(fmt)
                            if (size > 16) raf.skipBytes(size - 16)
                            channels = (fmt[2].toInt() and 0xff) or ((fmt[3].toInt() and 0xff) shl 8)
                            sampleRate = (fmt[4].toInt() and 0xff) or
                                ((fmt[5].toInt() and 0xff) shl 8) or
                                ((fmt[6].toInt() and 0xff) shl 16) or
                                ((fmt[7].toInt() and 0xff) shl 24)
                            bits = (fmt[14].toInt() and 0xff) or ((fmt[15].toInt() and 0xff) shl 8)
                        }
                        "data" -> {
                            dataOffset = raf.filePointer
                            dataSize = size
                            break
                        }
                        else -> raf.skipBytes(size)
                    }
                }
                if (dataOffset < 0 || sampleRate != 48_000 || channels != 1 || bits != 16) {
                    Log.e(
                        TAG,
                        "WAV reject path=[${file.absolutePath}] exists=${file.isFile} " +
                            "readable=${file.canRead()} sr=$sampleRate ch=$channels bits=$bits " +
                            "(need 48k mono s16)"
                    )
                    return null
                }
                raf.seek(dataOffset)
                val raw = ByteArray(dataSize)
                raf.readFully(raw)
                val n = dataSize / 2
                val out = ShortArray(n)
                val bb = ByteBuffer.wrap(raw).order(ByteOrder.LITTLE_ENDIAN)
                bb.asShortBuffer().get(out)
                out
            }
        } catch (e: Exception) {
            Log.e(TAG, "WAV read failed path=[${file.absolutePath}]: ${e.message}")
            null
        }
    }
}
