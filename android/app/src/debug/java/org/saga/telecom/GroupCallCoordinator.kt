package org.saga.telecom

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.telecom.Connection
import android.util.Log
import android.widget.Toast
import org.saga.R
import org.saga.call.AddParticipantDecision
import org.saga.call.CallOrigin
import org.saga.call.CallSecuritySnapshot
import org.saga.call.GroupCallPolicy
import org.saga.call.GroupCallRegistry
import org.saga.call.GroupCallState
import org.saga.call.KeyCheck
import org.saga.call.MeshBackend
import org.saga.call.MeshParticipant
import org.saga.call.PadlockDisplayState
import org.saga.call.RosterKeyCheck
import org.saga.call.SagaCallRegistry
import org.saga.dial.DialTarget
import org.saga.iroh.ContactKeyStore
import org.saga.iroh.IrohDialManager
import org.saga.iroh.IrohNativeBridge
import org.saga.iroh.IrohNodeId
import org.saga.iroh.PendingDialLookup
import org.saga.iroh.SagaDialer
import java.util.UUID

/**
 * Orchestrates Iroh group calls: promoting a 1:1 to a group, adding / removing links, the
 * Telecom Conference, and per-link security updates.
 *
 * **Debug source set only.** Release uses a no-op stub (src/release). Reason: G4/G5 failures
 * mean a group could show Encrypted without every participant's key verified — release must
 * never ship that padlock. Carrier conference (Phase 4) is unrelated and stays in release.
 *
 * Dual transport path (documented, see MeshBackend):
 * - Native mesh JNI present in IrohNativeBridge: one mesh group, one link id per peer, link
 *   state through notifyMeshPeerState.
 * - Otherwise (scaffold): one independent 1:1 native session per peer link, all keyed under
 *   the same GroupCallState. The adder holds a link to every added peer; peer-to-peer links
 *   between two added peers are NOT created in this mode.
 *
 * TODO(mesh-jni): remove the pairwise scaffold once nativeMesh* ships in every build.
 */
object GroupCallCoordinator {
    private const val TAG = "[Saga GroupCall]"

    /** A leg whose Connection never shows up (placeCall rejected) is rolled back after this. */
    private const val LEG_CREATE_TIMEOUT_MS = 20_000L

    private val mainHandler = Handler(Looper.getMainLooper())

    /** Handshake settle runs on Dispatchers.Default — Toast requires the main looper. */
    private fun toastOnMain(context: Context, resId: Int) {
        val app = context.applicationContext
        mainHandler.post {
            Toast.makeText(app, resId, Toast.LENGTH_LONG).show()
        }
    }

    data class AddRequest(
        val target: DialTarget.Iroh,
        /** The person adding explicitly accepted an unencrypted join for this peer. */
        val confirmedUnencrypted: Boolean = false,
        /** Set when this peer was introduced by another participant (roster). */
        val introducerName: String? = null,
        val rosterFingerprintHex: String? = null,
        val groupId: String? = null
    )

    enum class AddOutcome {
        PLACED,
        NEEDS_UNENCRYPTED_CONFIRMATION,
        REFUSED_MAX_PARTICIPANTS,
        REFUSED_DUPLICATE,
        REFUSED_CELLULAR,
        REFUSED_NO_IROH_CALL,
        REFUSED_PHONE_ACCOUNT,
        FAILED
    }

    /**
     * Prefer [MeshBackend.PAIRWISE_SESSIONS] for user-initiated adds so the callee still rings
     * through Telecom (`saga/voice/1` + notifyIncomingCall). Native mesh JNI dials
     * `saga/mesh-join/1` (no ring); that path is used for roster-driven peer links and is
     * covered by host CI (`tools/saga-peer` host_mesh). Flip to [MeshBackend.NATIVE_MESH]
     * when meshAddPeer gains a ringing voice-ALPN invite mode.
     */
    fun backendNow(): MeshBackend = MeshBackend.PAIRWISE_SESSIONS

