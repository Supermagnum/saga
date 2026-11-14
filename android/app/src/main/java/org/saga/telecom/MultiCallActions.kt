package org.saga.telecom

import android.telecom.Call
import android.telecom.Call.Details
import android.util.Log
import org.saga.call.CallOrigin
import org.saga.call.SagaCallRegistry

/**
 * Capability-gated multi-call / conference helpers for the InCall UI.
 *
 * Carrier conference is cleartext stock parity. Iroh↔carrier merge is refused
 * (UI hide + API guard). Hold/swap across origins is allowed.
 */
object MultiCallActions {
    private const val TAG = "[Saga MultiCall]"

    fun origin(call: Call): CallOrigin = SagaCallRegistry.originForTelecomCall(call)

    fun canHold(call: Call): Boolean =
        call.details.can(Details.CAPABILITY_HOLD) ||
            call.details.can(Details.CAPABILITY_SUPPORT_HOLD)

    fun canSwap(active: Call?, held: Call?): Boolean {
        if (active == null || held == null) return false
        return canHold(active) && (
            held.state == Call.STATE_HOLDING ||
                held.details.can(Details.CAPABILITY_HOLD) ||
                held.details.can(Details.CAPABILITY_SUPPORT_HOLD)
            )
    }

    /** True when every call in [calls] is cellular (carrier conference only). */
    fun allCellular(calls: Collection<Call>): Boolean =
        calls.isNotEmpty() && calls.all { origin(it) == CallOrigin.CELLULAR }

    /**
     * Merge is allowed only when Telecom says so AND all parties are cellular.
     * Mixed Iroh+carrier is never mergeable.
     */
    fun canMerge(primary: Call, others: List<Call>): Boolean {
        if (others.isEmpty()) return false
        val conferenceable = primary.conferenceableCalls
        if (conferenceable.isNullOrEmpty()) return false
        val candidates = others.filter { conferenceable.contains(it) }
        if (candidates.isEmpty()) return false
        if (!allCellular(listOf(primary) + candidates)) {
            return false
        }
        return primary.details.can(Details.CAPABILITY_MERGE_CONFERENCE) ||
            primary.details.can(Details.CAPABILITY_MANAGE_CONFERENCE) ||
            candidates.any { it.details.can(Details.CAPABILITY_MERGE_CONFERENCE) }
    }

    fun canSplit(call: Call): Boolean =
        call.details.can(Details.CAPABILITY_SEPARATE_FROM_CONFERENCE) &&
            call.parent != null

    fun refuseMergeIfMixed(primary: Call, other: Call): Boolean {
        val mixed =
            origin(primary) != origin(other) ||
                origin(primary) == CallOrigin.IROH ||
                origin(other) == CallOrigin.IROH
        if (mixed) {
            Log.w(
                TAG,
                "REFUSE_IROH_CARRIER_MERGE primary=${origin(primary)} other=${origin(other)}"
            )
            return true
        }
        return false
    }

    fun merge(primary: Call, other: Call): Boolean {
        if (refuseMergeIfMixed(primary, other)) return false
        val conferenceable = primary.conferenceableCalls
        if (conferenceable == null || !conferenceable.contains(other)) {
            Log.w(TAG, "merge refused — other not conferenceable")
            return false
        }
        return try {
            primary.conference(other)
            Log.i(TAG, "conference() issued")
            true
        } catch (e: Exception) {
            Log.e(TAG, "conference() failed: ${e.message}")
            false
        }
    }

    fun mergeConference(call: Call): Boolean {
        val childrenOrPeers = call.conferenceableCalls.orEmpty()
        if (!allCellular(listOf(call) + childrenOrPeers)) {
            Log.w(TAG, "REFUSE_IROH_CARRIER_MERGE mergeConference")
            return false
        }
        return try {
            call.mergeConference()
            Log.i(TAG, "mergeConference() issued")
            true
        } catch (e: Exception) {
            Log.e(TAG, "mergeConference() failed: ${e.message}")
            false
        }
    }

    fun split(call: Call): Boolean {
        if (!canSplit(call)) return false
        return try {
            call.splitFromConference()
            Log.i(TAG, "splitFromConference() issued")
            true
        } catch (e: Exception) {
            Log.e(TAG, "splitFromConference() failed: ${e.message}")
            false
        }
    }

    fun swap(active: Call, held: Call) {
        try {
            active.hold()
        } catch (e: Exception) {
            Log.w(TAG, "swap hold active failed: ${e.message}")
        }
        try {
            held.unhold()
        } catch (e: Exception) {
            Log.w(TAG, "swap unhold held failed: ${e.message}")
        }
    }

    fun answerHoldingOthers(incoming: Call, others: List<Call>) {
        others.filter {
            it.state == Call.STATE_ACTIVE || it.state == Call.STATE_DIALING
        }.forEach { call ->
            if (canHold(call)) {
                try {
                    call.hold()
                } catch (e: Exception) {
                    Log.w(TAG, "hold before answer failed: ${e.message}")
                }
            }
        }
        try {
            incoming.answer(0)
        } catch (e: Exception) {
            Log.e(TAG, "answer failed: ${e.message}")
        }
    }

    fun endAndAnswer(incoming: Call, toEnd: Call) {
        try {
            toEnd.disconnect()
        } catch (e: Exception) {
            Log.w(TAG, "end before answer failed: ${e.message}")
        }
        try {
            incoming.answer(0)
        } catch (e: Exception) {
            Log.e(TAG, "answer after end failed: ${e.message}")
        }
    }

    fun reject(incoming: Call) {
        try {
            incoming.reject(false, null)
        } catch (e: Exception) {
            try {
                incoming.disconnect()
            } catch (e2: Exception) {
                Log.e(TAG, "reject failed: ${e.message} / ${e2.message}")
            }
        }
    }

    fun conferenceChildren(call: Call): List<Call> {
        val parent = call.parent ?: call
        return parent.children?.toList().orEmpty()
    }

    fun classify(calls: List<Call>): CallRoles {
        val ringing = calls.filter { it.state == Call.STATE_RINGING }
        val active = calls.firstOrNull {
            it.state == Call.STATE_ACTIVE ||
                it.state == Call.STATE_DIALING ||
                it.state == Call.STATE_CONNECTING
        }
        val held = calls.filter { it.state == Call.STATE_HOLDING }
        val conference = calls.firstOrNull { it.children?.isNotEmpty() == true }
        return CallRoles(
            active = active,
            held = held,
            ringing = ringing,
            conference = conference
        )
    }

    data class CallRoles(
        val active: Call?,
        val held: List<Call>,
        val ringing: List<Call>,
        val conference: Call?
    )
}
