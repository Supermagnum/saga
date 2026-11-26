package org.saga.ui

import android.view.View
import android.widget.EditText
import android.widget.Toast
import androidx.core.content.ContextCompat
import androidx.lifecycle.LifecycleOwner
import com.google.android.material.dialog.MaterialAlertDialogBuilder
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch
import org.saga.R
import org.saga.audio.SagaCallMediaController
import org.saga.call.CallOrigin
import org.saga.call.CallSecurityPresenter
import org.saga.call.CallSecuritySnapshot
import org.saga.call.CallSecurityStateResolver
import org.saga.call.ConnectCuePolicy
import org.saga.call.GroupCallFeatures
import org.saga.call.GroupCallRegistry
import org.saga.call.GroupCallState
import org.saga.call.MeshParticipant
import org.saga.call.SagaCallEventHub
import org.saga.databinding.ActivityInCallBinding
import org.saga.databinding.ItemGroupParticipantBinding
import org.saga.dial.DialTarget
import org.saga.dial.DialTargetResolver
import org.saga.dial.IrohContactDirectory
import org.saga.iroh.IrohDialManager
import org.saga.telecom.GroupCallCoordinator
import org.saga.telecom.MultiCallActions
import org.saga.telecom.SagaIrohConnectionRegistry

/**
 * Iroh group-call InCall UI (Add participant, roster rows, leave-group).
 * Enabled in release and debug when [GroupCallFeatures.ENABLED] is true.
 */
object GroupCallInCallUi {
    private val groupDowngradeShown = mutableSetOf<String>()
    private val unencryptedLinkConfirmShown = mutableSetOf<String>()

    fun enabled(): Boolean = GroupCallFeatures.ENABLED

    fun bind(
        activity: SagaInCallActivity,
        binding: ActivityInCallBinding,
        onRefresh: () -> Unit
    ) {
        binding.btnAddParticipant.text = activity.getString(R.string.add_participant)
        binding.btnAddParticipant.setOnClickListener { onAddParticipantClicked(activity) }
    }

    fun collectGroups(owner: LifecycleOwner, scope: CoroutineScope, onRefresh: () -> Unit) {
        scope.launch {
            GroupCallRegistry.groups.collect { onRefresh() }
        }
    }

    fun refreshVisibility(
        binding: ActivityInCallBinding,
        active: android.telecom.Call?,
        conference: android.telecom.Call?,
        ringingEmpty: Boolean
    ) {
        val group = currentGroupForUi()
        val irohFocus = group != null ||
            (active != null && MultiCallActions.origin(active) == CallOrigin.IROH &&
                (conference == null || MultiCallActions.origin(conference) == CallOrigin.IROH))
        binding.btnAddParticipant.visibility =
            if (irohFocus && ringingEmpty) View.VISIBLE else View.GONE
    }

    fun tryRenderGroup(
        activity: SagaInCallActivity,
        binding: ActivityInCallBinding,
        setCallId: (String) -> Unit,
        renderSnapshot: (CallSecuritySnapshot) -> Unit
    ): Boolean {
        val group = currentGroupForUi() ?: return false
        renderGroup(activity, binding, group, setCallId, renderSnapshot)
        return true
    }

    fun onSecurityUpdate(
        activity: SagaInCallActivity,
        update: SagaCallEventHub.SecurityUpdate,
        refresh: () -> Unit
    ): Boolean {
        val group = GroupCallRegistry.groupForLeg(update.telecomCallId)
        if (group == null || group.participants.size < 2) {
            if (update.playConnectCue) {
                GroupCallRegistry.markLegConnectCueConsumed(update.telecomCallId)
            }
            return false
        }
        onGroupSecurityUpdate(activity, group.groupId, update, refresh)
        return true
    }

    fun leaveIfGroup(activity: SagaInCallActivity): Boolean {
        val group = currentGroupForUi() ?: return false
        GroupCallCoordinator.leaveGroup(activity, group.groupId)
        return true
    }