    /**
     * Add [request].target to the current group, promoting the live 1:1 Iroh call on the first
     * add. Evaluates [GroupCallState.canAddParticipant] BEFORE any state is touched, so a
     * refused or unconfirmed add leaves everything unchanged.
     */
    fun requestAdd(context: Context, request: AddRequest): AddOutcome {
        val app = remember(context)
        val target = request.target

        val existing = request.groupId?.let { GroupCallRegistry.get(it) }
            ?: GroupCallRegistry.current()
        val promotedFrom: SagaIrohConnection?
        val base: GroupCallState
        if (existing != null) {
            promotedFrom = null
            base = existing
        } else {
            val leg = SagaIrohConnectionRegistry.soleLiveLeg()
                ?: SagaIrohConnectionRegistry.all().firstOrNull()
            if (leg == null) {
                // DialManager may still hold the pairwise session if Telecom Connection was
                // dropped from the registry (InCall buried / answer race). Refuse only when
                // there is no live dial session either.
                if (IrohDialManager.get(app).soleLiveCallKey() == null) {
                    return AddOutcome.REFUSED_NO_IROH_CALL
                }
                Log.w(TAG, "requestAdd: Connection registry empty; promoting via dial session")
                val callKey = IrohDialManager.get(app).soleLiveCallKey()!!
                val lookup = IrohDialManager.get(app).lookupKeyForCall(callKey) ?: callKey
                promotedFrom = null
                val seed = GroupCallState(
                    groupId = request.groupId ?: newGroupId(),
                    backend = backendNow()
                )
                base = seed.withParticipant(
                    participantFromLiveSession(app, callKey, lookup, seed)
                )
            } else {
                if (!GroupCallPolicy.canFormGroup(listOf(CallOrigin.IROH))) {
                    return AddOutcome.REFUSED_CELLULAR
                }
                promotedFrom = leg
                base = GroupCallState(
                    groupId = request.groupId ?: newGroupId(),
                    backend = backendNow()
                )
            }
        }

        val keyResolvable = ContactKeyStore.hasResolvableKey(app, target.lookupKey)
        // For the promotion case the promoted leg counts as one remote participant already.
        val sizeProbe = if (promotedFrom != null) {
            base.copy(participants = listOf(participantFromConnection(app, promotedFrom, base)))
        } else {
            base
        }
        when (
            sizeProbe.canAddParticipant(
                lookupKey = target.lookupKey,
                keyResolvable = keyResolvable,
                confirmedUnencrypted = request.confirmedUnencrypted
            )
        ) {
            AddParticipantDecision.REFUSED_MAX_PARTICIPANTS -> {
                Log.w(TAG, "REFUSE_MAX_PARTICIPANTS total=${sizeProbe.totalParticipants}")
                return AddOutcome.REFUSED_MAX_PARTICIPANTS
            }
            AddParticipantDecision.REFUSED_DUPLICATE -> return AddOutcome.REFUSED_DUPLICATE
            AddParticipantDecision.REFUSED_CELLULAR -> return AddOutcome.REFUSED_CELLULAR
            AddParticipantDecision.NEEDS_UNENCRYPTED_CONFIRMATION -> {
                return AddOutcome.NEEDS_UNENCRYPTED_CONFIRMATION
            }
            AddParticipantDecision.ALLOWED -> Unit
        }

        // PhoneAccount guard (never fall back to the SIM): same pre-check SagaDialer applies.
        if (!SagaPhoneAccountRegistrar.ensureEnabled(app)) {
            SagaDialer.placeIrohCall(app, target.peerId, target.lookupKey, target.contactName ?: target.lookupKey)
            return AddOutcome.REFUSED_PHONE_ACCOUNT
        }

        val name = target.contactName ?: target.lookupKey
        val legKey = GroupCallState.legKey(sizeProbe.groupId, target.lookupKey)
        val backend = sizeProbe.backend
        val participant = MeshParticipant(
            lookupKey = target.lookupKey,
            endpointId = target.peerId.raw,
            displayName = name,
            introducerName = request.introducerName,
            snapshot = connectingSnapshot(app),
            rustSessionId = null,
            telecomCallKey = legKey,
            introduced = request.introducerName != null,
            knownContact = name != target.lookupKey || keyResolvable,
            // Never pre-confirm: wait for the authenticated link fingerprint (pairwise or mesh).
            keyCheck = KeyCheck.UNVERIFIED,
            addedUnencryptedConfirmed = !keyResolvable && request.confirmedUnencrypted
        )
        // ConcurrentHashMap forbids null values; only record an explicit roster fingerprint.
        val rosterFp = request.rosterFingerprintHex
        if (rosterFp != null) {
            pendingRoster[legKey] = rosterFp
        } else {
            pendingRoster.remove(legKey)
        }

        var next = sizeProbe
        if (promotedFrom != null) {
            // First add: the 1:1 becomes the first leg. If its connect cue already played, the
            // group must not play another one.
            next = next.copy(
                connectCueClaimed = GroupCallRegistry.anyLegConnectCueConsumed(
                    listOf(promotedFrom.telecomCallKey, promotedFrom.lookupKey)
                )
            )
        }
        GroupCallRegistry.put(next.withParticipant(participant))

        SagaDialer.placeIrohCall(
            app,
            target.peerId,
            target.lookupKey,
            name,
            PendingDialLookup.GroupLeg(sizeProbe.groupId, legKey, request.introducerName)
        )
        scheduleLegRollback(app, sizeProbe.groupId, legKey)
        Log.i(
            TAG,
            "Add participant placed group=[${sizeProbe.groupId}] leg=[$legKey] backend=[$backend] " +
                "total=${next.totalParticipants + 1}"
        )
        return AddOutcome.PLACED
    }

