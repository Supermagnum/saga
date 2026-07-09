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
import org.saga.security.DowngradeEventLog
import org.saga.security.midcall.MidCallRehandshakeOutcome
import org.saga.security.midcall.SagaMidCallSecurityAudioWarning
import org.saga.telecom.SagaIrohConnection
import java.util.concurrent.ConcurrentHashMap

class IrohDialManager private constructor(context: Context) {
    private val appContext = context.applicationContext
    private val encryptionStore = EncryptionEstablishedStore(appContext)
    private val handshakeCoordinator = SagaHandshakeCoordinator(appContext, encryptionStore)
    private val sessions = ConcurrentHashMap<String, IrohCallSession>()
    private val lookupKeys = ConcurrentHashMap<String, String>()
    private val scope = CoroutineScope(Dispatchers.Default + SupervisorJob())

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
        sessions.remove(telecomCallKey)?.disconnect()
        lookupKeys.remove(telecomCallKey)
    }

    fun sessionIdForCall(telecomCallKey: String): String? = sessions[telecomCallKey]?.sessionId

    fun lookupKeyForCall(telecomCallKey: String): String? = lookupKeys[telecomCallKey]

    fun findTelecomCallKey(lookupKey: String): String? {
        lookupKeys.entries.firstOrNull { it.value == lookupKey }?.key?.let { return it }
        val peerUri = SagaCallUri.forPeer(IrohNodeId.parse(lookupKey) ?: return null).toString()
        if (sessions.containsKey(peerUri)) return peerUri
        return sessions.keys.firstOrNull { it.contains(lookupKey) }
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
                        Log.i(TAG, "Mid-call re-handshake succeeded — no interruption")
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
        SagaCallEventHub.emit(
            SagaCallEventHub.SecurityUpdate(
                telecomCallKey,
                snapshot,
                playConnectCue = state != SagaHandshakeState.Downgraded
            )
        )
        if (state != SagaHandshakeState.Securing) {
            Log.i(TAG, "Handshake settled to [$state] for [$lookupKey]")
        }
        if (activateConnection && connection != null &&
            state != SagaHandshakeState.Downgraded &&
            state != SagaHandshakeState.Securing
        ) {
            connection.setActive()
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
