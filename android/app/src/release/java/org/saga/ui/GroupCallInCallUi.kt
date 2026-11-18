package org.saga.ui

import android.view.View
import androidx.lifecycle.LifecycleOwner
import kotlinx.coroutines.CoroutineScope
import org.saga.call.GroupCallFeatures
import org.saga.call.SagaCallEventHub
import org.saga.databinding.ActivityInCallBinding

/**
 * Release stub: no Iroh Add participant / group roster UI.
 * Carrier multi-call (Add call / Merge / Split) stays in [SagaInCallActivity].
 */
object GroupCallInCallUi {
    fun enabled(): Boolean = GroupCallFeatures.ENABLED

    fun bind(
        @Suppress("UNUSED_PARAMETER") activity: SagaInCallActivity,
        binding: ActivityInCallBinding,
        @Suppress("UNUSED_PARAMETER") onRefresh: () -> Unit
    ) {
        binding.btnAddParticipant.visibility = View.GONE
        binding.btnAddParticipant.setOnClickListener(null)
        binding.groupParticipants.visibility = View.GONE
    }

    fun collectGroups(
        @Suppress("UNUSED_PARAMETER") owner: LifecycleOwner,
        @Suppress("UNUSED_PARAMETER") scope: CoroutineScope,
        @Suppress("UNUSED_PARAMETER") onRefresh: () -> Unit
    ) {
        // no-op
    }

    fun refreshVisibility(
        binding: ActivityInCallBinding,
        @Suppress("UNUSED_PARAMETER") active: android.telecom.Call?,
        @Suppress("UNUSED_PARAMETER") conference: android.telecom.Call?,
        @Suppress("UNUSED_PARAMETER") ringingEmpty: Boolean
    ) {
        binding.btnAddParticipant.visibility = View.GONE
    }

    fun tryRenderGroup(
        @Suppress("UNUSED_PARAMETER") activity: SagaInCallActivity,
        @Suppress("UNUSED_PARAMETER") binding: ActivityInCallBinding,
        @Suppress("UNUSED_PARAMETER") setCallId: (String) -> Unit,
        @Suppress("UNUSED_PARAMETER") renderSnapshot: (org.saga.call.CallSecuritySnapshot) -> Unit
    ): Boolean = false

    fun onSecurityUpdate(
        @Suppress("UNUSED_PARAMETER") activity: SagaInCallActivity,
        @Suppress("UNUSED_PARAMETER") update: SagaCallEventHub.SecurityUpdate,
        @Suppress("UNUSED_PARAMETER") refresh: () -> Unit
    ): Boolean = false

    fun leaveIfGroup(@Suppress("UNUSED_PARAMETER") activity: SagaInCallActivity): Boolean = false

    fun muteGroupOrNull(
        @Suppress("UNUSED_PARAMETER") activity: SagaInCallActivity,
        @Suppress("UNUSED_PARAMETER") muted: Boolean
    ): Boolean = false

    fun hasActiveGroup(): Boolean = false

    fun holdGroupOrNull(
        @Suppress("UNUSED_PARAMETER") activity: SagaInCallActivity,
        @Suppress("UNUSED_PARAMETER") held: Boolean
    ): Boolean = false

    fun isGroupHeld(): Boolean = false

    fun markConnectCueConsumed(@Suppress("UNUSED_PARAMETER") callId: String) {
        // no-op
    }
}
