package org.saga.iroh

import android.content.Context
import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.saga.call.CallOrigin
import org.saga.call.CallSecuritySnapshot
import org.saga.call.CallSecurityStateResolver
import org.saga.call.GroupCallRegistry
import org.saga.call.KeyCheck
import org.saga.call.PadlockDisplayState
import org.saga.call.RosterKeyCheck
import org.saga.call.SagaCallEventHub
import org.saga.call.SagaCallRegistry
import org.saga.handshake.EncryptionEstablishedStore
import org.saga.handshake.SagaHandshakeCoordinator
import org.saga.handshake.SagaHandshakeState
import org.saga.handshake.SagaTestFlags
import org.saga.audio.SagaCallMediaController
import org.saga.security.DowngradeEventLog
import org.saga.security.midcall.MidCallRehandshakeOutcome
import org.saga.security.midcall.SagaMidCallSecurityAudioWarning
import org.saga.telecom.GroupCallCoordinator
import org.saga.telecom.SagaIrohConnection
import android.telecom.Connection
import android.telecom.DisconnectCause
import java.util.concurrent.ConcurrentHashMap

class IrohDialManager private constructor(context: Context) {
    private val appContext = context.applicationContext
    private val encryptionStore = EncryptionEstablishedStore(appContext)
    private val handshakeCoordinator = SagaHandshakeCoordinator(appContext, encryptionStore)
    private val sessions = ConcurrentHashMap<String, IrohCallSession>()
    private val lookupKeys = ConcurrentHashMap<String, String>()
    private val incomingConnections = ConcurrentHashMap<String, SagaIrohConnection>()
    private val pendingIncomingActivation = ConcurrentHashMap<String, SagaHandshakeState>()
    private val scope = CoroutineScope(Dispatchers.Default + SupervisorJob())

    fun startIncoming(
        peerId: IrohNodeId,
        rustSessionId: String,
        connection: SagaIrohConnection,
        telecomCallKey: String,
        lookupKey: String,
        contactName: String
    ) {
        val session = InboundIrohCallSession(peerId, rustSessionId)
        sessions[telecomCallKey] = session
        lookupKeys[telecomCallKey] = lookupKey
        incomingConnections[telecomCallKey] = connection
        Log.i(
            TAG,
            "startIncoming contact=[$contactName] lookup=[$lookupKey] session=[$rustSessionId]"
        )

        session.connect(
            onConnected = {
                handshakeCoordinator.start(lookupKey, session.sessionId) { state ->
                    onIncomingHandshakeSettled(lookupKey, telecomCallKey, state, connection)
                }
            },
            onFailed = {
                onIncomingHandshakeSettled(
                    lookupKey,
                    telecomCallKey,
                    handshakeCoordinator.resolveFailure(lookupKey),
                    connection
                )
            }
        )
    }

    fun onIncomingAnswered(telecomCallKeyOrLookup: String) {
        val resolved = resolveIncomingCallKey(telecomCallKeyOrLookup)
        if (resolved == null) {
            Log.w(TAG, "onIncomingAnswered: no connection for [$telecomCallKeyOrLookup]")
            return
        }
        val (telecomCallKey, connection) = resolved
        connection.setActive()
        Log.i(TAG, "CHECKPOINT incoming Connection setActive key=[$telecomCallKey]")
        val sessionId = sessions[telecomCallKey]?.sessionId
        // Flush pre-answer JB + signal caller before AudioTrack/mic attach.
        SagaCallMediaController.onLocalAnswered(appContext, telecomCallKey, sessionId)
        pendingIncomingActivation.remove(telecomCallKey)?.let { state ->
            publishHandshakeState(
                lookupKeys[telecomCallKey] ?: telecomCallKey,
                telecomCallKey,
                state,
                connection,
                activateConnection = false
            )
        }
    }

