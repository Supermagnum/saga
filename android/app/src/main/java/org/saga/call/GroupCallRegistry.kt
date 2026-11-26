package org.saga.call

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import org.saga.handshake.SagaHandshakeState

/**
 * Process-wide holder of Iroh group call state (JVM-testable, no Android types).
 *
 * Single group at a time; multiple concurrent Iroh groups are out of scope.
 */
object GroupCallRegistry {
    private val lock = Any()
    private val _groups = MutableStateFlow<Map<String, GroupCallState>>(emptyMap())
    val groups: StateFlow<Map<String, GroupCallState>> = _groups.asStateFlow()

    /** Legs (telecom call keys) whose connect-cue slot was already consumed (settled 1:1). */
    private val legsWithCuePlayed = mutableSetOf<String>()

    fun get(groupId: String): GroupCallState? = _groups.value[groupId]

    fun current(): GroupCallState? = _groups.value.values.firstOrNull()

    fun put(state: GroupCallState) {
        synchronized(lock) {
            _groups.value = _groups.value + (state.groupId to state)
        }
    }

    /** Apply [transform] atomically; no-op when the group is gone. */
    fun update(groupId: String, transform: (GroupCallState) -> GroupCallState) {
        synchronized(lock) {
            val existing = _groups.value[groupId] ?: return
            _groups.value = _groups.value + (groupId to transform(existing))
        }
    }

    fun remove(groupId: String) {
        synchronized(lock) {
            _groups.value = _groups.value - groupId
        }
    }

    fun clearForTest() {
        synchronized(lock) {
            _groups.value = emptyMap()
            legsWithCuePlayed.clear()
        }
    }

    /**
     * Matches a Telecom call id to a leg. Exact key first, then lookup key (a promoted 1:1
     * leg keeps its legacy key, which may differ by a leading '+').
     */
    fun findLeg(callId: String): Pair<GroupCallState, MeshParticipant>? {
        val groups = _groups.value.values
        for (group in groups) {
            group.participantForLeg(callId)?.let { return group to it }
        }
        val digits = callId.filter { it.isDigit() }
        for (group in groups) {
            val match = group.participants.firstOrNull { p ->
                p.lookupKey == callId ||
                    (digits.length >= 7 && p.lookupKey.filter { it.isDigit() } == digits)
            }
            if (match != null) return group to match
        }
        return null
    }

    fun groupForLeg(callId: String): GroupCallState? = findLeg(callId)?.first

    fun groupById(groupId: String): GroupCallState? = get(groupId)

    /** Called from every snapshot publication; no-op for legs that are not in a group. */
    fun onLegSnapshot(callId: String, snapshot: CallSecuritySnapshot) {
        val (group, leg) = findLeg(callId) ?: return
        update(group.groupId) { g ->
            g.updateLeg(leg.telecomCallKey) { it.copy(snapshot = snapshot) }
        }
    }

    fun setKeyCheck(groupId: String, legKey: String, keyCheck: KeyCheck) {
        // Mesh JNI emits mesh-* group ids; the UI model often lives under grp-*. Prefer the
        // group that actually owns the leg so ENCRYPTED_LEGS / padlock state stay accurate.
        val ownerId = get(groupId)?.takeIf { it.participantForLeg(legKey) != null }?.groupId
            ?: findLeg(legKey)?.first?.groupId
            ?: groupId
        update(ownerId) { g -> g.updateLeg(legKey) { it.copy(keyCheck = keyCheck) } }
    }

    /** Confirmed-encrypted remote legs in [groupId] (for e2e ENCRYPTED_LEGS accounting). */
    fun confirmedEncryptedCount(groupId: String): Int =
        get(groupId)?.participants?.count { it.keyCheck.confirmed } ?: 0

    fun confirmedEncryptedKeys(groupId: String): List<String> =
        get(groupId)?.participants?.filter { it.keyCheck.confirmed }?.map { it.lookupKey }
            ?: emptyList()

    /**
     * Gate for plain media on a group leg. Returns true when media must stay off because the
     * link settled without encryption and the adder has not yet confirmed. Encrypted links and
     * links the adder already confirmed are never gated; non-group legs are never gated.
     */
    fun gateUnencryptedMedia(callId: String, state: SagaHandshakeState): Boolean {
        val (group, leg) = findLeg(callId) ?: return false
        if (state != SagaHandshakeState.NeverEncrypted) return false
        if (leg.addedUnencryptedConfirmed) return false
        update(group.groupId) { g ->
            g.updateLeg(leg.telecomCallKey) { it.copy(awaitingUnencryptedConfirmation = true) }
        }
        return true
    }

    /** The user confirmed this link may run without encryption. */
    fun confirmUnencrypted(groupId: String, legKey: String) {
        update(groupId) { g ->
            g.updateLeg(legKey) {
                it.copy(addedUnencryptedConfirmed = true, awaitingUnencryptedConfirmation = false)
            }
        }
    }

    fun markLegConnectCueConsumed(callId: String) {
        synchronized(lock) { legsWithCuePlayed.add(callId) }
    }

    fun anyLegConnectCueConsumed(callIds: Collection<String>): Boolean =
        synchronized(lock) { callIds.any { it in legsWithCuePlayed } }

    /**
     * Claim the single connect cue of this conference. True exactly once per group, so a late
     * joiner, swap or unhold never replays it.
     */
    fun claimConnectCue(groupId: String): Boolean {
        synchronized(lock) {
            val g = _groups.value[groupId] ?: return false
            if (g.connectCueClaimed) return false
            _groups.value = _groups.value + (groupId to g.copy(connectCueClaimed = true))
            return true
        }
    }
}
