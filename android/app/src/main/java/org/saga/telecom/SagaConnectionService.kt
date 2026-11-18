package org.saga.telecom

import android.telecom.CallAudioState
import android.telecom.Connection
import android.telecom.ConnectionRequest
import android.telecom.ConnectionService
import android.telecom.DisconnectCause
import android.telecom.PhoneAccountHandle
import android.os.Bundle
import android.util.Log
import org.saga.audio.SagaCallMediaController
import org.saga.call.GroupCallFeatures
import org.saga.call.GroupCallState
import org.saga.iroh.IrohDialManager
import org.saga.iroh.IrohNodeId
import org.saga.iroh.PendingDialLookup
import org.saga.iroh.SagaDialer

/**
 * Managed (non-self-managed) ConnectionService for Iroh-originated Saga calls.
 */
class SagaConnectionService : ConnectionService() {
    companion object {
        private const val TAG = "[Saga ConnectionService]"

        /** Live service instance, needed by GroupCallCoordinator to call addConference. */
        @Volatile
        var current: SagaConnectionService? = null
            private set
    }

    override fun onCreate() {
        super.onCreate()
        current = this
    }

    override fun onDestroy() {
        if (current === this) current = null
        super.onDestroy()
    }

    override fun onCreateOutgoingConnection(
        connectionManagerPhoneAccount: PhoneAccountHandle?,
        request: ConnectionRequest
    ): Connection {
        val uri = request.address
        val peerId = SagaDialer.peerIdFromConnectionRequest(uri, request.extras)
        if (peerId == null) {
            Log.e(TAG, "Rejecting outgoing call — no peer in extras or URI [$uri]")
            return Connection.createFailedConnection(
                DisconnectCause(DisconnectCause.ERROR, "Invalid saga peer URI")
            )
        }
        val pending = PendingDialLookup.take(peerId.raw)
        val lookupKey = pending?.lookupKey ?: peerId.raw
        val contactName = pending?.contactName ?: lookupKey
        // Group leg: unique per-leg key (group id + lookup key), never just the lookup key.
        val groupLeg = pending?.groupLeg
        val telecomCallKey = groupLeg?.legKey ?: lookupKey
        Log.i(
            TAG,
            "Outgoing Iroh call to contact [$contactName] lookup=[$lookupKey] " +
                "group=[${groupLeg?.groupId}] key=[$telecomCallKey]"
        )
        val connection = SagaIrohConnection(
            this,
            contactName,
            telecomCallKey,
            incoming = false,
            lookupKey = lookupKey,
            endpointId = peerId.raw,
            groupId = groupLeg?.groupId
        )
        // Stay INITIALIZING during discovery/connect so Telecom does not play ringback
        // before the peer is actually reached (InCall shows "Connecting…").
        connection.setInitializing()
        PendingDialLookup.markIrohDialConsumed()
        SagaIrohConnectionRegistry.put(connection)
        if (groupLeg != null && GroupCallFeatures.ENABLED) {
            // Dual path: native mesh link, or one 1:1 native session under the group.
            GroupCallCoordinator.startOutgoingGroupLeg(
                this, groupLeg, peerId, connection, lookupKey
            )
        } else {
            // Release: never form an Iroh group (GroupCallFeatures.ENABLED == false).
            IrohDialManager.get(this).startOutgoing(peerId, connection, telecomCallKey, lookupKey)
        }
        return connection
    }

    override fun onCreateOutgoingConnectionFailed(
        connectionManagerPhoneAccount: PhoneAccountHandle?,
        request: ConnectionRequest
    ) {
        val uri = request.address
        val peerId = SagaDialer.peerIdFromConnectionRequest(uri, request.extras)
        Log.e(
            TAG,
            "IROH_ABORT_OUTGOING_FAILED uri=[$uri] peer=[${peerId?.raw}] — Telecom rejected Saga account"
        )
        peerId?.raw?.let { PendingDialLookup.clear(it) }
        PendingDialLookup.markIrohDialConsumed()
    }