    /** Resolve an answer intent lookup / call key to the live incoming Connection. */
    private fun resolveIncomingCallKey(
        telecomCallKeyOrLookup: String
    ): Pair<String, SagaIrohConnection>? {
        incomingConnections[telecomCallKeyOrLookup]?.let { return telecomCallKeyOrLookup to it }
        val viaFind = findTelecomCallKey(telecomCallKeyOrLookup)
        if (viaFind != null) {
            incomingConnections[viaFind]?.let { return viaFind to it }
        }
        val digits = telecomCallKeyOrLookup.filter { it.isDigit() }
        if (digits.isNotEmpty()) {
            incomingConnections.entries.firstOrNull { (key, conn) ->
                key.filter { it.isDigit() } == digits ||
                    conn.lookupKey.filter { it.isDigit() } == digits ||
                    (lookupKeys[key]?.filter { it.isDigit() } == digits)
            }?.let { return it.key to it.value }
        }
        return incomingConnections.entries.singleOrNull()?.let { it.key to it.value }
    }

    private fun onIncomingHandshakeSettled(
        lookupKey: String,
        telecomCallKey: String,
        state: SagaHandshakeState,
        connection: SagaIrohConnection
    ) {
        if (state == SagaHandshakeState.Downgraded) {
            publishHandshakeState(lookupKey, telecomCallKey, state, connection, activateConnection = true)
            incomingConnections.remove(telecomCallKey)
            return
        }
        if (connection.state == Connection.STATE_RINGING) {
            pendingIncomingActivation[telecomCallKey] = state
            connection.onHandshakeState(state)
            // Open RoQ ReceiveFlow + JB now so caller RTP is not lost before answer.
            // AudioRecord/AudioTrack attach in publishHandshakeState on answer.
            if (state == SagaHandshakeState.Encrypted ||
                state == SagaHandshakeState.NeverEncrypted
            ) {
                SagaCallMediaController.startNativeReceive(
                    telecomCallId = telecomCallKey,
                    rustSessionId = sessions[telecomCallKey]?.sessionId,
                    state = state,
                    userContinuedPast5a = false
                )
                Log.i(
                    TAG,
                    "Incoming handshake settled to [$state] while RINGING — RX media armed, waiting for answer"
                )
            } else {
                Log.i(
                    TAG,
                    "Incoming handshake settled to [$state] while RINGING — waiting for answer"
                )
            }
            return
        }
        publishHandshakeState(lookupKey, telecomCallKey, state, connection, activateConnection = true)
        incomingConnections.remove(telecomCallKey)
    }

    fun startOutgoing(
        peerId: IrohNodeId,
        connection: SagaIrohConnection,
        telecomCallKey: String,
        lookupKey: String = peerId.raw
    ) {
        val session = IrohSessionFactory.create(peerId)
        sessions[telecomCallKey] = session
        lookupKeys[telecomCallKey] = lookupKey
        publishConnecting(lookupKey, telecomCallKey)

        session.connect(
            onConnected = {
                publishSecuring(lookupKey, telecomCallKey)
                handshakeCoordinator.start(lookupKey, session.sessionId) { state ->
                    onHandshakeSettled(lookupKey, telecomCallKey, state, connection)
                }
            },
            onFailed = { reason ->
                // Connect-phase failure is reachability, not §5a downgrade / cellular fallback.
                failNotReachable(lookupKey, telecomCallKey, connection, reason)
            }
        )
    }

    /**
     * Native-mesh path for a group leg: no per-peer 1:1 native session is opened. The link id
     * stands in for the session id; link state arrives via [onMeshLinkState].
     *
     * TODO(mesh-jni): media for mesh links still goes through the per-session
     * SagaCallMediaController path keyed by link id until native mesh media is wired.
     */
    fun registerMeshLink(
        groupId: String,
        peerId: IrohNodeId,
        linkId: String,
        telecomCallKey: String,
        lookupKey: String
    ) {
        sessions[telecomCallKey] = MeshLinkSession(groupId, peerId, linkId)
        lookupKeys[telecomCallKey] = lookupKey
        publishConnecting(lookupKey, telecomCallKey)
    }

