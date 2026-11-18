package org.saga.call

/**
 * Release stub: Iroh group-call model is debug-only (see src/debug).
 * Types remain so main ConnectionService / InCall compile; nothing is reachable.
 *
 * Reason: G4/G5 mean a group could show Encrypted without every key verified.
 */

enum class MeshBackend {
    NATIVE_MESH,
    PAIRWISE_SESSIONS
}

enum class KeyCheck {
    UNVERIFIED,
    CONFIRMED_LOCAL_CONTACT,
    CONFIRMED_ROSTER,
    KEY_CHANGED,
    MISMATCH;

    val confirmed: Boolean
        get() = this == CONFIRMED_LOCAL_CONTACT || this == CONFIRMED_ROSTER

    val rejected: Boolean
        get() = this == KEY_CHANGED || this == MISMATCH
}

data class MeshParticipant(
    val lookupKey: String,
    val endpointId: String,
    val displayName: String,
    val introducerName: String? = null,
    val snapshot: CallSecuritySnapshot,
    val rustSessionId: String? = null,
    val telecomCallKey: String,
    val introduced: Boolean = false,
    val knownContact: Boolean = true,
    val keyCheck: KeyCheck = KeyCheck.UNVERIFIED,
    val addedUnencryptedConfirmed: Boolean = false,
    val awaitingUnencryptedConfirmation: Boolean = false
) {
    fun effectiveSnapshot(): CallSecuritySnapshot = snapshot

    fun isEncryptedAndConfirmed(): Boolean = false

    fun showsIntroducedBy(): Boolean = false

    fun displayLabel(introducedByFmt: String): String = displayName

    fun needsKeyCheckNote(): Boolean = false
}

enum class AddParticipantDecision {
    ALLOWED,
    NEEDS_UNENCRYPTED_CONFIRMATION,
    REFUSED_MAX_PARTICIPANTS,
    REFUSED_DUPLICATE,
    REFUSED_CELLULAR
}

data class GroupCallState(
    val groupId: String,
    val participants: List<MeshParticipant> = emptyList(),
    val backend: MeshBackend = MeshBackend.PAIRWISE_SESSIONS,
    val connectCueClaimed: Boolean = false
) {
    val totalParticipants: Int get() = participants.size + 1

    fun canAddMore(): Boolean = false

    fun canAddParticipant(
        lookupKey: String,
        keyResolvable: Boolean,
        confirmedUnencrypted: Boolean,
        origin: CallOrigin = CallOrigin.IROH
    ): AddParticipantDecision = AddParticipantDecision.REFUSED_MAX_PARTICIPANTS

    fun aggregateSnapshot(): CallSecuritySnapshot? = null

    fun isEncrypted(): Boolean = false

    fun participantForLeg(telecomCallKey: String): MeshParticipant? = null

    fun withParticipant(participant: MeshParticipant): GroupCallState = this

    fun withoutLeg(telecomCallKey: String): GroupCallState = this

    fun updateLeg(
        telecomCallKey: String,
        transform: (MeshParticipant) -> MeshParticipant
    ): GroupCallState = this

    fun title(): String = ""

    companion object {
        const val MAX_PARTICIPANTS = 4

        fun legKey(groupId: String, lookupKey: String): String = "$groupId/$lookupKey"
    }
}

object GroupCallPolicy {
    fun canFormGroup(origins: Collection<CallOrigin>): Boolean = false

    fun refuseMerge(a: CallOrigin, b: CallOrigin): Boolean = true
}

object ConnectCuePolicy {
    fun shouldPlay(snapshot: CallSecuritySnapshot, irohOrigin: Boolean): Boolean {
        if (!snapshot.playSecureCue && !snapshot.playUnsecureCue) return false
        if (snapshot.playUnsecureCue && !snapshot.playSecureCue &&
            snapshot.padlockState == PadlockDisplayState.KEY_NOT_FOUND &&
            irohOrigin
        ) {
            return false
        }
        return true
    }
}

/** Same trust rule as debug: never confirm without a link fingerprint; roster mismatch rejects. */
object RosterKeyCheck {
    private val FINGERPRINT_HEX = Regex("^[0-9a-f]{64}$")

    private fun normalize(raw: String?): String? =
        raw?.trim()?.lowercase()?.takeIf { FINGERPRINT_HEX.matches(it) }

    private fun same(a: String, b: String): Boolean =
        java.security.MessageDigest.isEqual(
            a.toByteArray(Charsets.US_ASCII),
            b.toByteArray(Charsets.US_ASCII)
        )

    fun evaluate(
        linkFingerprintHex: String?,
        localContactFingerprintHex: String?,
        rosterFingerprintHex: String?
    ): KeyCheck {
        val link = normalize(linkFingerprintHex) ?: return KeyCheck.UNVERIFIED
        val local = normalize(localContactFingerprintHex)
        val roster = normalize(rosterFingerprintHex)
        if (local != null && !same(link, local)) {
            return KeyCheck.KEY_CHANGED
        }
        if (roster != null && !same(link, roster)) {
            return KeyCheck.MISMATCH
        }
        if (local != null) return KeyCheck.CONFIRMED_LOCAL_CONTACT
        if (roster != null) return KeyCheck.CONFIRMED_ROSTER
        return KeyCheck.UNVERIFIED
    }
}
