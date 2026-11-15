package org.saga.call

import java.security.MessageDigest

/**
 * Pure (JVM-testable) model for Iroh multi-peer encrypted group calls.
 *
 * Decisions (fixed):
 * - Full mesh, no host: when the starter leaves, the call continues for the others.
 * - Anyone in the call may add participants; at most [GroupCallState.MAX_PARTICIPANTS]
 *   people in total, including the local user (so at most 3 remote links).
 * - A participant whose link cannot be encrypted may join only after the person adding
 *   them explicitly confirms. They are marked "Not encrypted" on every screen and the
 *   group is never shown as Encrypted while such a link exists.
 */

/**
 * How the per-peer links of a group are carried.
 *
 * TODO(mesh-jni): when the native mesh JNI (nativeMesh* / notifyMeshPeerState in
 * IrohNativeBridge) lands everywhere, drop [PAIRWISE_SESSIONS] and use [NATIVE_MESH]
 * unconditionally.
 */
enum class MeshBackend {
    /** Native mesh JNI present: one mesh group, one link id per peer. */
    NATIVE_MESH,

    /** Scaffold: one independent 1:1 native session per peer link, keyed under the group. */
    PAIRWISE_SESSIONS
}

/** Result of checking a link's key against what is known locally / vouched by the introducer. */
enum class KeyCheck {
    /** No key check has happened (or no key material was available). Never shown as verified. */
    UNVERIFIED,

    /** Link key equals the key stored for this contact in Contacts. */
    CONFIRMED_LOCAL_CONTACT,

    /** Unknown contact: link key equals the fingerprint the introducer put in the roster. */
    CONFIRMED_ROSTER,

    /** Link key contradicts the local contact key or the roster fingerprint. Link must not run. */
    MISMATCH;

    val confirmed: Boolean
        get() = this == CONFIRMED_LOCAL_CONTACT || this == CONFIRMED_ROSTER
}

/** One remote person in a group call (one link of the mesh). */
data class MeshParticipant(
    /** Contact lookup key (phone-digit label / content lookup) used for key resolution. */
    val lookupKey: String,
    /** Iroh endpoint id the link dials / is connected to. */
    val endpointId: String,
    val displayName: String,
    /** Name of the participant who introduced this peer (null when we added them directly). */
    val introducerName: String? = null,
    /** Raw handshake snapshot for this link (before key-check enforcement). */
    val snapshot: CallSecuritySnapshot,
    /** Native session id (pairwise scaffold) or mesh link id (native mesh). */
    val rustSessionId: String? = null,
    /** Unique per leg (group id + lookup key), never just the lookup key. */
    val telecomCallKey: String,
    /** True when this peer was introduced by another participant (roster), not dialled by us. */
    val introduced: Boolean = false,
    /** True when the peer resolves to a saved contact on this device. */
    val knownContact: Boolean = true,
    val keyCheck: KeyCheck = KeyCheck.UNVERIFIED,
    /** The person adding them explicitly accepted that this link is not encrypted. */
    val addedUnencryptedConfirmed: Boolean = false,
    /** Link settled without encryption and nobody has confirmed it yet: media stays off. */
    val awaitingUnencryptedConfirmation: Boolean = false
) {
    /**
     * The snapshot the UI must render. Encrypted is only ever shown when the link key was
     * actually checked ([KeyCheck.confirmed]); a mismatch is never shown as Encrypted.
     */
    fun effectiveSnapshot(): CallSecuritySnapshot {
        val encryptedWithoutCheck =
            snapshot.padlockState == PadlockDisplayState.ENCRYPTED && !keyCheck.confirmed
        if (keyCheck == KeyCheck.MISMATCH || encryptedWithoutCheck) {
            return notEncryptedSnapshot()
        }
        return snapshot
    }

    fun isEncryptedAndConfirmed(): Boolean =
        effectiveSnapshot().padlockState == PadlockDisplayState.ENCRYPTED

    /** Introduced-by suffix only for unknown contacts that someone else introduced. */
    fun showsIntroducedBy(): Boolean =
        introduced && !knownContact && !introducerName.isNullOrBlank()

    /**
     * "Charlene, introduced by Alice" for unknown contacts introduced by a roster peer.
     * [introducedByFmt] is the localized `introduced_by_fmt` ("%1$s, introduced by %2$s").
     */
    fun displayLabel(introducedByFmt: String): String =
        if (showsIntroducedBy()) {
            String.format(introducedByFmt, displayName, introducerName)
        } else {
            displayName
        }

    /** Introduced unknown peer whose key was not checked against the roster fingerprint. */
    fun needsKeyCheckNote(): Boolean = showsIntroducedBy() && !keyCheck.confirmed

    private fun notEncryptedSnapshot(): CallSecuritySnapshot = CallSecuritySnapshot(
        padlockState = PadlockDisplayState.KEY_NOT_FOUND,
        statusText = NOT_ENCRYPTED_TEXT,
        playSecureCue = false,
        playUnsecureCue = false,
        showDowngradeModal = false
    )

    companion object {
        const val NOT_ENCRYPTED_TEXT = "Not encrypted"
    }
}

/** Outcome of [GroupCallState.canAddParticipant]. */
enum class AddParticipantDecision {
    ALLOWED,

    /** Link cannot be encrypted (no resolvable key); the adder must confirm first. */
    NEEDS_UNENCRYPTED_CONFIRMATION,

    /** Already four people (including the local user). */
    REFUSED_MAX_PARTICIPANTS,

    REFUSED_DUPLICATE,

    /** Carrier legs never join an Iroh group. */
    REFUSED_CELLULAR
}