    /** Translate a native mesh link state into the same handshake outcomes as the 1:1 path. */
    fun onMeshLinkState(
        telecomCallKey: String,
        meshState: Int,
        connection: SagaIrohConnection?
    ) {
        val lookupKey = lookupKeys[telecomCallKey] ?: return
        when (meshState) {
            IrohNativeBridge.MESH_PEER_PENDING -> publishSecuring(lookupKey, telecomCallKey)
            IrohNativeBridge.MESH_PEER_ENCRYPTED -> {
                // Prefer GroupCall keyCheck (roster or contact) set by onMeshPeerState.
                val keyCheck = GroupCallRegistry.findLeg(telecomCallKey)?.second?.keyCheck
                val state = when {
                    keyCheck?.confirmed == true -> {
                        encryptionStore.markEncryptionEstablished(lookupKey)
                        SagaHandshakeState.Encrypted
                    }
                    keyCheck?.rejected == true -> {
                        if (keyCheck == KeyCheck.KEY_CHANGED ||
                            encryptionStore.hasEncryptionHistory(lookupKey)
                        ) {
                            SagaHandshakeState.Downgraded
                        } else {
                            SagaHandshakeState.NeverEncrypted
                        }
                    }
                    ContactKeyStore.hasResolvableKey(appContext, lookupKey) -> {
                        encryptionStore.markEncryptionEstablished(lookupKey)
                        SagaHandshakeState.Encrypted
                    }
                    else -> SagaHandshakeState.NeverEncrypted
                }
                if (connection != null) {
                    onHandshakeSettled(lookupKey, telecomCallKey, state, connection)
                } else {
                    publishHandshakeState(lookupKey, telecomCallKey, state, null, false)
                }
            }
            IrohNativeBridge.MESH_PEER_FAILED -> {
                val state = handshakeCoordinator.resolveFailure(lookupKey)
                if (connection != null) {
                    onHandshakeSettled(lookupKey, telecomCallKey, state, connection)
                } else {
                    publishHandshakeState(lookupKey, telecomCallKey, state, null, false)
                }
            }
        }
    }

    /**
     * Outgoing peer never answered the QUIC dial (discovery miss, budget exhausted, etc.).
     * Disconnect cleanly with InCall status "Not reachable" — never open the §5a padlock path.
     */
    private fun failNotReachable(
        lookupKey: String,
        telecomCallKey: String,
        connection: SagaIrohConnection,
        reason: String
    ) {
        Log.w(
            TAG,
            "NOT_REACHABLE lookup=[$lookupKey] key=[$telecomCallKey] reason=[$reason]"
        )
        val status = appContext.getString(org.saga.R.string.call_not_reachable)
        val snapshot = CallSecuritySnapshot(
            padlockState = PadlockDisplayState.KEY_NOT_FOUND,
            statusText = status,
            playSecureCue = false,
            playUnsecureCue = false,
            showDowngradeModal = false
        )
        SagaCallRegistry.updateSnapshot(telecomCallKey, snapshot)
        SagaCallEventHub.emit(
            SagaCallEventHub.SecurityUpdate(
                telecomCallKey,
                snapshot,
                playConnectCue = false
            )
        )
        sessions.remove(telecomCallKey)?.disconnect()
        lookupKeys.remove(telecomCallKey)
        try {
            connection.setDisconnected(
                DisconnectCause(DisconnectCause.ERROR, status)
            )
            connection.destroy()
        } catch (e: Exception) {
            Log.e(TAG, "failNotReachable disconnect failed: ${e.message}")
        }
        // Group leg: drop only this link; the rest of the conference keeps running.
        GroupCallCoordinator.onLegEnded(appContext, telecomCallKey)
    }

    fun endCall(telecomCallKey: String) {
        val session = sessions.remove(telecomCallKey)
        val sessionId = session?.sessionId
        SagaCallMediaController.stop(telecomCallKey, sessionId)
        session?.disconnect()
        lookupKeys.remove(telecomCallKey)
        incomingConnections.remove(telecomCallKey)
        pendingIncomingActivation.remove(telecomCallKey)
    }

    fun continueUnencryptedMedia(telecomCallKey: String, lookupKey: String) {
        val sessionId = sessions[telecomCallKey]?.sessionId
        SagaCallMediaController.onSecurityResolved(
            context = appContext,
            telecomCallId = telecomCallKey,
            rustSessionId = sessionId,
            state = SagaHandshakeState.Downgraded,
            userContinuedPast5a = true
        )
        Log.i(TAG, "§5a continue → plain RTP media call=[$telecomCallKey] lookup=[$lookupKey]")
    }

    fun sessionIdForCall(telecomCallKey: String): String? = sessions[telecomCallKey]?.sessionId

    fun lookupKeyForCall(telecomCallKey: String): String? = lookupKeys[telecomCallKey]

