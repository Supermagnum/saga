package org.saga.telecom

import android.content.Context
import android.os.Bundle
import android.telecom.Conference
import android.telecom.Connection
import android.telecom.DisconnectCause
import android.telecom.PhoneAccountHandle
import android.util.Log
import org.saga.iroh.SagaDialer

/**
 * Telecom Conference for an Iroh mesh group.
 *
 * Children are only [SagaIrohConnection] legs, so a carrier call can never join an Iroh
 * conference (the type is enforced in [GroupCallCoordinator] before addConnection). There is
 * no host: [onDisconnect] means "the local user leaves", it closes the local links and
 * never signals the other participants, who keep their own mesh links.
 */
class SagaIrohConference(
    private val appContext: Context,
    val groupId: String,
    handle: PhoneAccountHandle
) : Conference(handle) {

    init {
        connectionCapabilities = Connection.CAPABILITY_MANAGE_CONFERENCE or
            Connection.CAPABILITY_HOLD or
            Connection.CAPABILITY_SUPPORT_HOLD
        // Key the Conference Call like a leg so InCall can find the group parent.
        putExtras(
            Bundle().apply {
                putString(SagaDialer.EXTRA_TELECOM_CALL_KEY, groupId)
                putString(SagaDialer.EXTRA_GROUP_ID, groupId)
            }
        )
    }

    fun legs(): List<SagaIrohConnection> = connections.filterIsInstance<SagaIrohConnection>()

    /** Participant names shown as the Conference title in InCall. */
    fun setLabel(label: String) {
        putExtras(Bundle().apply { putString(SagaDialer.EXTRA_GROUP_LABEL, label) })
    }

    /** Unhold every held leg and mark the conference active (no connect cue on unhold). */
    fun resumeAll() {
        legs().forEach { leg ->
            if (leg.state == Connection.STATE_HOLDING) leg.onUnhold()
        }
        setActive()
    }

    override fun onHold() {
        Log.i(TAG, "onHold group=[$groupId]")
        legs().forEach { it.onHold() }
        setOnHold()
    }

    override fun onUnhold() {
        Log.i(TAG, "onUnhold group=[$groupId]")
        resumeAll()
    }

    /** Local user leaves: close our links only. Others are never told to end. */
    override fun onDisconnect() {
        Log.i(TAG, "onDisconnect (local leave) group=[$groupId]")
        GroupCallCoordinator.leaveGroup(appContext, groupId)
    }

    /**
     * SEPARATE_FROM_CONFERENCE: the chosen leg becomes a standalone 1:1; the remaining legs are
     * held. The link itself stays up, so this is a private side conversation, not a hang-up.
     */
    override fun onSeparate(connection: Connection) {
        val leg = connection as? SagaIrohConnection ?: return
        Log.i(TAG, "onSeparate group=[$groupId] leg=[${leg.telecomCallKey}]")
        GroupCallCoordinator.separateLeg(appContext, groupId, leg)
    }

    override fun onMerge(connection: Connection) {
        // Only Add participant grows an Iroh conference; Telecom merges are refused.
        Log.w(TAG, "onMerge refused group=[$groupId] — use Add participant")
    }

    override fun onMerge() {
        Log.w(TAG, "onMerge refused group=[$groupId] — use Add participant")
    }

    /** Detach all children (they continue as standalone 1:1 Iroh calls) and end the Conference. */
    fun dissolve() {
        connections.toList().forEach { child ->
            removeConnection(child)
            (child as? SagaIrohConnection)?.setInConference(false)
        }
        setDisconnected(DisconnectCause(DisconnectCause.OTHER))
        destroy()
    }

    companion object {
        private const val TAG = "[Saga Iroh Conference]"
    }
}
