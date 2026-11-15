# Conference / multi-call UI

Design note for the InCall multi-call surface and Iroh mesh group calls.

Two independent conference paths exist and never mix:

- **Carrier conference** (cleartext stock parity): Telecom merge of carrier calls.
- **Iroh mesh group** (encrypted, per-link): built in Kotlin through `SagaIrohConference`
  and "Add participant". Part 1 (Android / Telecom UI) is implemented; native mesh media is
  tracked at the end of this note.

## Fixed decisions (Iroh group)

1. **No host.** Full mesh. When the starter leaves, the call continues for the others.
   "End call" in the InCall UI means the local user leaves; it closes the local links only
   and never signals the others to end.
2. **Anyone in the call may add participants**, up to 4 people in total **including the
   local user** (`GroupCallState.MAX_PARTICIPANTS = 4`, so at most 3 remote links). The 5th is
   refused with a dialog ("Call is full ... at most 4 people, including you").
3. **A participant whose link cannot be encrypted** may join only after explicit
   confirmation by the person adding them. They are marked "Not encrypted" on every screen
   and the group is never shown as Encrypted while that link exists.

## Current surface

`SagaInCallActivity` + `SagaInCallService` host one InCall composition for:

| Role | Source |
|------|--------|
| Active | `Call.STATE_ACTIVE` / dialing / connecting |
| Held | `Call.STATE_HOLDING` |
| Ringing (second) | `Call.STATE_RINGING` while another call exists |
| Conference | Telecom conference parent + `getChildren()` (carrier or Iroh) |

Actions are shown only when `Call.Details.can(...)` / `getConferenceableCalls()` allow them
(`MultiCallActions`).

## Origin rules (enforced)

| Action | Carrier | Iroh | Mixed Iroh+carrier |
|--------|---------|------|--------------------|
| Hold / unhold / swap | yes | yes | yes (Iroh media pause/resume on `STATE_HOLDING`) |
| Merge (Telecom `conference()`) | yes (cleartext stock parity) | not offered; Iroh groups use Add participant | **never**: Merge hidden; `conference()` / `mergeConference()` refused with log; `SagaConnectionService.onConference` refuses |
| Add participant | never offered | yes (Iroh contacts only) | never |
| Padlock / section 5a | always KEY_NOT_FOUND / "Not encrypted" | normal handshake path, per link | unchanged per call; no connect cue on swap/unhold/merge |

Carrier calls always resolve via `CallOrigin.CELLULAR`. `SagaIrohConference` accepts only
`SagaIrohConnection` children (type-level guard in `GroupCallCoordinator.syncConference`), and
`GroupCallPolicy` / `GroupCallState.canAddParticipant` refuse any non-Iroh origin.

## What was built (Iroh mesh Conference path)

### Data model (`org.saga.call`, JVM-testable)

- `MeshParticipant`: `lookupKey`, `endpointId`, `displayName`, `introducerName?`,
  `snapshot` (`CallSecuritySnapshot`), `rustSessionId` (native session id in the scaffold,
  mesh link id with native mesh), unique `telecomCallKey`, `introduced` flag, `knownContact`,
  `keyCheck`, `addedUnencryptedConfirmed`, `awaitingUnencryptedConfirmation`.
- `GroupCallState`: `groupId`, participants, `MeshBackend`, `connectCueClaimed`.
  - `aggregateSnapshot()` = worst child by `CallSecurityStateResolver.displayPriority`;
    `isEncrypted()` is true only when every child is ENCRYPTED **and** its key is confirmed.
    Aggregate carries no cues and no downgrade modal.
  - `canAddParticipant(...)` returns `ALLOWED`, `NEEDS_UNENCRYPTED_CONFIRMATION`,
    `REFUSED_MAX_PARTICIPANTS`, `REFUSED_DUPLICATE`, `REFUSED_CELLULAR`. Max is checked first.
  - `legKey(groupId, lookupKey)`: unique `telecomCallKey` per leg (`"<groupId>/<lookupKey>"`).
    A promoted 1:1 leg keeps its legacy key because sessions and media already use it.
- `KeyCheck` + `RosterKeyCheck`: link fingerprint vs. saved contact key vs. introducer roster
  fingerprint. A saved contact key always wins. No authenticated link fingerprint means
  `UNVERIFIED`; `UNVERIFIED` and `MISMATCH` are never rendered as Encrypted
  (`MeshParticipant.effectiveSnapshot()`), and a mismatch closes the link.
- `GroupCallRegistry`: single process-wide group (StateFlow), per-leg snapshot updates
  (hooked from `SagaCallRegistry.updateSnapshot`), `claimConnectCue` (true once per group),
  `gateUnencryptedMedia` / `confirmUnencrypted`.

### Telecom (`org.saga.telecom`)

- `SagaIrohConference : Conference` with `CAPABILITY_MANAGE_CONFERENCE`, `HOLD`,
  `SUPPORT_HOLD`. Hold/unhold apply to every leg. `onDisconnect` = local leave (closes our
  links, no group end signal). `onSeparate` turns one leg into a standalone 1:1 and holds the
  rest. `onMerge` is refused. Conference extras carry the group key and participant label so
  `SagaCallRegistry.telecomCallId` / `displayNameFromCall` resolve the parent.
- `SagaIrohConnection` gains `lookupKey`, `endpointId`, `groupId`. In a conference it adds
  `CAPABILITY_SEPARATE_FROM_CONFERENCE` and `CAPABILITY_DISCONNECT_FROM_CONFERENCE`. Its
  `onDisconnect` closes that link only and calls `GroupCallCoordinator.onLegEnded`.
