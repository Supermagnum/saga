package org.saga.ui

import android.content.Intent
import android.net.Uri
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
import android.view.View
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
import org.saga.telecom.MultiCallActions

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
    private val registeredCallbacks = mutableListOf<Pair<Call, Call.Callback>>()

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
        binding.endCall.setOnClickListener { endFocusedCall() }

        binding.toggleMute.setOnClickListener { toggleMute() }
        binding.toggleSpeaker.setOnClickListener { toggleSpeaker() }
        binding.toggleHold.setOnClickListener { toggleHold() }
        binding.btnAddCall.setOnClickListener { launchAddCall() }
        binding.btnSwap.setOnClickListener { swapCalls() }
        binding.btnMerge.setOnClickListener { mergeCalls() }
        binding.btnSplit.setOnClickListener { splitFocused() }
        binding.btnAnswerHold.setOnClickListener { answerHoldingOthers() }
        binding.btnReject.setOnClickListener { rejectIncoming() }
        binding.btnEndAndAnswer.setOnClickListener { endAndAnswer() }

        val registryContext = SagaCallRegistry.get(callId)
        val snapshot = registryContext?.snapshot
            ?: CallSecurityStateResolver.resolve(origin)

        renderSnapshot(snapshot)
        maybePlayConnectCue(snapshot)
        maybeShowDowngradeModal(snapshot, displayName)
        refreshAudioStatus()
        refreshMultiCallUi()

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
                    Handler(Looper.getMainLooper()).postDelayed({
                        if (!isFinishing) finish()
                    }, 1500L)
                }
            }
        }

        attachCallCallbacks()
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        callId = intent.getStringExtra(EXTRA_CALL_ID).orEmpty()
        val displayName = intent.getStringExtra(EXTRA_DISPLAY_NAME).orEmpty()
        if (displayName.isNotEmpty()) {
            binding.callerName.text = displayName
        }
        refreshMultiCallUi()
    }

    override fun onResume() {
        super.onResume()
        refreshMultiCallUi()
    }

    override fun onDestroy() {
        registeredCallbacks.forEach { (call, cb) ->
            try {
                call.unregisterCallback(cb)
            } catch (_: Exception) {
            }
        }
        registeredCallbacks.clear()
        super.onDestroy()
    }

    private fun telecomCalls(): List<Call> =
        SagaInCallServiceBinder.service?.calls?.toList().orEmpty()

    private fun focusedCall(): Call? {
        val calls = telecomCalls()
        return calls.firstOrNull { SagaCallRegistry.telecomCallId(it) == callId }
            ?: MultiCallActions.classify(calls).active
            ?: calls.firstOrNull()
    }

    private fun attachCallCallbacks() {
        registeredCallbacks.forEach { (call, cb) ->
            try {
                call.unregisterCallback(cb)
            } catch (_: Exception) {
            }
        }
        registeredCallbacks.clear()
        telecomCalls().forEach { call ->
            val cb = object : Call.Callback() {
                override fun onStateChanged(call: Call, state: Int) {
                    runOnUiThread {
                        if (state == Call.STATE_DISCONNECTED || state == Call.STATE_DISCONNECTING) {
                            val label = call.details?.disconnectCause?.label?.toString()
                            if (!label.isNullOrBlank()) {
                                binding.securityStatus.text = label
                            }
                            if (telecomCalls().none {
                                    it.state != Call.STATE_DISCONNECTED &&
                                        it.state != Call.STATE_DISCONNECTING
                                }
                            ) {
                                Handler(Looper.getMainLooper()).postDelayed({
                                    if (!isFinishing) finish()
                                }, 1200L)
                            }
                        }
                        refreshMultiCallUi()
                    }
                }

                override fun onDetailsChanged(call: Call, details: Call.Details) {
                    runOnUiThread { refreshMultiCallUi() }
                }

                override fun onChildrenChanged(call: Call, children: MutableList<Call>) {
                    runOnUiThread { refreshMultiCallUi() }
                }

                override fun onParentChanged(call: Call, parent: Call?) {
                    runOnUiThread { refreshMultiCallUi() }
                }

                override fun onConferenceableCallsChanged(
                    call: Call,
                    conferenceableCalls: MutableList<Call>
                ) {
                    runOnUiThread { refreshMultiCallUi() }
                }
            }
            call.registerCallback(cb)
            registeredCallbacks.add(call to cb)
        }
    }

    private fun refreshMultiCallUi() {
        val calls = telecomCalls()
        if (calls.size != registeredCallbacks.size) {
            attachCallCallbacks()
        }
        val roles = MultiCallActions.classify(calls)
        val active = roles.active
        val held = roles.held
        val ringing = roles.ringing
        val conference = roles.conference

        // Secondary line: held / incoming.
        val secondary = when {
            ringing.isNotEmpty() -> {
                val name = SagaCallRegistry.displayNameFromCall(ringing.first())
                getString(R.string.incoming_second_fmt, name)
            }
            held.isNotEmpty() -> {
                val name = SagaCallRegistry.displayNameFromCall(held.first())
                getString(R.string.call_on_hold_fmt, name)
            }
            else -> null
        }
        if (secondary != null) {
            binding.callSecondaryLine.visibility = View.VISIBLE
            binding.callSecondaryLine.text = secondary
        } else {
            binding.callSecondaryLine.visibility = View.GONE
        }

        if (active != null) {
            callId = SagaCallRegistry.telecomCallId(active)
            binding.callerName.text = SagaCallRegistry.displayNameFromCall(active)
            SagaCallRegistry.get(callId)?.snapshot?.let { renderSnapshot(it) }
                ?: renderSnapshot(
                    CallSecurityStateResolver.resolve(SagaCallRegistry.originForTelecomCall(active))
                )
        }

        // Conference participants.
        val children = when {
            conference != null -> MultiCallActions.conferenceChildren(conference)
            active != null -> MultiCallActions.conferenceChildren(active)
            else -> emptyList()
        }
        if (children.isNotEmpty()) {
            val names = children.joinToString(", ") { SagaCallRegistry.displayNameFromCall(it) }
            binding.conferenceParticipants.visibility = View.VISIBLE
            binding.conferenceParticipants.text =
                getString(R.string.conference_participants_fmt, names)
        } else {
            binding.conferenceParticipants.visibility = View.GONE
        }

        // Second incoming controls.
        val showIncoming = ringing.isNotEmpty() && (active != null || held.isNotEmpty())
        binding.incomingSecondControls.visibility = if (showIncoming) View.VISIBLE else View.GONE

        // Add call: available when we have an active non-conference leaf call.
        binding.btnAddCall.visibility =
            if (active != null && ringing.isEmpty()) View.VISIBLE else View.GONE

        // Swap when active + held.
        binding.btnSwap.visibility =
            if (MultiCallActions.canSwap(active, held.firstOrNull())) View.VISIBLE else View.GONE

        // Merge: only when conferenceable AND all cellular (never Iroh↔carrier).
        val mergePeer = held.firstOrNull()
            ?: active?.conferenceableCalls?.firstOrNull()
        val showMerge = active != null &&
            mergePeer != null &&
            MultiCallActions.canMerge(active, listOf(mergePeer))
        binding.btnMerge.visibility = if (showMerge) View.VISIBLE else View.GONE

        // Split: child of conference.
        val splitTarget = children.firstOrNull { MultiCallActions.canSplit(it) }
            ?: active?.takeIf { MultiCallActions.canSplit(it) }
        binding.btnSplit.visibility = if (splitTarget != null) View.VISIBLE else View.GONE

        held.forEach { h ->
            if (SagaCallRegistry.originForTelecomCall(h) == CallOrigin.IROH) {
                // Iroh media pause already driven by InCallService onStateChanged.
            }
        }
        binding.toggleHold.isEnabled = active != null && MultiCallActions.canHold(active)
        val anyHeld = held.isNotEmpty()
        this.held = anyHeld && active == null
        binding.toggleHold.text = getString(
            if (this.held) R.string.resume else R.string.hold
        )
    }

    private fun launchAddCall() {
        // Open Saga's own dialer in add-call mode (contacts + search + keypad), then return here.
        Log.i(TAG, "Add call → Saga dialer (add-call mode)")
        startActivity(
            Intent(this, MainActivity::class.java).apply {
                action = Intent.ACTION_DIAL
                putExtra(MainActivity.EXTRA_ADD_CALL_MODE, true)
                addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            }
        )
    }

    private fun swapCalls() {
        val roles = MultiCallActions.classify(telecomCalls())
        val active = roles.active ?: return
        val heldCall = roles.held.firstOrNull() ?: return
        MultiCallActions.swap(active, heldCall)
        refreshMultiCallUi()
    }

    private fun mergeCalls() {
        val roles = MultiCallActions.classify(telecomCalls())
        val active = roles.active ?: return
        val other = roles.held.firstOrNull()
            ?: active.conferenceableCalls?.firstOrNull()
            ?: return
        if (MultiCallActions.refuseMergeIfMixed(active, other)) {
            binding.btnMerge.visibility = View.GONE
            return
        }
        if (!MultiCallActions.merge(active, other)) {
            MultiCallActions.mergeConference(active)
        }
        refreshMultiCallUi()
    }

    private fun splitFocused() {
        val roles = MultiCallActions.classify(telecomCalls())
        val children = roles.conference?.let { MultiCallActions.conferenceChildren(it) }.orEmpty()
        val target = children.firstOrNull { MultiCallActions.canSplit(it) }
            ?: roles.active?.takeIf { MultiCallActions.canSplit(it) }
            ?: return
        MultiCallActions.split(target)
        refreshMultiCallUi()
    }

    private fun answerHoldingOthers() {
        val roles = MultiCallActions.classify(telecomCalls())
        val incoming = roles.ringing.firstOrNull() ?: return
        val others = telecomCalls().filter { it !== incoming }
        MultiCallActions.answerHoldingOthers(incoming, others)
        refreshMultiCallUi()
    }

    private fun rejectIncoming() {
        val incoming = MultiCallActions.classify(telecomCalls()).ringing.firstOrNull() ?: return
        MultiCallActions.reject(incoming)
        refreshMultiCallUi()
    }

    private fun endAndAnswer() {
        val roles = MultiCallActions.classify(telecomCalls())
        val incoming = roles.ringing.firstOrNull() ?: return
        val toEnd = roles.active ?: roles.held.firstOrNull() ?: return
        MultiCallActions.endAndAnswer(incoming, toEnd)
        refreshMultiCallUi()
    }

    private fun endFocusedCall() {
        val call = focusedCall()
        if (call != null) {
            try {
                // If this is a conference parent, end all children.
                val children = MultiCallActions.conferenceChildren(call)
                if (children.isNotEmpty()) {
                    children.forEach { child ->
                        try {
                            child.disconnect()
                        } catch (_: Exception) {
                        }
                    }
                }
                call.disconnect()
            } catch (e: Exception) {
                Log.w(TAG, "disconnect failed: ${e.message}")
                finish()
            }
        } else {
            finish()
        }
    }

    private fun rustSessionId(): String? =
        IrohDialManager.get(this).sessionIdForCall(callId)

    private fun toggleMute() {
        muted = !muted
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
        val call = focusedCall() ?: return
        val roles = MultiCallActions.classify(telecomCalls())
        if (call.state == Call.STATE_HOLDING) {
            call.unhold()
            held = false
        } else if (MultiCallActions.canHold(call)) {
            call.hold()
            held = true
            // If another call is held and we just held active with no other active, try unhold peer.
            roles.held.firstOrNull()?.let { other ->
                // leave held; swap is explicit
            }
            SagaCallMediaController.setHeld(callId, rustSessionId(), true)
        }
        binding.toggleHold.text = getString(if (held) R.string.resume else R.string.hold)
        refreshAudioStatus()
        refreshMultiCallUi()
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
