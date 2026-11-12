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
import org.saga.telecom.SagaIrohConnection
import android.telecom.Connection
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

    fun onIncomingAnswered(telecomCallKey: String) {
        val connection = incomingConnections[telecomCallKey]
        if (connection == null) {
            Log.w(TAG, "onIncomingAnswered: no connection for [$telecomCallKey]")
            return
        }
        connection.setActive()
        Log.i(TAG, "CHECKPOINT incoming Connection setActive key=[$telecomCallKey]")
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
            Log.i(
                TAG,
                "Incoming handshake settled to [$state] while RINGING — waiting for answer"
            )
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
        publishSecuring(lookupKey, telecomCallKey)

        session.connect(
            onConnected = {
                handshakeCoordinator.start(lookupKey, session.sessionId) { state ->
                    onHandshakeSettled(lookupKey, telecomCallKey, state, connection)
                }
            },
            onFailed = {
                onHandshakeSettled(
                    lookupKey,
                    telecomCallKey,
                    handshakeCoordinator.resolveFailure(lookupKey),
                    connection
                )
            }
        )
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
        publishHandshakeState(lookupKey, telecomCallKey, state, connection, activateConnection = true)
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
        if (state == SagaHandshakeState.Encrypted || state == SagaHandshakeState.NeverEncrypted) {
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

    companion object {
        private const val TAG = "[Iroh Dial Manager]"
        @Volatile private var instance: IrohDialManager? = null
        fun get(context: Context) = instance ?: synchronized(this) {
            instance ?: IrohDialManager(context).also { instance = it }
        }
    }
}
