package org.saga.telecom

import android.telecom.Connection
import android.telecom.ConnectionRequest
import android.telecom.ConnectionService
import android.telecom.DisconnectCause
import android.telecom.PhoneAccountHandle
import android.os.Bundle
import android.util.Log
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
        val telecomCallKey = lookupKey
        Log.i(TAG, "Outgoing Iroh call to contact [$contactName] lookup=[$lookupKey]")
        val connection = SagaIrohConnection(this, contactName, telecomCallKey)
        connection.setInitializing()
        connection.setDialing()
        IrohDialManager.get(this).startOutgoing(peerId, connection, telecomCallKey, lookupKey)
        return connection
    }

    override fun onCreateIncomingConnection(
        connectionManagerPhoneAccount: PhoneAccountHandle?,
        request: ConnectionRequest
    ): Connection {
        val uri = request.address
        val peerId = SagaDialer.peerIdFromConnectionRequest(uri, request.extras) ?: IrohNodeId("unknown")
        val telecomCallKey = peerId.raw
        Log.i(TAG, "Incoming Iroh call from [${peerId.raw}]")
        val connection = SagaIrohConnection(this, peerId.raw, telecomCallKey)
        connection.setRinging()
        IrohDialManager.get(this).startOutgoing(peerId, connection, telecomCallKey)
        return connection
    }
}

class SagaIrohConnection(
    private val appContext: android.content.Context,
    private val peerAddress: String,
    val telecomCallKey: String
) : Connection() {
    init {
        connectionCapabilities = CAPABILITY_HOLD or CAPABILITY_SUPPORT_HOLD
    }

    fun onHandshakeState(state: org.saga.handshake.SagaHandshakeState) {
        val extras = Bundle()
        extras.putString(EXTRA_PEER_ADDRESS, peerAddress)
        extras.putString(EXTRA_HANDSHAKE_STATE, state.name)
        putExtras(extras)
    }

    override fun onAnswer() {
        setActive()
    }

    override fun onDisconnect() {
        IrohDialManager.get(appContext).endCall(telecomCallKey)
        setDisconnected(DisconnectCause(DisconnectCause.LOCAL))
        destroy()
    }

    override fun onAbort() {
        IrohDialManager.get(appContext).endCall(telecomCallKey)
        setDisconnected(DisconnectCause(DisconnectCause.CANCELED))
        destroy()
    }

    companion object {
        const val EXTRA_PEER_ADDRESS = "saga_peer_address"
        const val EXTRA_HANDSHAKE_STATE = "saga_handshake_state"
    }
}
