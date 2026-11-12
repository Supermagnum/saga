package org.saga.audio

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * Host/JVM stand-in for the JNI capture path: direct ByteBuffer must use
 * [ByteOrder.nativeOrder] and pack i16 the same way Rust `i16::from_le_bytes`
 * unpacks on little-endian devices (emulator + production SoCs).
 */
class JniPcmByteOrderTest {
    @Test
    fun nativeOrderDirectBuffer_roundTripsRampBitExact() {
        val samples = ShortArray(SagaVoiceAudioEngine.FRAME_SAMPLES) { i ->
            (i * 3 - 1000).toShort()
        }
        val buf = ByteBuffer
            .allocateDirect(SagaVoiceAudioEngine.FRAME_BYTES)
            .order(ByteOrder.nativeOrder())
        buf.asShortBuffer().put(samples)
        buf.position(0)
        buf.limit(SagaVoiceAudioEngine.FRAME_BYTES)

        val bytes = ByteArray(SagaVoiceAudioEngine.FRAME_BYTES)
        buf.duplicate().order(ByteOrder.LITTLE_ENDIAN).get(bytes)

        val back = ShortArray(SagaVoiceAudioEngine.FRAME_SAMPLES)
        for (i in back.indices) {
            val lo = bytes[i * 2].toInt() and 0xff
            val hi = bytes[i * 2 + 1].toInt() and 0xff
            // Matches Rust i16::from_le_bytes([lo, hi]).
            back[i] = ((hi shl 8) or lo).toShort()
        }
        assertArrayEquals(samples, back)
        assertEquals(ByteOrder.nativeOrder(), buf.order())
        assertEquals(960, SagaVoiceAudioEngine.FRAME_SAMPLES)
        assertEquals(48_000, SagaVoiceAudioEngine.SAMPLE_RATE)
    }

    @Test
    fun frameDuration_is20msAt48k() {
        // Capture loop paces inject to this period; regressions here re-break B→A.
        val frameMs =
            1000L * SagaVoiceAudioEngine.FRAME_SAMPLES / SagaVoiceAudioEngine.SAMPLE_RATE
        assertEquals(20L, frameMs)
    }
}