    /**
     * Sole live dial session key (1:1 promotion / roster-intro fallback when Telecom
     * Connection registry is empty). Prefers a single session; otherwise first entry.
     */
    fun soleLiveCallKey(): String? =
        when (sessions.size) {
            0 -> null
            1 -> sessions.keys.firstOrNull()
            else -> sessions.keys.firstOrNull()
        }

    /** Debug / E2E: number of live dial sessions (Connection registry may lag). */
    fun sessionCount(): Int = sessions.size

    fun findTelecomCallKey(lookupKey: String): String? {
        if (sessions.containsKey(lookupKey)) return lookupKey
        lookupKeys.entries.firstOrNull { it.value == lookupKey }?.key?.let { return it }
        val digits = lookupKey.filter { it.isDigit() }
        if (digits.isNotEmpty()) {
            lookupKeys.entries.firstOrNull { it.value.filter { ch -> ch.isDigit() } == digits }
                ?.key
                ?.let { return it }
            sessions.keys.firstOrNull { it.filter { ch -> ch.isDigit() } == digits }?.let { return it }
        }
        IrohNodeId.parse(lookupKey)?.let { id ->
            val peerUri = SagaCallUri.forPeer(id).toString()
            if (sessions.containsKey(peerUri)) return peerUri
        }
        return sessions.keys.firstOrNull { it.contains(lookupKey) }
            ?: sessions.keys.singleOrNull()
    }

    fun handleMidCallRehandshake(
        telecomCallKey: String?,
        lookupKey: String,
        outcome: MidCallRehandshakeOutcome
    ) {
        scope.launch {
            val callKey = telecomCallKey?.takeIf { sessions.containsKey(it) }
                ?: findTelecomCallKey(lookupKey)
            if (callKey == null) {
                Log.w(TAG, "Mid-call rehandshake: no active call for lookup [$lookupKey]")
                return@launch
            }
            val sessionId = sessions[callKey]?.sessionId
            if (sessionId == null) {
                Log.w(TAG, "Mid-call rehandshake: no session for call [$callKey]")
                return@launch
            }
            val effectiveLookup = lookupKeys[callKey] ?: lookupKey
            Log.i(TAG, "Mid-call rehandshake outcome=[$outcome] call=[$callKey] lookup=[$effectiveLookup]")

            when (outcome) {
                MidCallRehandshakeOutcome.SUCCESS -> {
                    IrohNativeBridge.setForceHandshakeFail(false)
                    delay(300)
                    val poll = IrohNativeBridge.pollHandshake(sessionId)
                    if (poll == IrohNativeBridge.HANDSHAKE_ENCRYPTED) {
                        val us = IrohNativeBridge.rekeyMedia(sessionId)
                        Log.i(
                            TAG,
                            "Mid-call re-handshake succeeded — rekey_swap_us=$us (budget <100000)"
                        )
                    } else {
                        Log.w(TAG, "Mid-call re-handshake expected success but poll=[$poll]")
                    }
                }
                MidCallRehandshakeOutcome.CRYPTO_FAILURE -> {
                    IrohNativeBridge.setForceHandshakeFail(true)
                    delay(300)
                    IrohNativeBridge.pollHandshake(sessionId)
                    applyDowngradePath(effectiveLookup, callKey, playMidCallWarning = true, reason = "crypto_failure")
                }
                MidCallRehandshakeOutcome.TIMEOUT -> {
                    Log.i(TAG, "Mid-call re-handshake simulating 4s timeout")
                    delay(4000)
                    applyDowngradePath(effectiveLookup, callKey, playMidCallWarning = true, reason = "timeout")
                }
            }
            IrohNativeBridge.setForceHandshakeFail(SagaTestFlags.isForceHandshakeFail(appContext))
            IrohNativeBridge.setForceBindingPerturb(SagaTestFlags.isForceBindingPerturb(appContext))
        }
    }

    private fun applyDowngradePath(
        lookupKey: String,
        telecomCallKey: String,
        playMidCallWarning: Boolean,
        reason: String
    ) {
        val state = handshakeCoordinator.resolveFailure(lookupKey)
        if (state != SagaHandshakeState.Downgraded) {
            Log.w(TAG, "Mid-call $reason did not downgrade (state=$state lookup=$lookupKey)")
            return
        }
        publishHandshakeState(lookupKey, telecomCallKey, state, connection = null, activateConnection = false)
        if (playMidCallWarning) {
            SagaMidCallSecurityAudioWarning.playWarning(appContext)
        }
    }

