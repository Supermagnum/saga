package org.saga.ui

import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.OutcomeReceiver
import android.telecom.Call
import android.telecom.CallAudioState
import android.telecom.CallEndpoint
import android.telecom.InCallService
import android.util.Log
import androidx.appcompat.app.AppCompatActivity
import androidx.lifecycle.lifecycleScope
import com.google.android.material.dialog.MaterialAlertDialogBuilder
import java.util.concurrent.Executors
import kotlinx.coroutines.launch
import org.saga.R
import org.saga.audio.SagaCallMediaController
import org.saga.call.CallOrigin
import org.saga.call.CallSecurityPresenter
import org.saga.call.CallSecurityStateResolver
import org.saga.call.PadlockDisplayState
import org.saga.call.SagaCallEventHub
import org.saga.call.SagaCallRegistry
import org.saga.databinding.ActivityInCallBinding
import org.saga.iroh.IrohDialManager

class SagaInCallActivity : AppCompatActivity() {
    companion object {
        private const val TAG = "[Saga InCallUI]"
        const val EXTRA_CALL_ID = "call_id"
        const val EXTRA_DISPLAY_NAME = "display_name"
        const val EXTRA_ORIGIN = "origin"
    }

    private lateinit var binding: ActivityInCallBinding
    private var callId: String = ""
    private var connectCuePlayed = false
    private var downgradeModalShown = false
    private var muted = false
    private var speakerOn = false
    private var held = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        binding = ActivityInCallBinding.inflate(layoutInflater)
        setContentView(binding.root)

        callId = intent.getStringExtra(EXTRA_CALL_ID).orEmpty()
        val displayName = intent.getStringExtra(EXTRA_DISPLAY_NAME).orEmpty()
        val origin = intent.getStringExtra(EXTRA_ORIGIN)?.let {
            runCatching { CallOrigin.valueOf(it) }.getOrDefault(CallOrigin.CELLULAR)
        } ?: CallOrigin.CELLULAR

        binding.callerName.text = displayName.ifEmpty { "Call" }
        binding.endCall.setOnClickListener { finish() }

        binding.toggleMute.setOnClickListener { toggleMute() }
        binding.toggleSpeaker.setOnClickListener { toggleSpeaker() }
        binding.toggleHold.setOnClickListener { toggleHold() }

        val registryContext = SagaCallRegistry.get(callId)
        val snapshot = registryContext?.snapshot
            ?: CallSecurityStateResolver.resolve(origin)

        renderSnapshot(snapshot)
        maybePlayConnectCue(snapshot)
        maybeShowDowngradeModal(snapshot, displayName)
        refreshAudioStatus()

        lifecycleScope.launch {
            SagaCallEventHub.securityUpdates.collect { update ->
                if (update.telecomCallId != callId) return@collect
                renderSnapshot(update.snapshot)
                if (update.playConnectCue) {
                    maybePlayConnectCue(update.snapshot)
                }
                maybeShowDowngradeModal(update.snapshot, displayName)
                if (update.snapshot.statusText == getString(R.string.call_not_reachable) ||
                    update.snapshot.statusText == "Not reachable"
                ) {
                    // Brief status visibility, then leave the dead call screen.
                    Handler(Looper.getMainLooper()).postDelayed({
                        if (!isFinishing) finish()
                    }, 1500L)
                }
            }
        }

