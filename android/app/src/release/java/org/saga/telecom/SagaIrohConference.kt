package org.saga.telecom

import android.content.Context
import android.telecom.Conference
import android.telecom.Connection
import android.telecom.PhoneAccountHandle

/**
 * Release stub: Iroh Conference / mesh group is debug-only.
 * Carrier conference uses Telecom's stock Conference path, not this class.
 */
class SagaIrohConference(
    @Suppress("UNUSED_PARAMETER") appContext: Context,
    val groupId: String,
    handle: PhoneAccountHandle
) : Conference(handle) {

    fun legs(): List<SagaIrohConnection> = emptyList()

    fun setLabel(label: String) {
        // no-op
    }

    fun resumeAll() {
        // no-op
    }

    override fun onHold() {
        // no-op
    }

    override fun onUnhold() {
        // no-op
    }

    override fun onDisconnect() {
        // no-op
    }

    override fun onSeparate(connection: Connection) {
        // no-op
    }

    fun dissolve() {
        // no-op
    }
}
