package org.saga.ui

import android.os.Bundle
import androidx.appcompat.app.AppCompatActivity
import androidx.lifecycle.lifecycleScope
import com.google.android.material.dialog.MaterialAlertDialogBuilder
import kotlinx.coroutines.launch
import org.saga.R
import org.saga.call.CallOrigin
import org.saga.call.CallSecurityPresenter
import org.saga.call.CallSecurityStateResolver
import org.saga.call.SagaCallEventHub
import org.saga.call.SagaCallRegistry
import org.saga.databinding.ActivityInCallBinding

class SagaInCallActivity : AppCompatActivity() {
    companion object {
        const val EXTRA_CALL_ID = "call_id"
        const val EXTRA_DISPLAY_NAME = "display_name"
        const val EXTRA_ORIGIN = "origin"
    }

    private lateinit var binding: ActivityInCallBinding
    private var callId: String = ""
    private var connectCuePlayed = false
    private var downgradeModalShown = false

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

        val registryContext = SagaCallRegistry.get(callId)
        val snapshot = registryContext?.snapshot
            ?: CallSecurityStateResolver.resolve(origin)

        renderSnapshot(snapshot)
        maybePlayConnectCue(snapshot)
        maybeShowDowngradeModal(snapshot, displayName)

        lifecycleScope.launch {
            SagaCallEventHub.securityUpdates.collect { update ->
                if (update.telecomCallId != callId) return@collect
                renderSnapshot(update.snapshot)
                if (update.playConnectCue) {
                    maybePlayConnectCue(update.snapshot)
                }
                maybeShowDowngradeModal(update.snapshot, displayName)
            }
        }
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
                val snapshot = CallSecurityStateResolver.resolve(
                    origin = CallOrigin.IROH,
                    handshakeFailed = true,
                    hasEncryptionHistory = true,
                    contactKeyResolvable = true
                ).copy(showDowngradeModal = false)
                CallSecurityPresenter.applyConnectSnapshot(applicationContext, snapshot)
            }
            .setNegativeButton(R.string.cancel_call) { dialog, _ ->
                dialog.dismiss()
                finish()
            }
            .show()
    }
}
