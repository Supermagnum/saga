package org.saga.call

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import org.saga.handshake.SagaHandshakeState

/**
 * Release stub: no Iroh group state. Debug implementation in src/debug.
 */
object GroupCallRegistry {
    private val _groups = MutableStateFlow<Map<String, GroupCallState>>(emptyMap())
    val groups: StateFlow<Map<String, GroupCallState>> = _groups.asStateFlow()

    fun get(groupId: String): GroupCallState? = null

    fun current(): GroupCallState? = null

    fun put(state: GroupCallState) {
        // no-op
    }

    fun update(groupId: String, transform: (GroupCallState) -> GroupCallState) {
        // no-op
    }

    fun remove(groupId: String) {
        // no-op
    }

    fun clearForTest() {
        // no-op
    }

    fun findLeg(callId: String): Pair<GroupCallState, MeshParticipant>? = null

    fun groupForLeg(callId: String): GroupCallState? = null

    fun groupById(groupId: String): GroupCallState? = null

    fun onLegSnapshot(callId: String, snapshot: CallSecuritySnapshot) {
        // no-op
    }

    fun setKeyCheck(groupId: String, legKey: String, keyCheck: KeyCheck) {
        // no-op
    }

    fun gateUnencryptedMedia(callId: String, state: SagaHandshakeState): Boolean = false

    fun confirmUnencrypted(groupId: String, legKey: String) {
        // no-op
    }

    fun markLegConnectCueConsumed(callId: String) {
        // no-op
    }

    fun anyLegConnectCueConsumed(callIds: Collection<String>): Boolean = false

    fun claimConnectCue(groupId: String): Boolean = false
}
