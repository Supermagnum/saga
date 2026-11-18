package org.saga.call

import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.saga.handshake.SagaHandshakeState

class GroupCallStateTest {
    private val introducedByFmt = "%1\$s, introduced by %2\$s"

    private fun encrypted() = CallSecurityStateResolver.resolve(
        CallOrigin.IROH, handshakeEncrypted = true, contactKeyResolvable = true
    )

    private fun securing() = CallSecurityStateResolver.resolve(
        CallOrigin.IROH, handshakeSecuring = true, contactKeyResolvable = true
    )

    private fun notEncrypted() = CallSecurityStateResolver.resolve(CallOrigin.IROH)

    private fun downgraded() = CallSecurityStateResolver.resolve(
        CallOrigin.IROH, handshakeFailed = true, hasEncryptionHistory = true, contactKeyResolvable = true
    )

    private fun participant(
        lookup: String,
        snapshot: CallSecuritySnapshot = encrypted(),
        keyCheck: KeyCheck = KeyCheck.CONFIRMED_LOCAL_CONTACT,
        groupId: String = "g1",
        introducer: String? = null,
        known: Boolean = true,
        name: String = lookup
    ) = MeshParticipant(
        lookupKey = lookup,
        endpointId = "ep-$lookup",
        displayName = name,
        introducerName = introducer,
        snapshot = snapshot,
        rustSessionId = "sess-$lookup",
        telecomCallKey = GroupCallState.legKey(groupId, lookup),
        introduced = introducer != null,
        knownContact = known,
        keyCheck = keyCheck
    )

    private fun group(vararg p: MeshParticipant) =
        GroupCallState(groupId = "g1", participants = p.toList())

    // --- aggregate padlock ---------------------------------------------------------------

    @Test
    fun aggregate_emptyGroupHasNoSnapshot() {
        assertNull(group().aggregateSnapshot())
        assertFalse(group().isEncrypted())
    }

    @Test
    fun aggregate_allEncrypted_headerEncrypted() {
        val g = group(participant("bob"), participant("alice"), participant("charlene"))
        assertEquals(PadlockDisplayState.ENCRYPTED, g.aggregateSnapshot()!!.padlockState)
        assertTrue(g.isEncrypted())
    }

    @Test
    fun aggregate_oneSecuring_headerNotEncrypted() {
        val g = group(participant("bob"), participant("alice", snapshot = securing()))
        assertEquals(PadlockDisplayState.SECURING, g.aggregateSnapshot()!!.padlockState)
        assertFalse(g.isEncrypted())
    }

    @Test
    fun aggregate_oneNotEncrypted_headerNotEncrypted() {
        val g = group(participant("bob"), participant("thor", snapshot = notEncrypted()))
        assertEquals(PadlockDisplayState.KEY_NOT_FOUND, g.aggregateSnapshot()!!.padlockState)
        assertEquals("Not encrypted", g.aggregateSnapshot()!!.statusText)
        assertFalse(g.isEncrypted())
    }

    @Test
    fun aggregate_worstByPriority_downgradedBeatsNotEncrypted() {
        val g = group(
            participant("bob", snapshot = notEncrypted()),
            participant("alice", snapshot = downgraded()),
            participant("charlene")
        )
        assertEquals(PadlockDisplayState.DOWNGRADED, g.aggregateSnapshot()!!.padlockState)
    }

    @Test
    fun aggregate_neverCarriesCuesOrModal() {
        val g = group(participant("bob", snapshot = downgraded()), participant("alice"))
        val agg = g.aggregateSnapshot()!!
        assertFalse(agg.playSecureCue)
        assertFalse(agg.playUnsecureCue)
        assertFalse(agg.showDowngradeModal)
    }

    @Test
    fun aggregate_encryptedWithoutKeyCheck_isNotEncrypted() {
        // Handshake says Encrypted but the key was never checked: must not show Encrypted.
        val g = group(participant("bob"), participant("alice", keyCheck = KeyCheck.UNVERIFIED))
        assertEquals(PadlockDisplayState.KEY_NOT_FOUND, g.aggregateSnapshot()!!.padlockState)
        assertFalse(g.isEncrypted())
    }

    @Test
    fun aggregate_keyMismatch_neverEncrypted() {
        val p = participant("bob", keyCheck = KeyCheck.MISMATCH, known = false)
        assertEquals(PadlockDisplayState.KEY_NOT_FOUND, p.effectiveSnapshot().padlockState)
        assertEquals(MeshParticipant.NOT_VERIFIED_TEXT, p.effectiveSnapshot().statusText)
        assertFalse(group(p, participant("alice")).isEncrypted())
    }