    fun muteGroupOrNull(activity: SagaInCallActivity, muted: Boolean): Boolean {
        val group = currentGroupForUi() ?: return false
        group.participants.forEach {
            SagaCallMediaController.setMuted(
                it.telecomCallKey,
                IrohDialManager.get(activity).sessionIdForCall(it.telecomCallKey),
                muted
            )
        }
        return true
    }

    /** True when the InCall UI is showing an Iroh group (≥2 remote links). */
    fun hasActiveGroup(): Boolean = currentGroupForUi() != null

    /**
     * Hold/resume every group link's media (spec §14.2.2). Returns true when a group was
     * active so the caller can skip the 1:1 hold path (G9). Media-first: do not rely on
     * Telecom Conference hold/unhold (emulator Call.STATE_HOLDING is flaky).
     * Uses [SagaCallMediaController.setHeldAll] so engine keys always match (G9).
     */
    fun holdGroupOrNull(@Suppress("UNUSED_PARAMETER") activity: SagaInCallActivity, held: Boolean): Boolean {
        if (currentGroupForUi() == null) return false
        SagaCallMediaController.setHeldAll(held)
        return true
    }

    /** Whether any group engine is currently held (survives InCall activity recreation). */
    fun isGroupHeld(): Boolean =
        currentGroupForUi() != null && SagaCallMediaController.anyHeld()

    fun markConnectCueConsumed(callId: String) {
        GroupCallRegistry.markLegConnectCueConsumed(callId)
    }

    private fun currentGroupForUi(): GroupCallState? =
        GroupCallRegistry.current()?.takeIf { it.participants.size >= 2 }

    private fun renderGroup(
        activity: SagaInCallActivity,
        binding: ActivityInCallBinding,
        group: GroupCallState,
        setCallId: (String) -> Unit,
        renderSnapshot: (CallSecuritySnapshot) -> Unit
    ) {
        setCallId(group.groupId)
        binding.callerName.text = group.title()
        group.aggregateSnapshot()?.let { renderSnapshot(it) }

        binding.groupParticipants.removeAllViews()
        val introducedByFmt = activity.getString(R.string.introduced_by_fmt)
        group.participants.forEach { participant ->
            val notes = mutableListOf<String>()
            if (participant.awaitingUnencryptedConfirmation) {
                notes.add(activity.getString(R.string.participant_awaiting_confirmation))
            }
            if (participant.needsKeyCheckNote()) {
                notes.add(activity.getString(R.string.participant_key_unverified))
            }
            addParticipantRow(
                activity,
                binding,
                name = participant.displayLabel(introducedByFmt),
                snapshot = participant.effectiveSnapshot(),
                statusSuffix = notes.takeIf { it.isNotEmpty() }?.joinToString(" · "),
                onHangup = {
                    GroupCallCoordinator.hangupParticipant(activity, participant.telecomCallKey)
                }
            )
        }
        binding.groupParticipants.visibility = View.VISIBLE

        group.participants.forEach { participant ->
            if (participant.effectiveSnapshot().showDowngradeModal &&
                groupDowngradeShown.add(participant.telecomCallKey)
            ) {
                showGroupDowngradeModal(activity, participant)
            }
            if (participant.awaitingUnencryptedConfirmation &&
                unencryptedLinkConfirmShown.add(participant.telecomCallKey)
            ) {
                showLinkUnencryptedConfirm(activity, group.groupId, participant)
            }
        }
    }

    private fun addParticipantRow(
        activity: SagaInCallActivity,
        binding: ActivityInCallBinding,
        name: String,
        snapshot: CallSecuritySnapshot,
        statusSuffix: String?,
        onHangup: () -> Unit
    ) {
        val row = ItemGroupParticipantBinding.inflate(
            activity.layoutInflater, binding.groupParticipants, false
        )
        row.participantName.text = name
        row.participantStatus.text =
            if (statusSuffix.isNullOrBlank()) snapshot.statusText
            else "${snapshot.statusText} · $statusSuffix"
        row.participantPadlock.setImageResource(CallSecurityPresenter.padlockDrawableRes(snapshot))
        row.participantPadlock.contentDescription =
            activity.getString(CallSecurityPresenter.padlockContentDescription(snapshot))
        row.participantHangup.setOnClickListener { onHangup() }
        binding.groupParticipants.addView(row.root)
    }

