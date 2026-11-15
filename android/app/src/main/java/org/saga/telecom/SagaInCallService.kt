package org.saga.telecom

import android.content.Intent
import android.os.Build
import android.os.SystemClock
import android.telecom.Call
import android.telecom.CallEndpoint
import android.telecom.InCallService
import android.util.Log
import org.saga.call.CallOrigin
import org.saga.call.CallSecurityPresenter
import org.saga.call.CallSecurityStateResolver
import org.saga.call.SagaCallEventHub
import org.saga.call.SagaCallRegistry
import org.saga.iroh.PendingDialLookup
import org.saga.iroh.SagaDialer
import org.saga.ui.SagaInCallActivity
import org.saga.ui.SagaInCallServiceBinder

/**
 * Default-dialer InCallService: one InCall UI for all calls (active + held + ringing).
 */
class SagaInCallService : InCallService() {
    companion object {
        private const val TAG = "[Saga InCallService]"
    }

    override fun onCreate() {
        super.onCreate()
        SagaInCallServiceBinder.service = this
    }

    override fun onDestroy() {
        if (SagaInCallServiceBinder.service === this) {
            SagaInCallServiceBinder.service = null
            SagaInCallServiceBinder.availableEndpoints = emptyList()
        }
        super.onDestroy()
    }

    override fun onAvailableCallEndpointsChanged(availableEndpoints: MutableList<CallEndpoint>) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            SagaInCallServiceBinder.availableEndpoints = availableEndpoints.toList()
            Log.i(TAG, "CallEndpoints available=${availableEndpoints.map { it.endpointType }}")
        }
    }

    override fun onCallEndpointChanged(callEndpoint: CallEndpoint) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            Log.i(TAG, "CallEndpoint active type=${callEndpoint.endpointType}")
        }
    }

    override fun onCallAdded(call: Call) {
        super.onCallAdded(call)
        val accountId = call.details.accountHandle?.id
        val isSagaAccount = accountId == SagaPhoneAccountRegistrar.ACCOUNT_ID

        // BACKSTOP ONLY: if Telecom substituted a SIM account after an Iroh dial, abort.
        if (!isSagaAccount && PendingDialLookup.hasRecentIrohDial()) {
            val now = SystemClock.elapsedRealtime()
            val placeAt = SagaDialer.debugForceSimPlaceCallElapsedRealtimeMs
            val delta = if (placeAt > 0L) now - placeAt else -1L
            Log.e(
                TAG,
                "IROH_ABORT_SIM_SUBSTITUTION account=[$accountId] handle=[${call.details.handle}] " +
                    "abortAtElapsed=$now placeCallDeltaMs=$delta"
            )
            PendingDialLookup.clearAll()
            PendingDialLookup.markIrohDialConsumed()
            try {
                call.disconnect()
            } catch (e: Exception) {
                Log.e(TAG, "Failed to disconnect substituted SIM call: ${e.message}")
            }
            return
        }

        val origin = SagaCallRegistry.originForTelecomCall(call)
        val peerId = SagaCallRegistry.peerIdFromCall(call)
        val displayName = SagaCallRegistry.displayNameFromCall(call)
        val callId = SagaCallRegistry.telecomCallId(call)
        val snapshot = SagaCallRegistry.initialSnapshot(origin, peerId, applicationContext)
        SagaCallRegistry.put(
            org.saga.call.ActiveCallContext(
                telecomCallId = callId,
                displayName = displayName,
                origin = origin,
                snapshot = snapshot
            )
        )
        Log.i(
            TAG,
            "CHECKPOINT onCallAdded origin=[$origin] peer=[$peerId] telecomState=[${call.state}] " +
                "calls=${calls.size}"
        )
        if (isSagaAccount) {
            PendingDialLookup.markIrohDialConsumed()
        }
        call.registerCallback(callCallback)

        // Single InCall activity: bring to front / update rather than stacking.
        startActivity(
            Intent(this, SagaInCallActivity::class.java).apply {
                addFlags(
                    Intent.FLAG_ACTIVITY_NEW_TASK or
                        Intent.FLAG_ACTIVITY_SINGLE_TOP or
                        Intent.FLAG_ACTIVITY_REORDER_TO_FRONT
                )
                putExtra(SagaInCallActivity.EXTRA_CALL_ID, callId)
                putExtra(SagaInCallActivity.EXTRA_DISPLAY_NAME, displayName)
                putExtra(SagaInCallActivity.EXTRA_ORIGIN, origin.name)
            }
        )
    }

    override fun onCallRemoved(call: Call) {
        super.onCallRemoved(call)
        val callId = SagaCallRegistry.telecomCallId(call)
        val sessionId = org.saga.iroh.IrohDialManager.get(applicationContext)
            .sessionIdForCall(callId)
        org.saga.audio.SagaCallMediaController.stop(callId, sessionId)
        SagaCallRegistry.remove(callId)
        call.unregisterCallback(callCallback)
        Log.i(TAG, "onCallRemoved callId=[$callId] remaining=${calls.size}")
    }

    private val callCallback = object : Call.Callback() {
        override fun onStateChanged(call: Call, state: Int) {
            val callId = SagaCallRegistry.telecomCallId(call)
            val ctx = SagaCallRegistry.get(callId) ?: return
            if (state == Call.STATE_ACTIVE && ctx.origin == CallOrigin.CELLULAR) {
                val snapshot = CallSecurityStateResolver.resolve(CallOrigin.CELLULAR)
                SagaCallRegistry.updateSnapshot(callId, snapshot)
                // Connect cue once per cellular active — never on swap/unhold/merge.
                if (!cellularCuePlayed.contains(callId)) {
                    CallSecurityPresenter.applyConnectSnapshot(applicationContext, snapshot)
                    cellularCuePlayed.add(callId)
                }
                SagaCallEventHub.emit(
                    SagaCallEventHub.SecurityUpdate(
                        callId,
                        snapshot,
                        playConnectCue = false
                    )
                )
            }
            // Cellular holding an Iroh call: STATE_HOLDING on the Iroh Call.
            if (ctx.origin == CallOrigin.IROH) {
                val sessionId = org.saga.iroh.IrohDialManager.get(applicationContext)
                    .sessionIdForCall(callId)
                when (state) {
                    Call.STATE_HOLDING ->
                        org.saga.audio.SagaCallMediaController.setHeld(callId, sessionId, true)
                    Call.STATE_ACTIVE ->
                        org.saga.audio.SagaCallMediaController.setHeld(callId, sessionId, false)
                }
            }
            if (state == Call.STATE_DISCONNECTED) {
                cellularCuePlayed.remove(callId)
            }
        }

        override fun onConferenceableCallsChanged(
            call: Call,
            conferenceableCalls: MutableList<Call>
        ) {
            // Guard: refuse programmatic merge of mixed origins if anything else tries.
            val origin = SagaCallRegistry.originForTelecomCall(call)
            conferenceableCalls.forEach { other ->
                if (MultiCallActions.refuseMergeIfMixed(call, other)) {
                    Log.i(
                        TAG,
                        "conferenceable mixed origins — Merge hidden; API merge refused " +
                            "primary=$origin other=${SagaCallRegistry.originForTelecomCall(other)}"
                    )
                }
            }
        }
    }

    private val cellularCuePlayed = mutableSetOf<String>()
}