    // --- canAddParticipant / max 4 -------------------------------------------------------

    @Test
    fun canAdd_firstAddToOneToOne_allowed() {
        // 1:1 = local + 1 remote.
        val g = group(participant("bob"))
        assertEquals(
            AddParticipantDecision.ALLOWED,
            g.canAddParticipant("alice", keyResolvable = true, confirmedUnencrypted = false)
        )
    }

    @Test
    fun canAdd_max4TotalIncludingLocal_fifthRefused() {
        val three = group(participant("bob"), participant("alice"), participant("charlene"))
        assertEquals(4, three.totalParticipants)
        assertFalse(three.canAddMore())
        assertEquals(
            AddParticipantDecision.REFUSED_MAX_PARTICIPANTS,
            three.canAddParticipant("thor", keyResolvable = true, confirmedUnencrypted = true)
        )
    }

    @Test
    fun canAdd_fourthPersonAllowedAtThreeTotal() {
        val two = group(participant("bob"), participant("alice"))
        assertEquals(3, two.totalParticipants)
        assertTrue(two.canAddMore())
        assertEquals(
            AddParticipantDecision.ALLOWED,
            two.canAddParticipant("charlene", keyResolvable = true, confirmedUnencrypted = false)
        )
    }

    @Test
    fun canAdd_duplicateRefused() {
        val g = group(participant("bob"))
        assertEquals(
            AddParticipantDecision.REFUSED_DUPLICATE,
            g.canAddParticipant("bob", keyResolvable = true, confirmedUnencrypted = false)
        )
    }

    @Test
    fun canAdd_noKey_needsExplicitConfirmation() {
        val g = group(participant("bob"))
        assertEquals(
            AddParticipantDecision.NEEDS_UNENCRYPTED_CONFIRMATION,
            g.canAddParticipant("stranger", keyResolvable = false, confirmedUnencrypted = false)
        )
        assertEquals(
            AddParticipantDecision.ALLOWED,
            g.canAddParticipant("stranger", keyResolvable = false, confirmedUnencrypted = true)
        )
    }

    @Test
    fun canAdd_cellularRefused() {
        val g = group(participant("bob"))
        assertEquals(
            AddParticipantDecision.REFUSED_CELLULAR,
            g.canAddParticipant("alice", true, true, origin = CallOrigin.CELLULAR)
        )
    }

    @Test
    fun canAdd_maxCheckedBeforeConfirmation() {
        val full = group(participant("a1"), participant("a2"), participant("a3"))
        assertEquals(
            AddParticipantDecision.REFUSED_MAX_PARTICIPANTS,
            full.canAddParticipant("stranger", keyResolvable = false, confirmedUnencrypted = false)
        )
    }

    // --- unique per-leg keys --------------------------------------------------------------

    @Test
    fun legKey_isUniquePerGroupAndLookup() {
        assertEquals("g1/bob", GroupCallState.legKey("g1", "bob"))
        assertFalse(GroupCallState.legKey("g1", "bob") == GroupCallState.legKey("g2", "bob"))
        assertFalse(GroupCallState.legKey("g1", "bob") == "bob")
    }

    // --- introduced-by display ------------------------------------------------------------

    @Test
    fun introducedBy_unknownContact_showsIntroducer() {
        val p = participant(
            "charlene", introducer = "Alice", known = false, name = "Charlene",
            keyCheck = KeyCheck.CONFIRMED_ROSTER
        )
        assertEquals("Charlene, introduced by Alice", p.displayLabel(introducedByFmt))
        assertFalse(p.needsKeyCheckNote())
    }

    @Test
    fun introducedBy_knownContact_plainName() {
        val p = participant("charlene", introducer = "Alice", known = true, name = "Charlene")
        assertEquals("Charlene", p.displayLabel(introducedByFmt))
    }

    @Test
    fun introducedBy_directlyAdded_plainName() {
        val p = participant("charlene", known = false, name = "Charlene")
        assertEquals("Charlene", p.displayLabel(introducedByFmt))
    }

    @Test
    fun introducedBy_withoutKeyCheck_neverShownVerified() {
        val p = participant(
            "charlene", introducer = "Alice", known = false, name = "Charlene",
            keyCheck = KeyCheck.UNVERIFIED
        )
        assertTrue(p.needsKeyCheckNote())
        // Even if the handshake claims Encrypted, no padlock without a key check.
        assertEquals(PadlockDisplayState.KEY_NOT_FOUND, p.effectiveSnapshot().padlockState)
    }