    override fun onCreateIncomingConnection(
        connectionManagerPhoneAccount: PhoneAccountHandle?,
        request: ConnectionRequest
    ): Connection {
        Log.i(TAG, "CHECKPOINT onCreateIncomingConnection entered address=[${request.address}]")
        val uri = request.address
        val extras = request.extras
        val peerId = SagaDialer.peerIdFromConnectionRequest(uri, extras)
            ?: IrohNodeId("unknown")
        val sessionId = SagaDialer.sessionIdFromConnectionRequest(extras)
        val lookupKey = SagaDialer.lookupKeyFromConnectionRequest(extras) ?: peerId.raw
        val contactName = SagaDialer.contactNameFromConnectionRequest(extras) ?: lookupKey
        // Inbound group link (set by the mesh once it lands): unique per-leg key.
        val groupId = SagaDialer.groupIdFromConnectionRequest(extras)
        val telecomCallKey = groupId?.let { GroupCallState.legKey(it, lookupKey) } ?: lookupKey

        Log.i(
            TAG,
            "CHECKPOINT building Connection caller=[$contactName] lookup=[$lookupKey] session=[$sessionId]"
        )
        val connection = SagaIrohConnection(
            this,
            contactName,
            telecomCallKey,
            incoming = true,
            lookupKey = lookupKey,
            endpointId = peerId.raw,
            groupId = groupId
        )
        SagaIrohConnectionRegistry.put(connection)
        if (groupId != null && GroupCallFeatures.ENABLED) {
            val rosterFingerprint = extras
                ?.getBundle(android.telecom.TelecomManager.EXTRA_INCOMING_CALL_EXTRAS)
                ?.getString(SagaDialer.EXTRA_ROSTER_FINGERPRINT)
            GroupCallCoordinator.registerIncomingGroupLeg(
                this,
                groupId,
                connection,
                SagaDialer.introducerNameFromConnectionRequest(extras),
                rosterFingerprint
            )
        }
        connection.setRinging()
        Log.i(
            TAG,
            "CHECKPOINT Connection STATE_RINGING state=[${connection.state}] caller=[$contactName]"
        )

        if (sessionId.isNullOrBlank()) {
            Log.e(TAG, "CHECKPOINT FAIL missing EXTRA_SESSION_ID on incoming request")
            return Connection.createFailedConnection(
                DisconnectCause(DisconnectCause.ERROR, "Missing inbound session id")
            )
        }

        IrohDialManager.get(this).startIncoming(
            peerId = peerId,
            rustSessionId = sessionId,
            connection = connection,
            telecomCallKey = telecomCallKey,
            lookupKey = lookupKey,
            contactName = contactName
        )
        Log.i(TAG, "CHECKPOINT startIncoming completed for [$contactName]")
        return connection
    }

    /**
     * Telecom merge request. Never merges carrier with Iroh; Iroh groups are built only through
     * "Add participant" (GroupCallCoordinator), so this is refused for every pairing.
     */
    override fun onConference(connection1: Connection, connection2: Connection) {
        val bothIroh = connection1 is SagaIrohConnection && connection2 is SagaIrohConnection
        Log.w(
            TAG,
            "REFUSE_IROH_CARRIER_MERGE onConference bothIroh=[$bothIroh] — " +
                "Iroh groups use Add participant; carrier+Iroh is never merged"
        )
    }

    override fun onCreateIncomingConnectionFailed(
        connectionManagerPhoneAccount: PhoneAccountHandle?,
        request: ConnectionRequest
    ) {
        Log.e(
            TAG,
            "CHECKPOINT FAIL onCreateIncomingConnectionFailed address=[${request.address}] " +
                "account=[$connectionManagerPhoneAccount] — Telecom rejected incoming Saga call"
        )
    }
}