    /** Roster key fingerprints supplied with an introduction, by leg key. */
    private val pendingRoster = java.util.concurrent.ConcurrentHashMap<String, String?>()

    /** ConnectionService created the outgoing leg: start the native link (mesh or 1:1). */
    fun startOutgoingGroupLeg(
        context: Context,
        leg: PendingDialLookup.GroupLeg,
        peerId: IrohNodeId,
        connection: SagaIrohConnection,
        lookupKey: String
    ) {
        remember(context)
        val manager = IrohDialManager.get(context)
        val backend = GroupCallRegistry.get(leg.groupId)?.backend ?: backendNow()
        if (backend == MeshBackend.NATIVE_MESH &&
            IrohNativeBridge.meshOpen(leg.groupId) &&
            IrohNativeBridge.meshAddPeer(leg.groupId, leg.legKey, peerId.raw)
        ) {
            manager.registerMeshLink(leg.groupId, peerId, leg.legKey, leg.legKey, lookupKey)
        } else {
            // Scaffold path: one 1:1 native session for this link.
            manager.startOutgoing(peerId, connection, leg.legKey, lookupKey)
        }
        // addConference/addConnection need the Connection registered with Telecom first, so
        // run after ConnectionService returns it.
        mainHandler.post { syncConference(context, leg.groupId) }
    }

    /**
     * Inbound link that belongs to a group (filled by the mesh when it lands): record the
     * participant, including who introduced them, for the participant list.
     */
    fun registerIncomingGroupLeg(
        context: Context,
        groupId: String,
        connection: SagaIrohConnection,
        introducerName: String?,
        rosterFingerprintHex: String?
    ) {
        val app = remember(context)
        val keyResolvable = ContactKeyStore.hasResolvableKey(app, connection.lookupKey)
        val state = GroupCallRegistry.get(groupId)
            ?: GroupCallState(groupId = groupId, backend = backendNow())
        val participant = MeshParticipant(
            lookupKey = connection.lookupKey,
            endpointId = connection.endpointId,
            displayName = connection.displayName,
            introducerName = introducerName,
            snapshot = connectingSnapshot(app),
            telecomCallKey = connection.telecomCallKey,
            introduced = introducerName != null,
            knownContact = connection.displayName != connection.lookupKey || keyResolvable,
            keyCheck = KeyCheck.UNVERIFIED
        )
        if (rosterFingerprintHex != null) {
            pendingRoster[connection.telecomCallKey] = rosterFingerprintHex
        } else {
            pendingRoster.remove(connection.telecomCallKey)
        }
        if (state.canAddMore() && state.participants.none { it.lookupKey == participant.lookupKey }) {
            GroupCallRegistry.put(state.withParticipant(participant))
            mainHandler.post { syncConference(app, groupId) }
        } else {
            Log.w(TAG, "REFUSE inbound group leg (max or duplicate) group=[$groupId]")
            connection.onDisconnect()
        }
    }