data class GroupCallState(
    val groupId: String,
    val participants: List<MeshParticipant> = emptyList(),
    val backend: MeshBackend = MeshBackend.PAIRWISE_SESSIONS,
    /** One connect cue per conference: set when the first link settles and plays the cue. */
    val connectCueClaimed: Boolean = false
) {
    /** Remote participants plus the local user. */
    val totalParticipants: Int get() = participants.size + 1

    fun canAddMore(): Boolean = totalParticipants < MAX_PARTICIPANTS

    /**
     * Decide whether [lookupKey] may be added right now.
     *
     * @param keyResolvable the contact has a resolvable Saga key (link can be encrypted)
     * @param confirmedUnencrypted the person adding explicitly confirmed an unencrypted join
     */
    fun canAddParticipant(
        lookupKey: String,
        keyResolvable: Boolean,
        confirmedUnencrypted: Boolean,
        origin: CallOrigin = CallOrigin.IROH
    ): AddParticipantDecision {
        if (origin != CallOrigin.IROH) return AddParticipantDecision.REFUSED_CELLULAR
        if (!canAddMore()) return AddParticipantDecision.REFUSED_MAX_PARTICIPANTS
        if (participants.any { it.lookupKey == lookupKey }) {
            return AddParticipantDecision.REFUSED_DUPLICATE
        }
        if (!keyResolvable && !confirmedUnencrypted) {
            return AddParticipantDecision.NEEDS_UNENCRYPTED_CONFIRMATION
        }
        return AddParticipantDecision.ALLOWED
    }

    /**
     * Group header snapshot: the worst child by [CallSecurityStateResolver.displayPriority].
     * Encrypted only when every child is Encrypted (and confirmed). Cues and the downgrade
     * modal are always off here; they are per link / once per conference elsewhere.
     */
    fun aggregateSnapshot(): CallSecuritySnapshot? {
        if (participants.isEmpty()) return null
        val effective = participants.map { it.effectiveSnapshot() }
        val worst = effective.minByOrNull {
            CallSecurityStateResolver.displayPriority(it.padlockState)
        } ?: return null
        return worst.copy(
            playSecureCue = false,
            playUnsecureCue = false,
            showDowngradeModal = false
        )
    }

    /** True only when every link is encrypted and its key confirmed. */
    fun isEncrypted(): Boolean =
        participants.isNotEmpty() && participants.all { it.isEncryptedAndConfirmed() }

    fun participantForLeg(telecomCallKey: String): MeshParticipant? =
        participants.firstOrNull { it.telecomCallKey == telecomCallKey }

    fun withParticipant(participant: MeshParticipant): GroupCallState =
        copy(participants = participants + participant)

    fun withoutLeg(telecomCallKey: String): GroupCallState =
        copy(participants = participants.filterNot { it.telecomCallKey == telecomCallKey })

    fun updateLeg(
        telecomCallKey: String,
        transform: (MeshParticipant) -> MeshParticipant
    ): GroupCallState = copy(
        participants = participants.map {
            if (it.telecomCallKey == telecomCallKey) transform(it) else it
        }
    )

    /** Header title: participant names, e.g. "bob, alice". */
    fun title(): String = participants.joinToString(", ") { it.displayName }

    companion object {
        /** People in a group call including the local user. */
        const val MAX_PARTICIPANTS = 4

        /** Unique per leg: the same contact can never share a key with another leg or a 1:1. */
        fun legKey(groupId: String, lookupKey: String): String = "$groupId/$lookupKey"
    }
}

/** Never merge a carrier call and an Iroh group (mirrors MultiCallActions.refuseMergeIfMixed). */
object GroupCallPolicy {
    /** A group may be formed only when every leg is an Iroh leg. */
    fun canFormGroup(origins: Collection<CallOrigin>): Boolean =
        origins.isNotEmpty() && origins.all { it == CallOrigin.IROH }

    /** True when merging [a] with [b] must be refused (any carrier leg involved). */
    fun refuseMerge(a: CallOrigin, b: CallOrigin): Boolean =
        !canFormGroup(listOf(a, b))
}

/**
 * Same connect-cue predicate the 1:1 InCall screen applies: an Iroh call with no key
 * (KEY_NOT_FOUND) stays silent, everything else with a cue flag plays once.
 */
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

/** Link key vs. local contact key vs. introducer roster fingerprint (64 lowercase hex). */
object RosterKeyCheck {
    private val FINGERPRINT_HEX = Regex("^[0-9a-f]{64}$")

    private fun normalize(raw: String?): String? =
        raw?.trim()?.lowercase()?.takeIf { FINGERPRINT_HEX.matches(it) }

    private fun same(a: String, b: String): Boolean =
        MessageDigest.isEqual(a.toByteArray(Charsets.US_ASCII), b.toByteArray(Charsets.US_ASCII))

    /**
     * @param linkFingerprintHex fingerprint authenticated by the link handshake (null = unknown)
     * @param localContactFingerprintHex key stored for this contact in Contacts (null = unknown contact)
     * @param rosterFingerprintHex fingerprint the introducer vouches for (null = not introduced)
     *
     * A saved contact key always wins over the roster. Without an authenticated link
     * fingerprint nothing is ever confirmed.
     */
    fun evaluate(
        linkFingerprintHex: String?,
        localContactFingerprintHex: String?,
        rosterFingerprintHex: String?
    ): KeyCheck {
        val link = normalize(linkFingerprintHex) ?: return KeyCheck.UNVERIFIED
        val local = normalize(localContactFingerprintHex)
        if (local != null) {
            return if (same(link, local)) KeyCheck.CONFIRMED_LOCAL_CONTACT else KeyCheck.MISMATCH
        }
        val roster = normalize(rosterFingerprintHex) ?: return KeyCheck.UNVERIFIED
        return if (same(link, roster)) KeyCheck.CONFIRMED_ROSTER else KeyCheck.MISMATCH
    }
}