    private fun onHandshakeSettled(
        lookupKey: String,
        telecomCallKey: String,
        state: SagaHandshakeState,
        connection: SagaIrohConnection
    ) {
        val effective = verifySettledKey(lookupKey, telecomCallKey, state)
        publishHandshakeState(lookupKey, telecomCallKey, effective, connection, activateConnection = true)
    }

    /**
     * After a cryptographic Encrypted settle, compare the authenticated peer fingerprint to
     * the local contact key and/or roster voucher. Mismatch must never show Encrypted (G4/G5).
     */
    private fun verifySettledKey(
        lookupKey: String,
        telecomCallKey: String,
        state: SagaHandshakeState
    ): SagaHandshakeState {
        if (state != SagaHandshakeState.Encrypted) return state
        val sessionId = sessions[telecomCallKey]?.sessionId ?: return state
        val groupLeg = GroupCallRegistry.findLeg(telecomCallKey)
        if (groupLeg != null) {
            val check = GroupCallCoordinator.onPairwiseLegEncrypted(
                appContext, telecomCallKey, sessionId
            )
            if (check.rejected) {
                Log.e(TAG, "KEY_MISMATCH group leg lookup=[$lookupKey] check=[$check]")
                val failed = if (check == KeyCheck.KEY_CHANGED ||
                    encryptionStore.hasEncryptionHistory(lookupKey)
                ) {
                    SagaHandshakeState.Downgraded
                } else {
                    SagaHandshakeState.NeverEncrypted
                }
                // Close after the failed settle is published (caller continues).
                scope.launch {
                    delay(50)
                    GroupCallCoordinator.hangupParticipant(appContext, telecomCallKey)
                }
                return failed
            }
            if (check.confirmed) return SagaHandshakeState.Encrypted
            // No contact key and no matching roster: never show Encrypted (G5b / unencrypted admit).
            return SagaHandshakeState.NeverEncrypted
        }
        // 1:1: require a stored contact key that matches the link fingerprint.
        val local = ContactKeyStore.fingerprintHexForPhone(appContext, lookupKey)
            ?: return SagaHandshakeState.NeverEncrypted
        val linkFp = IrohNativeBridge.sessionPeerFingerprint(sessionId)
        val check = RosterKeyCheck.evaluate(linkFp, local, null)
        if (check.rejected) {
            Log.e(TAG, "KEY_MISMATCH 1:1 lookup=[$lookupKey] check=[$check] — refusing Encrypted")
            return if (check == KeyCheck.KEY_CHANGED ||
                encryptionStore.hasEncryptionHistory(lookupKey)
            ) {
                SagaHandshakeState.Downgraded
            } else {
                SagaHandshakeState.NeverEncrypted
            }
        }
        if (!check.confirmed) return SagaHandshakeState.NeverEncrypted
        return SagaHandshakeState.Encrypted
    }

    private fun publishHandshakeState(
        lookupKey: String,
        telecomCallKey: String,
        state: SagaHandshakeState,
        connection: SagaIrohConnection?,
        activateConnection: Boolean
    ) {
        if (state == SagaHandshakeState.Downgraded) {
            DowngradeEventLog.record(appContext, lookupKey)
        }
        connection?.onHandshakeState(state)
        val snapshot = snapshotForHandshake(lookupKey, state)
        // Group leg that settled without encryption: media stays off until the adder confirms.
        val groupGatesPlainMedia = GroupCallRegistry.gateUnencryptedMedia(telecomCallKey, state)
        SagaCallRegistry.updateSnapshot(telecomCallKey, snapshot)
        // Play connect cue for Encrypted / NeverEncrypted / Downgraded (§5a).
        // Downgrade modal still shows; Continue uses playOnce so the wav is not doubled.
        SagaCallEventHub.emit(
            SagaCallEventHub.SecurityUpdate(
                telecomCallKey,
                snapshot,
                playConnectCue = state != SagaHandshakeState.Securing
            )
        )
        if (state != SagaHandshakeState.Securing) {
            Log.i(TAG, "Handshake settled to [$state] for [$lookupKey]")
        }
        if (activateConnection && connection != null && state != SagaHandshakeState.Securing) {
            // Outgoing calls stay in DIALING until setActive(); skipping Downgraded used to
            // leave ringback playing forever on the caller. Incoming downgrade while still
            // RINGING waits for explicit answer (onIncomingAnswered).
            val deferUntilAnswer =
                state == SagaHandshakeState.Downgraded &&
                    connection.state == Connection.STATE_RINGING
            if (!deferUntilAnswer) {
                connection.setActive()
                Log.i(
                    TAG,
                    "CHECKPOINT Connection setActive key=[$telecomCallKey] handshake=[$state] telecomState=[${connection.state}]"
                )
            }
        }
        // Media only after security resolves. Downgraded waits for §5a Continue.
        if (!groupGatesPlainMedia &&
            (state == SagaHandshakeState.Encrypted || state == SagaHandshakeState.NeverEncrypted)
        ) {
            val sessionId = sessions[telecomCallKey]?.sessionId
            SagaCallMediaController.onSecurityResolved(
                context = appContext,
                telecomCallId = telecomCallKey,
                rustSessionId = sessionId,
                state = state,
                userContinuedPast5a = false
            )
        }
    }