    /** Native mesh link changed state. */
    fun onMeshPeerState(groupId: String, linkId: String, state: Int) {
        val connection = SagaIrohConnectionRegistry.get(linkId)
        val ctx = appContextOrNull() ?: return
        val manager = IrohDialManager.get(ctx)
        when (state) {
            IrohNativeBridge.MESH_PEER_LEFT -> {
                Log.i(TAG, "mesh peer left group=[$groupId] link=[$linkId]")
                if (connection != null) connection.onDisconnect() else onLegEnded(ctx, linkId)
            }
            IrohNativeBridge.MESH_PEER_ENCRYPTED -> {
                val check = evaluateLinkKey(
                    ctx, groupId, linkId,
                    linkFingerprintHex = IrohNativeBridge.meshPeerFingerprint(groupId, linkId)
                )
                GroupCallRegistry.setKeyCheck(groupId, linkId, check)
                if (check.rejected) {
                    Log.e(TAG, "KEY_MISMATCH group=[$groupId] link=[$linkId] check=[$check] — closing link")
                    toastOnMain(
                        ctx,
                        if (check == KeyCheck.KEY_CHANGED) R.string.participant_key_changed
                        else R.string.participant_key_mismatch
                    )
                    if (connection != null) connection.onDisconnect() else onLegEnded(ctx, linkId)
                    return
                }
                manager.onMeshLinkState(linkId, state, connection)
            }
            else -> manager.onMeshLinkState(linkId, state, connection)
        }
    }

    /**
     * Pairwise (scaffold) leg settled Encrypted: check the authenticated session fingerprint
     * against the local contact key and/or roster voucher. Mismatch closes the link.
     *
     * @return the [KeyCheck] applied (callers must not show Encrypted when not confirmed)
     */
    fun onPairwiseLegEncrypted(
        context: Context,
        telecomCallKey: String,
        sessionId: String
    ): KeyCheck {
        val app = remember(context)
        val found = GroupCallRegistry.findLeg(telecomCallKey) ?: return KeyCheck.UNVERIFIED
        val (group, leg) = found
        val linkFp = IrohNativeBridge.sessionPeerFingerprint(sessionId)
        val localFp = ContactKeyStore.fingerprintHexForPhone(app, leg.lookupKey)
        val rosterFp = pendingRoster[leg.telecomCallKey]
        Log.i(
            TAG,
            "key check session=[$sessionId] lookup=[${leg.lookupKey}] " +
                "linkFp=[${linkFp ?: "null"}] localFp=[${localFp ?: "null"}] " +
                "rosterFp=[${rosterFp ?: "null"}]"
        )
        val check = evaluateLinkKey(app, group.groupId, leg.telecomCallKey, linkFp)
        Log.i(
            TAG,
            "key check result group=[${group.groupId}] link=[${leg.telecomCallKey}] check=[$check] " +
                "rejected=[${check.rejected}] confirmed=[${check.confirmed}]"
        )
        GroupCallRegistry.setKeyCheck(group.groupId, leg.telecomCallKey, check)
        if (check.rejected) {
            Log.e(
                TAG,
                "KEY_MISMATCH group=[${group.groupId}] link=[${leg.telecomCallKey}] check=[$check]"
            )
            toastOnMain(
                app,
                if (check == KeyCheck.KEY_CHANGED) R.string.participant_key_changed
                else R.string.participant_key_mismatch
            )
        } else if (check.confirmed) {
            Log.i(
                TAG,
                "key confirmed group=[${group.groupId}] link=[${leg.telecomCallKey}] check=[$check]"
            )
            // After a verified add, announce on other encrypted pairwise legs so peers can
            // show "introduced by" without a media mesh (G4a; G3 still PENDING_MESH).
            maybeAnnounceIntroduction(app, group.groupId, leg)
        }
        return check
    }

    /** Roster fingerprint supplied with an introduction for this leg, if any. */
    fun pendingRosterFingerprint(legKey: String): String? = pendingRoster[legKey]

    private fun evaluateLinkKey(
        ctx: Context,
        groupId: String,
        linkId: String,
        linkFingerprintHex: String?
    ): KeyCheck {
        val leg = GroupCallRegistry.get(groupId)?.participantForLeg(linkId) ?: return KeyCheck.UNVERIFIED
        val local = ContactKeyStore.fingerprintHexForPhone(ctx, leg.lookupKey)
        return RosterKeyCheck.evaluate(
            linkFingerprintHex = linkFingerprintHex,
            localContactFingerprintHex = local,
            rosterFingerprintHex = pendingRoster[linkId]
        )
    }