    // --- roster fingerprint check ---------------------------------------------------------

    private val fpA = "a".repeat(64)
    private val fpB = "b".repeat(64)

    @Test
    fun rosterCheck_unknownContactMatchesRoster_confirmed() {
        assertEquals(KeyCheck.CONFIRMED_ROSTER, RosterKeyCheck.evaluate(fpA, null, fpA))
    }

    @Test
    fun rosterCheck_unknownContactMismatch() {
        assertEquals(KeyCheck.MISMATCH, RosterKeyCheck.evaluate(fpA, null, fpB))
    }

    @Test
    fun rosterCheck_noLinkFingerprint_neverConfirmed() {
        assertEquals(KeyCheck.UNVERIFIED, RosterKeyCheck.evaluate(null, null, fpA))
        assertEquals(KeyCheck.UNVERIFIED, RosterKeyCheck.evaluate(null, fpA, fpA))
        assertEquals(KeyCheck.UNVERIFIED, RosterKeyCheck.evaluate("not-hex", null, "not-hex"))
    }

    @Test
    fun rosterCheck_noRoster_noLocalKey_unverified() {
        assertEquals(KeyCheck.UNVERIFIED, RosterKeyCheck.evaluate(fpA, null, null))
    }

    @Test
    fun rosterCheck_localMatches_rosterMismatch_isMismatch() {
        // G4b: wrong introducer roster claim must reject even when a local contact key matches.
        assertEquals(KeyCheck.MISMATCH, RosterKeyCheck.evaluate(fpA, fpA, fpB))
    }

    @Test
    fun rosterCheck_localMismatch_isKeyChanged() {
        // Contact key disagrees with the link → Key changed (§5a), not roster MISMATCH.
        assertEquals(KeyCheck.KEY_CHANGED, RosterKeyCheck.evaluate(fpA, fpB, fpA))
        assertEquals(KeyCheck.KEY_CHANGED, RosterKeyCheck.evaluate(fpA, fpB, fpB))
    }

    @Test
    fun rosterCheck_localAndRosterBothMatch_confirmedLocal() {
        assertEquals(KeyCheck.CONFIRMED_LOCAL_CONTACT, RosterKeyCheck.evaluate(fpA, fpA, fpA))
    }

    @Test
    fun keyChanged_neverShowsEncrypted() {
        val p = participant("charlene", keyCheck = KeyCheck.KEY_CHANGED, known = true)
        assertEquals("Key changed", p.effectiveSnapshot().statusText)
        assertEquals(PadlockDisplayState.DOWNGRADED, p.effectiveSnapshot().padlockState)
        assertFalse(group(p, participant("alice")).isEncrypted())
    }

    @Test
    fun rosterMismatch_showsNotVerified() {
        val p = participant(
            "charlene", introducer = "Alice", known = false, name = "Charlene",
            keyCheck = KeyCheck.MISMATCH
        )
        assertEquals("Not verified", p.effectiveSnapshot().statusText)
        assertFalse(p.isEncryptedAndConfirmed())
    }

    // --- never merge carrier + Iroh -------------------------------------------------------

    @Test
    fun policy_neverMergesCarrierWithIroh() {
        assertTrue(GroupCallPolicy.refuseMerge(CallOrigin.IROH, CallOrigin.CELLULAR))
        assertTrue(GroupCallPolicy.refuseMerge(CallOrigin.CELLULAR, CallOrigin.IROH))
        assertTrue(GroupCallPolicy.refuseMerge(CallOrigin.CELLULAR, CallOrigin.CELLULAR))
        assertFalse(GroupCallPolicy.refuseMerge(CallOrigin.IROH, CallOrigin.IROH))
    }

    @Test
    fun policy_groupOnlyFromAllIrohLegs() {
        assertTrue(GroupCallPolicy.canFormGroup(listOf(CallOrigin.IROH, CallOrigin.IROH)))
        assertFalse(GroupCallPolicy.canFormGroup(listOf(CallOrigin.IROH, CallOrigin.CELLULAR)))
        assertFalse(GroupCallPolicy.canFormGroup(emptyList()))
    }

    @Test
    fun cellular_resolverStaysNotEncrypted() {
        val s = CallSecurityStateResolver.resolve(CallOrigin.CELLULAR, contactKeyResolvable = true)
        assertEquals(PadlockDisplayState.KEY_NOT_FOUND, s.padlockState)
        assertEquals("Not encrypted", s.statusText)
    }

    // --- connect cue once per conference --------------------------------------------------