    /** Discovery / QUIC dial in progress — not yet a security handshake. */
    private fun publishConnecting(lookupKey: String, telecomCallKey: String) {
        val snapshot = CallSecuritySnapshot(
            padlockState = PadlockDisplayState.SECURING,
            statusText = appContext.getString(org.saga.R.string.call_connecting),
            playSecureCue = false,
            playUnsecureCue = false,
            showDowngradeModal = false
        )
        SagaCallRegistry.updateSnapshot(telecomCallKey, snapshot)
        SagaCallEventHub.emit(
            SagaCallEventHub.SecurityUpdate(telecomCallKey, snapshot, playConnectCue = false)
        )
    }

    private fun publishSecuring(lookupKey: String, telecomCallKey: String) {
        val hasKey = ContactKeyStore.hasResolvableKey(appContext, lookupKey)
        val securing = CallSecurityStateResolver.resolve(
            origin = CallOrigin.IROH,
            handshakeSecuring = true,
            contactKeyResolvable = hasKey
        )
        SagaCallRegistry.updateSnapshot(telecomCallKey, securing)
        SagaCallEventHub.emit(
            SagaCallEventHub.SecurityUpdate(telecomCallKey, securing, playConnectCue = false)
        )
    }

    private fun snapshotForHandshake(lookupKey: String, state: SagaHandshakeState): CallSecuritySnapshot {
        val hasKey = ContactKeyStore.hasResolvableKey(appContext, lookupKey)
        val hasHistory = encryptionStore.hasEncryptionHistory(lookupKey)
        return when (state) {
            SagaHandshakeState.Encrypted -> CallSecurityStateResolver.resolve(
                CallOrigin.IROH, handshakeEncrypted = true, contactKeyResolvable = hasKey
            )
            SagaHandshakeState.Downgraded -> CallSecurityStateResolver.resolve(
                CallOrigin.IROH, handshakeFailed = true, hasEncryptionHistory = true, contactKeyResolvable = hasKey
            )
            SagaHandshakeState.NeverEncrypted -> CallSecurityStateResolver.resolve(
                CallOrigin.IROH, handshakeFailed = true, hasEncryptionHistory = false, contactKeyResolvable = hasKey
            )
            SagaHandshakeState.Securing -> CallSecurityStateResolver.resolve(
                CallOrigin.IROH, handshakeSecuring = true, contactKeyResolvable = hasKey
            )
        }
    }

    /** Session stand-in for one link of a native mesh group (no 1:1 native session). */
    private class MeshLinkSession(
        private val groupId: String,
        override val peerId: IrohNodeId,
        override val sessionId: String
    ) : IrohCallSession {
        override fun connect(onConnected: () -> Unit, onFailed: (String) -> Unit) {
            // Link is opened by IrohNativeBridge.meshAddPeer; state arrives via notifyMeshPeerState.
        }

        override fun disconnect() {
            IrohNativeBridge.meshRemovePeer(groupId, sessionId)
        }
    }

    companion object {
        private const val TAG = "[Iroh Dial Manager]"
        @Volatile private var instance: IrohDialManager? = null
        fun get(context: Context) = instance ?: synchronized(this) {
            instance ?: IrohDialManager(context).also { instance = it }
        }
    }
}