    private fun onGroupSecurityUpdate(
        activity: SagaInCallActivity,
        groupId: String,
        update: SagaCallEventHub.SecurityUpdate,
        refresh: () -> Unit
    ) {
        if (update.playConnectCue && GroupCallRegistry.claimConnectCue(groupId)) {
            if (ConnectCuePolicy.shouldPlay(update.snapshot, irohOrigin = true)) {
                CallSecurityPresenter.applyConnectSnapshot(activity.applicationContext, update.snapshot)
            }
        }
        if (update.snapshot.statusText == activity.getString(R.string.call_not_reachable)) {
            val name = GroupCallRegistry.groupById(groupId)
                ?.participantForLeg(update.telecomCallId)?.displayName
            if (name != null) {
                Toast.makeText(
                    activity,
                    activity.getString(R.string.participant_not_reachable_fmt, name),
                    Toast.LENGTH_SHORT
                ).show()
            }
        }
        refresh()
    }

    private fun showGroupDowngradeModal(activity: SagaInCallActivity, participant: MeshParticipant) {
        if (activity.isFinishing) return
        MaterialAlertDialogBuilder(activity)
            .setTitle(R.string.saga_downgrade_title)
            .setMessage(activity.getString(R.string.saga_downgrade_message, participant.displayName))
            .setCancelable(false)
            .setPositiveButton(R.string.saga_continue_unencrypted) { dialog, _ ->
                dialog.dismiss()
                val continued = CallSecurityStateResolver.resolve(
                    origin = CallOrigin.IROH,
                    handshakeFailed = true,
                    hasEncryptionHistory = true,
                    contactKeyResolvable = true
                ).copy(showDowngradeModal = false)
                GroupCallRegistry.onLegSnapshot(participant.telecomCallKey, continued)
                IrohDialManager.get(activity)
                    .continueUnencryptedMedia(participant.telecomCallKey, participant.telecomCallKey)
            }
            .setNegativeButton(R.string.participant_remove) { dialog, _ ->
                dialog.dismiss()
                GroupCallCoordinator.hangupParticipant(activity, participant.telecomCallKey)
            }
            .show()
    }

    private fun showLinkUnencryptedConfirm(
        activity: SagaInCallActivity,
        groupId: String,
        participant: MeshParticipant
    ) {
        if (activity.isFinishing) return
        MaterialAlertDialogBuilder(activity)
            .setTitle(R.string.confirm_unencrypted_link_title)
            .setMessage(
                activity.getString(R.string.confirm_unencrypted_link_message, participant.displayName)
            )
            .setCancelable(false)
            .setPositiveButton(R.string.confirm_unencrypted_link_positive) { dialog, _ ->
                dialog.dismiss()
                GroupCallRegistry.confirmUnencrypted(groupId, participant.telecomCallKey)
                IrohDialManager.get(activity)
                    .continueUnencryptedMedia(participant.telecomCallKey, participant.telecomCallKey)
            }
            .setNegativeButton(R.string.participant_remove) { dialog, _ ->
                dialog.dismiss()
                GroupCallCoordinator.hangupParticipant(activity, participant.telecomCallKey)
            }
            .show()
    }

    private fun onAddParticipantClicked(activity: SagaInCallActivity) {
        val group = GroupCallRegistry.current()
        if (group != null && !group.canAddMore()) {
            showGroupFullDialog(activity)
            return
        }
        val exclude = buildSet {
            group?.participants?.forEach { add(it.lookupKey) }
            SagaIrohConnectionRegistry.soleLiveLeg()?.let { add(it.lookupKey) }
        }
        val hasContacts = ContextCompat.checkSelfPermission(
            activity, android.Manifest.permission.READ_CONTACTS
        ) == android.content.pm.PackageManager.PERMISSION_GRANTED
        val contacts = if (hasContacts) IrohContactDirectory.list(activity, exclude) else emptyList()
        val labels = contacts.map { it.displayName } + activity.getString(R.string.add_participant_other)
        MaterialAlertDialogBuilder(activity)
            .setTitle(R.string.add_participant_pick_title)
            .setItems(labels.toTypedArray()) { dialog, which ->
                dialog.dismiss()
                if (which < contacts.size) {
                    submitAddParticipant(activity, contacts[which].target, confirmedUnencrypted = false)
                } else {
                    promptOtherParticipant(activity)
                }
            }
            .setNegativeButton(R.string.cancel_call, null)
            .show()
    }

