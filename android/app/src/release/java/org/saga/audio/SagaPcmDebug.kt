package org.saga.audio

import java.nio.ByteBuffer

/**
 * Release stub: PCM inject/dump/markers must never ship in release builds.
 * Debug implementation lives in src/debug.
 */
object SagaPcmDebug {
    @JvmStatic
    fun setInjectWav(path: String?) {
        // no-op
    }

    @JvmStatic
    fun setDumpPath(path: String?) {
        // no-op
    }

    @JvmStatic
    fun injectActive(): Boolean = false

    @JvmStatic
    fun armClickMarker() {
        // no-op
    }

    @JvmStatic
    fun armExpectMarker() {
        // no-op
    }

    fun fillInjectFrame(buf: ByteBuffer): Boolean = false

    fun dumpPlaybackFrame(buf: ByteBuffer, len: Int) {
        // no-op
    }

    @JvmStatic
    fun finishDump() {
        // no-op
    }
}