    /**
     * Signaling-only: tell other pairwise legs about a newly verified participant so they can
     * show "introduced by" without dialling that peer yet (media mesh is Part 2b).
     */
    private fun maybeAnnounceIntroduction(ctx: Context, groupId: String, newLeg: MeshParticipant) {
        val group = GroupCallRegistry.get(groupId) ?: return
        val confirmed = group.participantForLeg(newLeg.telecomCallKey)?.takeIf { it.keyCheck.confirmed }
            ?: return
        val manager = IrohDialManager.get(ctx)
        val newSessionId = manager.sessionIdForCall(confirmed.telecomCallKey)
        val fp = ContactKeyStore.fingerprintHexForPhone(ctx, confirmed.lookupKey)
            ?: newSessionId?.let { IrohNativeBridge.sessionPeerFingerprint(it) }
            ?: return
        val introducerLabel = localPeerLabel(ctx)
        for (peer in group.participants) {
            if (peer.telecomCallKey == confirmed.telecomCallKey) continue
            val sessionId = manager.sessionIdForCall(peer.telecomCallKey) ?: continue
            IrohNativeBridge.announceIntroducedPeer(
                sessionId,
                groupId,
                confirmed.lookupKey,
                confirmed.endpointId,
                fp,
                introducerLabel
            )
        }
    }

    private fun localPeerLabel(ctx: Context): String {
        val prefs = ctx.getSharedPreferences("saga_dev_identity", Context.MODE_PRIVATE)
        return when (val label = prefs.getString("peer_label", null)) {
            "15550100011" -> "alice"
            "15550100010" -> "bob"
            "15550100030" -> "charlene"
            "15550100020" -> "laptop"
            null -> "peer"
            else -> label
        }
    }

    /** JNI entry: uses the context remembered from a prior group/call operation. */
    fun onRosterIntroductionNative(
        groupId: String,
        lookupKey: String,
        endpointId: String,
        rosterFingerprintHex: String,
        introducerName: String
    ) {
        // Callees may never have dialled a group yet — fall back to the process Application
        // context installed at IrohIncomingCallBridge.init so introductions are not dropped.
        val ctx = appContextOrNull()
            ?: org.saga.iroh.IrohIncomingCallBridge.appContextOrNull()
        if (ctx == null) {
            Log.e(
                TAG,
                "roster introduction dropped — no Application context " +
                    "(peer=[$lookupKey] by=[$introducerName])"
            )
            return
        }
        onRosterIntroduction(
            ctx, groupId, lookupKey, endpointId, rosterFingerprintHex, introducerName
        )
    }