    private fun promptOtherParticipant(activity: SagaInCallActivity) {
        val input = EditText(activity).apply {
            hint = activity.getString(R.string.add_participant_other_hint)
            setSingleLine()
        }
        MaterialAlertDialogBuilder(activity)
            .setTitle(R.string.add_participant)
            .setView(input)
            .setPositiveButton(R.string.add_participant_ok) { dialog, _ ->
                dialog.dismiss()
                val target = DialTargetResolver.fromInput(
                    activity, input.text?.toString().orEmpty(), preferCellular = false
                )
                if (target is DialTarget.Iroh) {
                    submitAddParticipant(activity, target, confirmedUnencrypted = false)
                } else {
                    Toast.makeText(
                        activity, R.string.add_participant_only_saga, Toast.LENGTH_SHORT
                    ).show()
                }
            }
            .setNegativeButton(R.string.cancel_call, null)
            .show()
    }

    private fun submitAddParticipant(
        activity: SagaInCallActivity,
        target: DialTarget.Iroh,
        confirmedUnencrypted: Boolean
    ) {
        val name = target.contactName ?: target.lookupKey
        val outcome = GroupCallCoordinator.requestAdd(
            activity,
            GroupCallCoordinator.AddRequest(target, confirmedUnencrypted)
        )
        when (outcome) {
            GroupCallCoordinator.AddOutcome.PLACED -> Unit
            GroupCallCoordinator.AddOutcome.NEEDS_UNENCRYPTED_CONFIRMATION -> {
                MaterialAlertDialogBuilder(activity)
                    .setTitle(R.string.confirm_add_unencrypted_title)
                    .setMessage(activity.getString(R.string.confirm_add_unencrypted_message, name))
                    .setPositiveButton(R.string.confirm_add_unencrypted_positive) { dialog, _ ->
                        dialog.dismiss()
                        submitAddParticipant(activity, target, confirmedUnencrypted = true)
                    }
                    .setNegativeButton(R.string.cancel_call, null)
                    .show()
            }
            GroupCallCoordinator.AddOutcome.REFUSED_MAX_PARTICIPANTS -> showGroupFullDialog(activity)
            GroupCallCoordinator.AddOutcome.REFUSED_DUPLICATE ->
                Toast.makeText(
                    activity,
                    activity.getString(R.string.add_participant_already_fmt, name),
                    Toast.LENGTH_SHORT
                ).show()
            GroupCallCoordinator.AddOutcome.REFUSED_CELLULAR ->
                Toast.makeText(
                    activity, R.string.add_participant_carrier_refused, Toast.LENGTH_LONG
                ).show()
            GroupCallCoordinator.AddOutcome.REFUSED_NO_IROH_CALL ->
                Toast.makeText(
                    activity, R.string.add_participant_no_iroh_call, Toast.LENGTH_LONG
                ).show()
            GroupCallCoordinator.AddOutcome.REFUSED_PHONE_ACCOUNT,
            GroupCallCoordinator.AddOutcome.FAILED ->
                Toast.makeText(
                    activity, R.string.add_participant_failed, Toast.LENGTH_SHORT
                ).show()
        }
    }

    private fun showGroupFullDialog(activity: SagaInCallActivity) {
        if (activity.isFinishing) return
        MaterialAlertDialogBuilder(activity)
            .setTitle(R.string.group_max_title)
            .setMessage(
                activity.getString(R.string.group_max_participants, GroupCallState.MAX_PARTICIPANTS)
            )
            .setPositiveButton(android.R.string.ok, null)
            .show()
    }
}