class SagaIrohConnection(
    private val appContext: android.content.Context,
    private val peerAddress: String,
    val telecomCallKey: String,
    private val incoming: Boolean = false,
    /** Contact lookup key used for key resolution (defaults to the telecom key for 1:1). */
    val lookupKey: String = telecomCallKey,
    val endpointId: String = telecomCallKey,
    /** Non-null for legs created by "Add participant"; carried as Connection extras. */
    val groupId: String? = null
) : Connection() {
    private var inConference = false

    init {
        applyCapabilities()
        if (groupId != null) {
            // Unique per-leg key so InCall can map this Call to its mesh participant.
            putExtras(
                Bundle().apply {
                    putString(SagaDialer.EXTRA_TELECOM_CALL_KEY, telecomCallKey)
                    putString(SagaDialer.EXTRA_GROUP_ID, groupId)
                }
            )
        }
    }

    /** Display name shown for this leg. */
    val displayName: String get() = peerAddress

    /** SEPARATE / DISCONNECT_FROM_CONFERENCE only while this leg is a conference child. */
    fun setInConference(value: Boolean) {
        inConference = value
        applyCapabilities()
    }

    private fun applyCapabilities() {
        var caps = CAPABILITY_HOLD or CAPABILITY_SUPPORT_HOLD
        if (inConference) {
            caps = caps or CAPABILITY_SEPARATE_FROM_CONFERENCE or CAPABILITY_DISCONNECT_FROM_CONFERENCE
        }
        connectionCapabilities = caps
    }

    fun onHandshakeState(state: org.saga.handshake.SagaHandshakeState) {
        val extras = Bundle()
        extras.putString(EXTRA_PEER_ADDRESS, peerAddress)
        extras.putString(EXTRA_HANDSHAKE_STATE, state.name)
        putExtras(extras)
    }

    override fun onAnswer() {
        Log.i(TAG, "CHECKPOINT onAnswer incoming=[$incoming] key=[$telecomCallKey]")
        if (incoming) {
            IrohDialManager.get(appContext).onIncomingAnswered(telecomCallKey)
        } else {
            setActive()
        }
    }

    /**
     * Closes THIS link only. In a group call this never tears down the other links: there is
     * no host, so the starter leaving (or any child being disconnected) leaves the rest running.
     */
    override fun onDisconnect() {
        IrohDialManager.get(appContext).endCall(telecomCallKey)
        setDisconnected(DisconnectCause(DisconnectCause.LOCAL))
        destroy()
        GroupCallCoordinator.onLegEnded(appContext, telecomCallKey)
    }

    override fun onAbort() {
        IrohDialManager.get(appContext).endCall(telecomCallKey)
        setDisconnected(DisconnectCause(DisconnectCause.CANCELED))
        destroy()
        GroupCallCoordinator.onLegEnded(appContext, telecomCallKey)
    }

    override fun onHold() {
        Log.i(TAG, "onHold key=[$telecomCallKey]")
        val sessionId = IrohDialManager.get(appContext).sessionIdForCall(telecomCallKey)
        SagaCallMediaController.setHeld(telecomCallKey, sessionId, true)
        setOnHold()
    }

    override fun onUnhold() {
        Log.i(TAG, "onUnhold key=[$telecomCallKey]")
        val sessionId = IrohDialManager.get(appContext).sessionIdForCall(telecomCallKey)
        SagaCallMediaController.setHeld(telecomCallKey, sessionId, false)
        setActive()
    }

    override fun onCallAudioStateChanged(state: CallAudioState) {
        SagaCallMediaController.logAudioRoute(state)
    }

    companion object {
        private const val TAG = "[Saga Iroh Connection]"
        const val EXTRA_PEER_ADDRESS = "saga_peer_address"
        const val EXTRA_HANDSHAKE_STATE = "saga_handshake_state"
    }
}

/**
 * Live Iroh Connections / Conferences by telecom call key / group id, so the group
 * coordinator can add legs to a Conference and close single links.
 */
object SagaIrohConnectionRegistry {
    private val connections = java.util.concurrent.ConcurrentHashMap<String, SagaIrohConnection>()
    private val conferences = java.util.concurrent.ConcurrentHashMap<String, SagaIrohConference>()

    fun put(connection: SagaIrohConnection) {
        connections[connection.telecomCallKey] = connection
    }

    fun get(telecomCallKey: String): SagaIrohConnection? = connections[telecomCallKey]

    fun remove(telecomCallKey: String) {
        connections.remove(telecomCallKey)
    }

    fun all(): List<SagaIrohConnection> = connections.values.toList()

    /**
     * The 1:1 Iroh leg that can be promoted to a group (active or held).
     * Prefers a single ACTIVE connection; if ghost dialing/holding legs exist alongside one
     * ACTIVE, still returns that ACTIVE leg (roster intro must not be dropped).
     */
    fun soleLiveLeg(): SagaIrohConnection? {
        val active = connections.values.filter { it.state == Connection.STATE_ACTIVE }
        when (active.size) {
            1 -> return active[0]
            0 -> Unit
            else -> return active.firstOrNull()
        }
        val live = connections.values.filter {
            it.state == Connection.STATE_ACTIVE ||
                it.state == Connection.STATE_HOLDING ||
                it.state == Connection.STATE_DIALING ||
                it.state == Connection.STATE_RINGING
        }
        return live.singleOrNull() ?: live.firstOrNull()
            ?: connections.values.firstOrNull()
    }

    fun conference(groupId: String): SagaIrohConference? = conferences[groupId]

    fun putConference(conference: SagaIrohConference) {
        conferences[conference.groupId] = conference
    }

    fun removeConference(groupId: String) {
        conferences.remove(groupId)
    }
}