- `GroupCallCoordinator.requestAdd`: evaluates `canAddParticipant` before touching any state,
  promotes the live 1:1 Iroh call to a group on the first add (`syncConference` creates the
  Conference, `addConnection` for each leg, then `ConnectionService.addConference`), dials the
  new leg through `SagaDialer.placeIrohCall` (PhoneAccount guard, `tel:` handle unchanged),
  and rolls the leg back if Telecom never creates its Connection. The Conference is dissolved
  when fewer than two legs remain (the survivor continues as a 1:1).
- `SagaIrohConnectionRegistry` tracks live Connections / Conferences.

### Dual transport path

`MeshBackend` is chosen per group at creation by `IrohNativeBridge.isMeshNativeAvailable()`:

| Backend | When | Behaviour |
|---------|------|-----------|
| `NATIVE_MESH` | `nativeMesh*` present in the loaded library | One mesh group, one link id per peer (`meshOpen`, `meshAddPeer`, `meshRemovePeer`, `meshClose`, `meshPeerFingerprint`); link state arrives through `IrohNativeBridge.notifyMeshPeerState` -> `GroupCallCoordinator.onMeshPeerState` -> `IrohDialManager.onMeshLinkState`. |
| `PAIRWISE_SESSIONS` (scaffold) | mesh JNI absent | One 1:1 native session per peer link, all keyed under the same `GroupCallState`. |

In the scaffold the adder holds a link to every added peer; links **between two added peers
are not created**. Decision 1 (call survives the starter leaving) therefore needs the mesh
JNI to hold end to end. TODO(mesh-jni): unify on `NATIVE_MESH` and delete the scaffold.

### InCall UI (`SagaInCallActivity`)

- `conference_participants` TextView replaced by participant rows
  (`item_group_participant.xml`): name (with "introduced by X" when it applies), per-row
  padlock via `CallSecurityPresenter`, status text, Remove (hang up that participant only).
  Carrier conference children use the same rows, always "Not encrypted".
- Header padlock/status = `aggregateSnapshot()`; Encrypted only when all links are.
- **Add participant** button (Iroh calls only): Saga contact picker
  (`IrohContactDirectory`, contacts with a Saga key row) plus "Other contact or peer ID"
  (must resolve to an Iroh target; carrier numbers are refused). On a 5th person the "Call is
  full" dialog is shown.
- **Confirm dialog** when the contact has no resolvable key: only the adder can allow the
  unencrypted join. If a keyed link later settles unencrypted, `gateUnencryptedMedia` keeps
  media off and asks once per link.
- Security updates are collected for every leg (not just the focused call).
- Section 5a modal per participant (once per leg key; Cancel removes only that participant).
- One connect cue per conference: `claimConnectCue` is consumed by the first leg that settles.
  A late joiner, swap or unhold never replays it. A 1:1 whose cue slot was already consumed
  carries that over on promotion. No cue is played from the 5a "Continue" path in a group.
- Section 5b mid-call re-handshake already resolves per link (lookup key -> leg key).

### Introduced participants

A peer introduced by another participant (mesh roster) shows as "Charlene, introduced by
Alice" **only for unknown contacts**. It is verified against the roster fingerprint
(`RosterKeyCheck`); without a matching authenticated link fingerprint the row carries
"key not verified" and no padlock. Known contacts show their saved name only.
Inbound introductions arrive through `SagaDialer.EXTRA_GROUP_ID`,
`EXTRA_INTRODUCER_NAME`, `EXTRA_ROSTER_FINGERPRINT` on the incoming call extras
(`GroupCallCoordinator.registerIncomingGroupLeg`); the native mesh will fill them.

## UI mapping

| UI control | Carrier | Iroh group |
|------------|---------|------------|
| Secondary line | held / incoming name | held peer or "waiting" |
| Participants list | Telecom `getChildren()` names, padlock each | `GroupCallState` rows, padlock each |
| Add call | Saga dialer `tel:` | unchanged (separate 1:1, never merged) |
| Add participant | hidden | Iroh contact picker, max 4 total |
| Merge | cellular-only | hidden / refused |
| Split | Telecom split | `onSeparate`: leg becomes 1:1, rest held |
| Remove / hang one | Telecom disconnect child | close that link only |
| End | disconnect parent (+ children) | leave: close our links only |

## Test hooks

Debug-only (`FLAG_DEBUGGABLE`) intents on `MainActivity`, same style as the other `TEST_*`:

- `org.saga.TEST_ADD_PARTICIPANT` extras: `contact_name` (required), `confirm_unencrypted`
  (bool, stands in for the adder's confirm dialog), `introduced_by` (name),
  `roster_fingerprint` (64 hex), `conference` (optional group id).
- `org.saga.TEST_GROUP_HANGUP_PARTICIPANT` extras: `contact_name` (or `lookup_key`).

E2E C1-C5 under `testing/e2e/` exercise carrier multi-call and the never-mix guard. Emulator
RIL may not expose real conference children: mark **needs hardware** rather than faking merge.
Group E2E (alice + bob + charlene) needs three emulators and the mesh JNI for the
peer-to-peer links.

## Gaps waiting on native mesh JNI

- `nativeMeshIsAvailable`, `nativeMeshOpen`, `nativeMeshAddPeer`, `nativeMeshRemovePeer`,
  `nativeMeshClose`, `nativeMeshPeerFingerprint`, and the native-side call of
  `IrohNativeBridge.notifyMeshPeerState(groupId, linkId, state)` are the Kotlin-side contract;
  reconcile with the final Rust names.
- Peer-to-peer links between added peers, roster signalling (introducer name, roster
  fingerprint) and inbound group legs without ringing.
- Mixing N-1 receive streams into one playout and one capture fan-out. The scaffold runs one
  audio engine per link.
- Authenticated per-link fingerprint (`meshPeerFingerprint`): until present, introduced
  unknown contacts stay "Not encrypted".
