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
import org.saga.call.SagaCallRegistry
import org.saga.iroh.PendingDialLookup
import org.saga.iroh.SagaDialer
import org.saga.ui.SagaInCallActivity
import org.saga.ui.SagaInCallServiceBinder

/**
 * Default-dialer InCallService: draws the call screen for every call on device.
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
        // Primary guard is SagaDialer refusing placeCall when the PhoneAccount is disabled.
        // API 35 emulator forensics (DEBUG_FORCE_SIM_PATH + legacy ssp "iroh"):
        //   placeCall → abort ~39ms later (handle already tel:4764), then RIL DIAL / AT> ATD4764;
        //   still ran ~110ms after placeCall (~70ms after abort). This backstop does NOT prevent
        //   a carrier dial attempt; keep the pre-dial enable check and non-digit SSP as primary.
        // See TEST_RESULTS.md (PhoneAccount tel SSP / SIM abort timing).
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
            "CHECKPOINT onCallAdded origin=[$origin] peer=[$peerId] telecomState=[${call.state}]"
        )
        if (isSagaAccount) {
            PendingDialLookup.markIrohDialConsumed()
        }
        call.registerCallback(callCallback)
        startActivity(
            Intent(this, SagaInCallActivity::class.java).apply {
                addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
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
    }

    private val callCallback = object : Call.Callback() {
        override fun onStateChanged(call: Call, state: Int) {
            val callId = SagaCallRegistry.telecomCallId(call)
            val ctx = SagaCallRegistry.get(callId) ?: return
            if (state == Call.STATE_ACTIVE && ctx.origin == CallOrigin.CELLULAR) {
                val snapshot = CallSecurityStateResolver.resolve(CallOrigin.CELLULAR)
                SagaCallRegistry.updateSnapshot(callId, snapshot)
                CallSecurityPresenter.applyConnectSnapshot(applicationContext, snapshot)
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
        }
    }
}
