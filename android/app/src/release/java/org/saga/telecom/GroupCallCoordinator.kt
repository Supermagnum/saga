package org.saga.telecom

import android.content.Context
import android.util.Log
import org.saga.call.MeshBackend
import org.saga.dial.DialTarget
import org.saga.iroh.IrohDialManager
import org.saga.iroh.IrohNodeId
import org.saga.iroh.PendingDialLookup

/**
 * Release stub: Iroh group calls are compiled out of release.
 *
 * G4/G5 failures mean Encrypted could appear without every key verified — never ship that.
 * Carrier conference (Phase 4 / MultiCallActions) is unchanged.
 * Debug implementation lives in src/debug.
 */
object GroupCallCoordinator {
    private const val TAG = "[Saga GroupCall]"

    data class AddRequest(
        val target: DialTarget.Iroh,
        val confirmedUnencrypted: Boolean = false,
        val introducerName: String? = null,
        val rosterFingerprintHex: String? = null,
        val groupId: String? = null
    )

    enum class AddOutcome {
        PLACED,
        NEEDS_UNENCRYPTED_CONFIRMATION,
        REFUSED_MAX_PARTICIPANTS,
        REFUSED_DUPLICATE,
        REFUSED_CELLULAR,
        REFUSED_NO_IROH_CALL,
        REFUSED_PHONE_ACCOUNT,
        FAILED
    }

    fun backendNow(): MeshBackend = MeshBackend.PAIRWISE_SESSIONS

    fun requestAdd(context: Context, request: AddRequest): AddOutcome {
        Log.w(TAG, "requestAdd refused — Iroh group calls not in release")
        return AddOutcome.FAILED
    }

    fun startOutgoingGroupLeg(
        context: Context,
        leg: PendingDialLookup.GroupLeg,
        peerId: IrohNodeId,
        connection: SagaIrohConnection,
        lookupKey: String
    ) {
        // Never form a group in release: fall back to a plain 1:1 dial.
        Log.w(TAG, "startOutgoingGroupLeg demoted to 1:1 — groups not in release")
        IrohDialManager.get(context).startOutgoing(peerId, connection, leg.legKey, lookupKey)
    }

    fun registerIncomingGroupLeg(
        context: Context,
        groupId: String,
        connection: SagaIrohConnection,
        introducerName: String?,
        rosterFingerprintHex: String?
    ) {
        Log.w(TAG, "registerIncomingGroupLeg refused — groups not in release group=[$groupId]")
        connection.onDisconnect()
    }

    fun onMeshPeerState(groupId: String, linkId: String, state: Int) {
        // no-op
    }

    fun syncConference(context: Context, groupId: String) {
        // no-op
    }

    fun onLegEnded(context: Context, telecomCallKey: String) {
        SagaIrohConnectionRegistry.remove(telecomCallKey)
    }

    fun hangupParticipant(context: Context, telecomCallKey: String): Boolean = false

    fun hangupParticipantByLookup(context: Context, lookupOrName: String): Boolean = false

    fun leaveGroup(context: Context, groupId: String) {
        // no-op
    }

    fun separateLeg(context: Context, groupId: String, leg: SagaIrohConnection) {
        // no-op
    }
}
