# Conference / multi-call UI readiness

Design note for Phase 4 InCall multi-call and future Iroh group calls.
Implementation today is **carrier conference + multi-call hold/swap** only.

## Current surface

`SagaInCallActivity` + `SagaInCallService` host one InCall composition for:

| Role | Source |
|------|--------|
| Active | `Call.STATE_ACTIVE` / dialing / connecting |
| Held | `Call.STATE_HOLDING` |
| Ringing (second) | `Call.STATE_RINGING` while another call exists |
| Conference | Telecom conference parent + `getChildren()` |

Actions are shown only when `Call.Details.can(...)` / `getConferenceableCalls()` allow them (`MultiCallActions`).

## Origin rules (enforced)

| Action | Carrier | Iroh | Mixed Iroh+carrier |
|--------|---------|------|--------------------|
| Hold / unhold / swap | yes | yes | yes (Iroh media pause/resume already on `STATE_HOLDING`) |
| Merge / conference | yes (cleartext stock parity) | no (not implemented) | **never** — Merge hidden; `conference()` / `mergeConference()` refused with log |
| Padlock / §5a | always KEY_NOT_FOUND / "Not encrypted" | normal handshake path | unchanged per call; no connect cue on swap/unhold/merge |

Carrier calls always resolve via `CallOrigin.CELLULAR` (§14.1 / existing resolver). Encrypted group calls remain Iroh-only (future).

## Future Iroh group calls (not built)

Keep components **origin-agnostic** where possible:

1. **Per-participant padlock** — extend the participants list with a security snapshot per child / mesh peer (Encrypted / Securing / Not encrypted), not one status for the whole conference.
2. **Add participant** — Iroh path: dial-by-public-key / contact key into a room or mesh; do not reuse carrier `ACTION_DIAL` / `tel:` placeCall.
3. **Never mix carrier/Iroh** — same guard as today; a group session is either all-cellular (unencrypted conference) or all-Iroh (encrypted mesh/SFU later per saga-spec §14).
4. **Multi-peer work still needed**
   - Signaling: room/ticket or pairwise mesh join (see §14.2–14.4).
   - Media: N−1 RoQ streams or SFU + SFrame-style frame encryption.
   - InCall model: treat Iroh participants as first-class children without Telecom `Call.conference()` (Telecom conference is carrier RIL).
   - Cues: one connect cue when the local session first reaches Encrypted; no cue on late joiner / swap.

## UI mapping (origin-agnostic hooks)

| UI control | Today | Future Iroh group |
|------------|-------|-------------------|
| Secondary line | held / incoming name | held peer or "waiting" |
| Participants list | Telecom `getChildren()` display names | mesh/SFU roster + per-peer padlock |
| Add call | Saga dialer `tel:` | Add participant (Iroh) |
| Merge | cellular-only | hidden for Iroh; join-room instead |
| Split / hang one | Telecom split / disconnect child | leave one peer / kick |
| End | disconnect parent (+ children) | leave room / end session |

## Test hooks

E2E C1–C5 under `testing/e2e/` exercise carrier multi-call and the never-mix guard. Emulator RIL may not expose real conference children — mark **needs hardware** rather than faking merge.