    @Before
    @After
    fun resetRegistry() {
        GroupCallRegistry.clearForTest()
    }

    @Test
    fun connectCue_claimedExactlyOncePerGroup() {
        GroupCallRegistry.put(group(participant("bob"), participant("alice")))
        assertTrue(GroupCallRegistry.claimConnectCue("g1"))
        // late joiner / swap / unhold
        assertFalse(GroupCallRegistry.claimConnectCue("g1"))
        assertFalse(GroupCallRegistry.claimConnectCue("g1"))
    }

    @Test
    fun connectCue_unknownGroupNeverClaims() {
        assertFalse(GroupCallRegistry.claimConnectCue("missing"))
    }

    @Test
    fun connectCue_policyKeepsIrohKeyNotFoundSilent() {
        assertFalse(ConnectCuePolicy.shouldPlay(notEncrypted(), irohOrigin = true))
        assertTrue(ConnectCuePolicy.shouldPlay(encrypted(), irohOrigin = true))
        assertTrue(ConnectCuePolicy.shouldPlay(downgraded(), irohOrigin = true))
    }

    // --- registry: per-leg updates, §5a / unencrypted gate --------------------------------

    @Test
    fun registry_legSnapshotUpdatesOnlyThatParticipant() {
        GroupCallRegistry.put(group(participant("bob"), participant("alice")))
        GroupCallRegistry.onLegSnapshot("g1/alice", downgraded())
        val g = GroupCallRegistry.get("g1")!!
        assertEquals(PadlockDisplayState.ENCRYPTED, g.participantForLeg("g1/bob")!!.snapshot.padlockState)
        assertEquals(PadlockDisplayState.DOWNGRADED, g.participantForLeg("g1/alice")!!.snapshot.padlockState)
        assertTrue(g.participantForLeg("g1/alice")!!.effectiveSnapshot().showDowngradeModal)
        assertFalse(g.participantForLeg("g1/bob")!!.effectiveSnapshot().showDowngradeModal)
    }

    @Test
    fun registry_unconfirmedPlainLinkGatesMediaUntilAdderConfirms() {
        GroupCallRegistry.put(group(participant("bob"), participant("thor", snapshot = notEncrypted())))
        assertTrue(GroupCallRegistry.gateUnencryptedMedia("g1/thor", SagaHandshakeState.NeverEncrypted))
        assertTrue(GroupCallRegistry.get("g1")!!.participantForLeg("g1/thor")!!.awaitingUnencryptedConfirmation)

        GroupCallRegistry.confirmUnencrypted("g1", "g1/thor")
        val thor = GroupCallRegistry.get("g1")!!.participantForLeg("g1/thor")!!
        assertFalse(thor.awaitingUnencryptedConfirmation)
        assertTrue(thor.addedUnencryptedConfirmed)
        assertFalse(GroupCallRegistry.gateUnencryptedMedia("g1/thor", SagaHandshakeState.NeverEncrypted))
        // Still not shown as Encrypted.
        assertFalse(GroupCallRegistry.get("g1")!!.isEncrypted())
    }

    @Test
    fun registry_encryptedAndNonGroupLegsNeverGated() {
        GroupCallRegistry.put(group(participant("bob"), participant("alice")))
        assertFalse(GroupCallRegistry.gateUnencryptedMedia("g1/bob", SagaHandshakeState.Encrypted))
        assertFalse(GroupCallRegistry.gateUnencryptedMedia("solo-1to1", SagaHandshakeState.NeverEncrypted))
    }

    @Test
    fun registry_promotedLegKeepsLegacyKeyAndIsFoundByLookup() {
        // Promoted 1:1 keeps its legacy key (the lookup key), not a group-scoped one.
        val promoted = participant("+15550100010").copy(telecomCallKey = "+15550100010")
        GroupCallRegistry.put(group(promoted, participant("alice")))
        assertNotNull(GroupCallRegistry.findLeg("+15550100010"))
        assertNotNull(GroupCallRegistry.findLeg("15550100010"))
    }

    @Test
    fun registry_removeLegKeepsOthers_noHost() {
        GroupCallRegistry.put(group(participant("bob"), participant("alice"), participant("charlene")))
        GroupCallRegistry.update("g1") { it.withoutLeg("g1/bob") }
        val g = GroupCallRegistry.get("g1")!!
        assertEquals(listOf("alice", "charlene"), g.participants.map { it.lookupKey })
        assertTrue(g.canAddMore())
    }

    @Test
    fun title_joinsNames() {
        assertEquals("bob, alice", group(participant("bob"), participant("alice")).title())
    }
}
