package org.saga.telecom

import android.content.Intent
import android.telecom.Call
import android.telecom.InCallService
import android.util.Log
import org.saga.call.CallOrigin
import org.saga.call.CallSecurityPresenter
import org.saga.call.CallSecurityStateResolver
import org.saga.call.SagaCallRegistry
import org.saga.ui.SagaInCallActivity

/**
 * Default-dialer InCallService: draws the call screen for every call on device.
 */
class SagaInCallService : InCallService() {
    companion object {
        private const val TAG = "[Saga InCallService]"
    }

    override fun onCallAdded(call: Call) {
        super.onCallAdded(call)
        val origin = SagaCallRegistry.originForTelecomCall(call)
        val peerId = SagaCallRegistry.peerIdFromCall(call)
        val displayName = peerId
        val callId = SagaCallRegistry.telecomCallId(call)
        val snapshot = SagaCallRegistry.initialSnapshot(origin, peerId)
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
        }
    }
}