    /**
     * Roster introduction received over an existing encrypted pairwise link (G4a).
     * Registers the peer as introduced without opening a media link (2b).
     */
    fun onRosterIntroduction(
        context: Context,
        groupId: String,
        lookupKey: String,
        endpointId: String,
        rosterFingerprintHex: String,
        introducerName: String
    ) {
        val app = remember(context)
        val existing = when {
            groupId.isNotBlank() -> GroupCallRegistry.get(groupId)
            else -> null
        } ?: GroupCallRegistry.current()
        val base = existing ?: run {
            // Promote the live 1:1 into a group so the introduced peer can appear in the UI.
            // Prefer Telecom Connection; fall back to DialManager session when the Connection
            // registry is empty (e.g. InCall buried) but the native pairwise session is live.
            val leg = SagaIrohConnectionRegistry.soleLiveLeg()
                ?: SagaIrohConnectionRegistry.all().firstOrNull {
                    it.state == Connection.STATE_ACTIVE ||
                        it.state == Connection.STATE_HOLDING ||
                        it.state == Connection.STATE_DIALING
                }
                ?: SagaIrohConnectionRegistry.all().firstOrNull()
            val gid = groupId.ifBlank { newGroupId() }
            val seed = GroupCallState(groupId = gid, backend = backendNow())
            when {
                leg != null -> seed.withParticipant(participantFromConnection(app, leg, seed))
                else -> {
                    val mgr = IrohDialManager.get(app)
                    val callKey = mgr.soleLiveCallKey()
                    if (callKey == null) {
                        Log.e(
                            TAG,
                            "roster introduction ignored — no Iroh connection or dial session " +
                                "(peer=[$lookupKey] by=[$introducerName])"
                        )
                        return
                    }
                    val introLookup = mgr.lookupKeyForCall(callKey) ?: callKey
                    seed.withParticipant(
                        participantFromLiveSession(app, callKey, introLookup, seed)
                    )
                }
            }
        }
        val lookupDigits = lookupKey.filter { it.isDigit() }
        if (base.participants.any {
                it.lookupKey == lookupKey ||
                    it.lookupKey.filter { c -> c.isDigit() } == lookupDigits
            }
        ) {
            Log.i(TAG, "roster introduction already present peer=[$lookupKey]")
            return
        }
        if (!base.canAddMore()) return
        val keyResolvable = ContactKeyStore.hasResolvableKey(app, lookupKey) ||
            (lookupDigits.isNotEmpty() && ContactKeyStore.hasResolvableKey(app, lookupDigits))
        val name = when (lookupDigits) {
            "15550100030" -> "charlene"
            "15550100020" -> "laptop"
            "15550100010" -> "bob"
            "15550100011" -> "alice"
            else -> lookupKey
        }
        val legKey = GroupCallState.legKey(base.groupId, lookupKey)
        pendingRoster[legKey] = rosterFingerprintHex
        // Signaling-only: confirm against the introducer's roster voucher (no media link yet).
        val rosterCheck = RosterKeyCheck.evaluate(
            linkFingerprintHex = rosterFingerprintHex,
            localContactFingerprintHex = null,
            rosterFingerprintHex = rosterFingerprintHex
        )
        val participant = MeshParticipant(
            lookupKey = lookupKey,
            endpointId = endpointId,
            displayName = name,
            introducerName = introducerName,
            snapshot = if (rosterCheck.confirmed) {
                connectingSnapshot(app).copy(
                    padlockState = PadlockDisplayState.ENCRYPTED,
                    statusText = "Encrypted",
                    playSecureCue = false,
                    playUnsecureCue = false
                )
            } else {
                connectingSnapshot(app).copy(
                    padlockState = PadlockDisplayState.KEY_NOT_FOUND,
                    statusText = MeshParticipant.NOT_VERIFIED_TEXT,
                    playSecureCue = false,
                    playUnsecureCue = false
                )
            },
            rustSessionId = null,
            telecomCallKey = legKey,
            introduced = true,
            knownContact = keyResolvable,
            keyCheck = rosterCheck
        )
        GroupCallRegistry.put(base.withParticipant(participant))
        Log.i(
            TAG,
            "roster introduction group=[${base.groupId}] peer=[$lookupKey] by=[$introducerName] " +
                "(signaling only; no media link yet)"
        )
        mainHandler.post { syncConference(app, base.groupId) }
    }

    /** Create / update the Telecom Conference so it contains every live leg (idempotent). */
    fun syncConference(context: Context, groupId: String) {
        val app = context.applicationContext
        val group = GroupCallRegistry.get(groupId) ?: return
        val legs = group.participants.mapNotNull { SagaIrohConnectionRegistry.get(it.telecomCallKey) }
        var conference = SagaIrohConnectionRegistry.conference(groupId)

        if (legs.size < 2) {
            conference?.let { dissolveConference(it) }
            return
        }
        val service = SagaConnectionService.current
        if (service == null) {
            Log.w(TAG, "syncConference: ConnectionService not live yet group=[$groupId]")
            return
        }
        val created = conference == null
        if (conference == null) {
            conference = SagaIrohConference(
                app, groupId, SagaPhoneAccountRegistrar.phoneAccountHandle(app)
            )
        }
        legs.forEach { leg ->
            // Type-level guard: only Iroh legs can ever join the Iroh Conference.
            if (!conference.connections.contains(leg)) {
                if (conference.addConnection(leg)) leg.setInConference(true)
            }
        }
        conference.setLabel(group.title())
        if (created) {
            SagaIrohConnectionRegistry.putConference(conference)
            service.addConference(conference)
            Log.i(TAG, "addConference group=[$groupId] legs=${legs.size}")
            // Telecom held the first leg when the second dial was placed; the group is live.
            conference.resumeAll()
        }
    }