        // Telecom disconnect (including Not reachable) should close the screen.
        SagaInCallServiceBinder.service?.calls?.firstOrNull {
            SagaCallRegistry.telecomCallId(it) == callId
        }?.registerCallback(object : Call.Callback() {
            override fun onStateChanged(call: Call, state: Int) {
                if (state == Call.STATE_DISCONNECTED || state == Call.STATE_DISCONNECTING) {
                    val label = call.details?.disconnectCause?.label?.toString()
                    if (!label.isNullOrBlank()) {
                        binding.securityStatus.text = label
                    }
                    if (!isFinishing) {
                        Handler(Looper.getMainLooper()).postDelayed({
                            if (!isFinishing) finish()
                        }, 1200L)
                    }
                }
            }
        })
    }

    private fun rustSessionId(): String? =
        IrohDialManager.get(this).sessionIdForCall(callId)

    private fun toggleMute() {
        muted = !muted
        // Telecom owns mute: InCallService.setMuted → Connection CallAudioState;
        // engine stops sending but keeps the capture/playback stream alive.
        SagaInCallServiceBinder.service?.setMuted(muted)
        SagaCallMediaController.setMuted(callId, rustSessionId(), muted)
        binding.toggleMute.text = getString(if (muted) R.string.unmute else R.string.mute)
        refreshAudioStatus()
    }

    private fun toggleSpeaker() {
        speakerOn = !speakerOn
        val route = if (speakerOn) CallAudioState.ROUTE_SPEAKER else CallAudioState.ROUTE_EARPIECE
        val svc = SagaInCallServiceBinder.service
        if (svc != null) {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
                routeViaCallEndpoint(svc, route)
            } else {
                @Suppress("DEPRECATION")
                svc.setAudioRoute(route)
            }
        }
        binding.toggleSpeaker.text = getString(if (speakerOn) R.string.earpiece else R.string.speaker)
        refreshAudioStatus()
    }

    @androidx.annotation.RequiresApi(Build.VERSION_CODES.UPSIDE_DOWN_CAKE)
    private fun routeViaCallEndpoint(svc: InCallService, route: Int) {
        val wantType = when (route) {
            CallAudioState.ROUTE_SPEAKER -> CallEndpoint.TYPE_SPEAKER
            CallAudioState.ROUTE_BLUETOOTH -> CallEndpoint.TYPE_BLUETOOTH
            CallAudioState.ROUTE_WIRED_HEADSET -> CallEndpoint.TYPE_WIRED_HEADSET
            else -> CallEndpoint.TYPE_EARPIECE
        }
        val endpoint = SagaInCallServiceBinder.availableEndpoints
            .firstOrNull { it.endpointType == wantType }
        if (endpoint == null) {
            Log.w(TAG, "no CallEndpoint type=$wantType; falling back to setAudioRoute")
            @Suppress("DEPRECATION")
            svc.setAudioRoute(route)
            return
        }
        svc.requestCallEndpointChange(
            endpoint,
            Executors.newSingleThreadExecutor(),
            object : OutcomeReceiver<Void, android.telecom.CallEndpointException> {
                override fun onResult(result: Void?) {
                    Log.i(TAG, "CallEndpoint route ok type=$wantType")
                }
                override fun onError(error: android.telecom.CallEndpointException) {
                    Log.w(TAG, "CallEndpoint change failed: ${error.message}")
                }
            }
        )
    }

    private fun toggleHold() {
        held = !held
        // Prefer Telecom Call.hold/unhold so Connection.onHold/onUnhold owns Iroh pause.
        val call = SagaInCallServiceBinder.service?.calls?.firstOrNull {
            SagaCallRegistry.telecomCallId(it) == callId
        }
        if (call != null) {
            if (held) call.hold() else call.unhold()
        } else {
            SagaCallMediaController.setHeld(callId, rustSessionId(), held)
        }
        binding.toggleHold.text = getString(if (held) R.string.resume else R.string.hold)
        refreshAudioStatus()
    }

    private fun refreshAudioStatus() {
        val parts = mutableListOf<String>()
        if (muted) parts.add(getString(R.string.audio_status_muted))
        if (speakerOn) parts.add(getString(R.string.audio_status_speaker))
        if (held) parts.add(getString(R.string.audio_status_held))
        binding.callAudioStatus.text =
            if (parts.isEmpty()) getString(R.string.audio_status_idle) else parts.joinToString(" · ")
    }

    private fun renderSnapshot(snapshot: org.saga.call.CallSecuritySnapshot) {
        binding.securityStatus.text = snapshot.statusText
        binding.padlockIcon.setImageResource(CallSecurityPresenter.padlockDrawableRes(snapshot))
        binding.padlockIcon.contentDescription = getString(
            CallSecurityPresenter.padlockContentDescription(snapshot)
        )
    }

    private fun maybePlayConnectCue(snapshot: org.saga.call.CallSecuritySnapshot) {
        if (connectCuePlayed) return
        if (!snapshot.playSecureCue && !snapshot.playUnsecureCue) return
        if (snapshot.playUnsecureCue && !snapshot.playSecureCue &&
            snapshot.padlockState == PadlockDisplayState.KEY_NOT_FOUND &&
            intent.getStringExtra(EXTRA_ORIGIN) == CallOrigin.IROH.name
        ) {
            return
        }
        CallSecurityPresenter.applyConnectSnapshot(applicationContext, snapshot)
        connectCuePlayed = true
    }

    private fun maybeShowDowngradeModal(snapshot: org.saga.call.CallSecuritySnapshot, displayName: String) {
        if (!snapshot.showDowngradeModal || downgradeModalShown) return
        downgradeModalShown = true
        MaterialAlertDialogBuilder(this)
            .setTitle(R.string.saga_downgrade_title)
            .setMessage(getString(R.string.saga_downgrade_message, displayName))
            .setCancelable(false)
            .setPositiveButton(R.string.saga_continue_unencrypted) { dialog, _ ->
                dialog.dismiss()
                val continued = CallSecurityStateResolver.resolve(
                    origin = CallOrigin.IROH,
                    handshakeFailed = true,
                    hasEncryptionHistory = true,
                    contactKeyResolvable = true
                ).copy(showDowngradeModal = false)
                CallSecurityPresenter.applyConnectSnapshot(applicationContext, continued)
                IrohDialManager.get(this).continueUnencryptedMedia(callId, callId)
            }
            .setNegativeButton(R.string.cancel_call) { dialog, _ ->
                dialog.dismiss()
                finish()
            }
            .show()
    }
}

/**
 * Weak binder so the in-call UI can call [InCallService.setAudioRoute] /
 * [InCallService.requestCallEndpointChange].
 */
object SagaInCallServiceBinder {
    @Volatile
    var service: InCallService? = null

    @Volatile
    var availableEndpoints: List<CallEndpoint> = emptyList()
}