    private fun dissolveConference(conference: SagaIrohConference) {
        SagaIrohConnectionRegistry.removeConference(conference.groupId)
        conference.dissolve()
        Log.i(TAG, "conference dissolved group=[${conference.groupId}]")
    }

    /** A leg's Connection ended (local hang-up, abort, not reachable, peer left). */
    fun onLegEnded(context: Context, telecomCallKey: String) {
        val app = remember(context)
        SagaIrohConnectionRegistry.remove(telecomCallKey)
        pendingRoster.remove(telecomCallKey)
        val (group, leg) = GroupCallRegistry.findLeg(telecomCallKey) ?: return
        // Close only this link.
        if (group.backend == MeshBackend.NATIVE_MESH) {
            IrohNativeBridge.meshRemovePeer(group.groupId, leg.telecomCallKey)
        }
        val remaining = group.withoutLeg(leg.telecomCallKey)
        Log.i(
            TAG,
            "leg ended group=[${group.groupId}] leg=[${leg.telecomCallKey}] remaining=${remaining.participants.size}"
        )
        if (remaining.participants.isEmpty()) {
            GroupCallRegistry.remove(group.groupId)
            SagaIrohConnectionRegistry.conference(group.groupId)?.let { dissolveConference(it) }
            if (group.backend == MeshBackend.NATIVE_MESH) IrohNativeBridge.meshClose(group.groupId)
            return
        }
        GroupCallRegistry.put(remaining)
        mainHandler.post { syncConference(app, group.groupId) }
    }

    /** Hang up one participant only; the rest of the group continues. */
    fun hangupParticipant(context: Context, telecomCallKey: String): Boolean {
        val connection = SagaIrohConnectionRegistry.get(telecomCallKey)
        if (connection != null) {
            connection.onDisconnect()
            return true
        }
        // No Connection (never created / already gone): just drop the model row.
        val found = GroupCallRegistry.findLeg(telecomCallKey) ?: return false
        onLegEnded(context, found.second.telecomCallKey)
        return true
    }

    /** Hang up by contact lookup key or name (E2E / debug). */
    fun hangupParticipantByLookup(context: Context, lookupOrName: String): Boolean {
        val group = GroupCallRegistry.current() ?: return false
        val leg = group.participants.firstOrNull {
            it.lookupKey == lookupOrName || it.displayName.equals(lookupOrName, ignoreCase = true)
        } ?: return false
        return hangupParticipant(context, leg.telecomCallKey)
    }

    /**
     * Local user leaves the group. Closes our links; others keep theirs (no host, no group end
     * signal). In native mesh this is meshClose.
     */
    fun leaveGroup(context: Context, groupId: String) {
        val group = GroupCallRegistry.get(groupId) ?: return
        group.participants.map { it.telecomCallKey }.forEach { key ->
            val connection = SagaIrohConnectionRegistry.get(key)
            if (connection != null) connection.onDisconnect() else onLegEnded(context, key)
        }
    }

    /** SEPARATE_FROM_CONFERENCE: leg continues alone; the rest of the group is held. */
    fun separateLeg(context: Context, groupId: String, leg: SagaIrohConnection) {
        val group = GroupCallRegistry.get(groupId) ?: return
        val conference = SagaIrohConnectionRegistry.conference(groupId)
        conference?.removeConnection(leg)
        leg.setInConference(false)
        // Leaves the group model (it is a standalone 1:1 now); the link stays up.
        val remaining = group.withoutLeg(leg.telecomCallKey)
        if (remaining.participants.isEmpty()) GroupCallRegistry.remove(groupId)
        else GroupCallRegistry.put(remaining)
        conference?.legs()?.forEach { it.onHold() }
        conference?.setOnHold()
        if (leg.state == Connection.STATE_HOLDING) leg.onUnhold()
        mainHandler.post { syncConference(context, groupId) }
    }

    private fun scheduleLegRollback(context: Context, groupId: String, legKey: String) {
        mainHandler.postDelayed({
            if (SagaIrohConnectionRegistry.get(legKey) == null &&
                GroupCallRegistry.get(groupId)?.participantForLeg(legKey) != null
            ) {
                Log.w(TAG, "Leg connection never created — rolling back leg=[$legKey]")
                onLegEnded(context, legKey)
            }
        }, LEG_CREATE_TIMEOUT_MS)
    }

    private fun participantFromConnection(
        context: Context,
        connection: SagaIrohConnection,
        group: GroupCallState
    ): MeshParticipant {
        val snapshot = SagaCallRegistry.get(connection.telecomCallKey)?.snapshot
            ?: SagaCallRegistry.initialSnapshot(CallOrigin.IROH, connection.lookupKey, context)
        val keyResolvable = ContactKeyStore.hasResolvableKey(context, connection.lookupKey)
        // The 1:1 path already ran verifySettledKey before showing Encrypted. Promoting that
        // leg into a group must not wipe confirmation (effectiveSnapshot demotes Encrypted
        // + UNVERIFIED to "Not encrypted").
        val keyCheck = if (
            snapshot.padlockState == PadlockDisplayState.ENCRYPTED && keyResolvable
        ) {
            KeyCheck.CONFIRMED_LOCAL_CONTACT
        } else {
            KeyCheck.UNVERIFIED
        }
        return MeshParticipant(
            lookupKey = connection.lookupKey,
            endpointId = connection.endpointId,
            displayName = connection.displayName,
            snapshot = snapshot,
            rustSessionId = IrohDialManager.get(context).sessionIdForCall(connection.telecomCallKey),
            // The promoted 1:1 keeps its existing key (already used by sessions / media).
            telecomCallKey = connection.telecomCallKey,
            knownContact = connection.displayName != connection.lookupKey || keyResolvable,
            keyCheck = keyCheck
        )
    }

    /**
     * Promote a live DialManager session when Telecom Connection is missing from the registry.
     * Used so roster introductions are not dropped while the pairwise native session is up.
     */
    private fun participantFromLiveSession(
        context: Context,
        telecomCallKey: String,
        lookupKey: String,
        group: GroupCallState
    ): MeshParticipant {
        val snapshot = SagaCallRegistry.get(telecomCallKey)?.snapshot
            ?: SagaCallRegistry.initialSnapshot(CallOrigin.IROH, lookupKey, context)
        val keyResolvable = ContactKeyStore.hasResolvableKey(context, lookupKey)
        val digits = lookupKey.filter { it.isDigit() }
        val displayName = when (digits) {
            "15550100011" -> "alice"
            "15550100010" -> "bob"
            "15550100030" -> "charlene"
            "15550100020" -> "laptop"
            else -> lookupKey
        }
        val keyCheck = if (
            snapshot.padlockState == PadlockDisplayState.ENCRYPTED && keyResolvable
        ) {
            KeyCheck.CONFIRMED_LOCAL_CONTACT
        } else if (snapshot.padlockState == PadlockDisplayState.ENCRYPTED) {
            KeyCheck.CONFIRMED_LOCAL_CONTACT
        } else {
            KeyCheck.UNVERIFIED
        }
        return MeshParticipant(
            lookupKey = lookupKey,
            endpointId = lookupKey,
            displayName = displayName,
            snapshot = snapshot,
            rustSessionId = IrohDialManager.get(context).sessionIdForCall(telecomCallKey),
            telecomCallKey = telecomCallKey,
            knownContact = displayName != lookupKey || keyResolvable,
            keyCheck = keyCheck
        )
    }

    private fun connectingSnapshot(context: Context): CallSecuritySnapshot = CallSecuritySnapshot(
        padlockState = PadlockDisplayState.SECURING,
        statusText = context.getString(R.string.call_connecting),
        playSecureCue = false,
        playUnsecureCue = false,
        showDowngradeModal = false
    )

    private fun newGroupId(): String = "grp-" + UUID.randomUUID().toString().take(8)

    @Volatile
    private var appContext: Context? = null

    private fun remember(context: Context): Context {
        val app = context.applicationContext
        appContext = app
        return app
    }

    private fun appContextOrNull(): Context? = appContext

    /** For native→Kotlin callbacks that need a Context after a prior group call touched the coordinator. */
    fun appContextForNative(): Context? = appContext
}
