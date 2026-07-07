# Saga — Encrypted Voice Calling (Preliminary Spec)

**Status:** Draft / pre-implementation
**Scope:** Android (primary), Linux (secondary peer)

## 1. Summary

Saga lets two phones make what looks and behaves like a normal cellular call,
while transparently negotiating end-to-end encryption over the data path
whenever both ends support it. If the peer doesn't support Saga, or no data
connectivity is available, the call may fall back to a normal unencrypted
cellular call — but only silently for contacts with no prior encrypted
history. Once encryption has been established with a contact, a failed
handshake requires explicit user confirmation before continuing unencrypted
(see §5a). Encryption is never required to place a call, and is not visible
as a separate "app" experience — it rides inside the native dialer UI.

## 2. Goals

- Calls appear in the native Android in-call UI, call log, and Bluetooth/car
  integrations — indistinguishable from a carrier call.
- Encryption activates automatically when both peers support it; otherwise
  the call may proceed unencrypted — silently only for contacts that have
  never been encrypted before, with an explicit warning for previously
  encrypted contacts (§5a).
- Key exchange uses the hardware token's on-device ephemeral ECDH + signature
  primitive — the long-term key never touches key agreement, only signs the
  ephemeral offer (forward secrecy per call).
- Audio transport uses the existing data channel (LTE data / WiFi), not the
  carrier voice codec — no acoustic modem, no voice-band bitrate ceiling.
- Handshake latency stays inside the existing call-setup window
  (dial → ringback → connected), which users already tolerate as multi-second.

## 3. Non-goals (this iteration)

- No in-call rekeying / ratcheting — one handshake per call; SRTP replay
  window limits single-key call duration to roughly 15–20 minutes at typical
  VoIP rates (see §8).
- No group calls in v1 core flow (§1–§13 describe two-party calling only);
  group calling and decentralized routing options are captured as a
  forward-looking design in §14 but not part of this iteration's build.
- No app-native calling UI on Android — must use system Telecom integration.
- Linux side is a SIP/RTP reference peer only (§8); no illusion of a
  "native OS dialer" since none exists to blend into.

## 4. Architecture

```
┌─────────────────────┐        Data path (LTE/WiFi, IP)        ┌─────────────────────┐
│  Android Phone A     │ ───────────────────────────────────── │  Android Phone B     │
│                      │        RTP/SRTP media + handshake      │                      │
│  Telecom             │                                        │  Telecom             │
│  ConnectionService   │                                        │  ConnectionService   │
│  (self-managed)      │                                        │  (self-managed)      │
│                      │                                        │                      │
│  Saga client ────────┼──── APDU (ECDH + sign) ────► Token A   │                      │
│  Hardware Token A     │                                        │  Hardware Token B ───┼──── APDU
└─────────────────────┘                                        └─────────────────────┘
```

Fallback path (no Saga peer / no data): call routes as an ordinary cellular
voice call. Silent downgrade only when no prior encrypted history exists
with that contact; otherwise §5a applies.

## 5. Call flow

1. **Dial.** User places call via Saga's `ConnectionService` (or receives one).
   Native in-call UI appears immediately, as with any call.
2. **Probe.** Saga attempts a handshake over the data channel in parallel
   with normal call setup (does not block ringback/connect).
3. **Handshake (if peer responds):**
   a. Each side's token generates a fresh ephemeral ECDH keypair, signs the
      ephemeral pubkey with the long-term key.
   b. Exchange `(ephemeral_pubkey, signature)` over the data connection.
   c. Each side verifies peer's signature, computes ECDH shared secret
      on-token.
   d. HKDF over shared secret + transcript hash → session key.
   e. Transcript-confirmation message exchanged (MAC over full handshake
      transcript) before switching to encrypted frames — prevents silent
      desync and binds the handshake against replay/splicing.
   f. Cipher profile ack (which AEAD/profile to use for this call).
4. **No response / timeout — branches on prior expectation, not silent
   in the general case:**
   - **No key ever resolved for this contact (never encrypted before):**
     call proceeds as a normal unencrypted call. Neutral "not encrypted"
     indicator only — no prior expectation exists to violate.
   - **A key resolves for this contact, or a prior call with them was
     encrypted, and *this* call's handshake fails or times out:** this is
     treated as a potential downgrade attack, not a neutral fallback. The
     call does not proceed silently — see §5a.
5. **Media:** RTP audio encrypted via negotiated AEAD (default:
   ChaCha20-Poly1305, one key per call, frame counter as nonce).
6. **UI state:** in-call UI shows "securing…" until handshake completes,
   then flips to "encrypted." If handshake fails after connect, flips to
   unencrypted rather than dropping the call.

## 5a. Downgrade handling (security-critical)

Rationale: a "not encrypted" outcome is only safe to treat as neutral when
there was never an expectation of encryption in the first place. Once a
contact's key is known — whether from a prior successful encrypted call or
an out-of-band exchange — a handshake failure on a subsequent call to that
same contact is indistinguishable from active interference (an attacker
blocking/delaying the handshake while letting the call itself connect).
Silently downgrading in that case defeats the point of having encryption at
all, and the person on the call has no way to notice it happened.

**Behavior:**

- Saga tracks, per contact, whether encryption has ever succeeded with them
  (a local "encryption established" flag/counter, not a server-side record).
- **Downgrade on a never-encrypted contact:** proceed silently, per §5 step 4
  — this is the existing, non-security-critical path.
- **Downgrade on a previously-encrypted contact:** do not silently connect
  unencrypted. Interrupt with an explicit warning before the call connects
  (or immediately upon connecting, if the handshake fails mid-setup):
  "Couldn't secure this call with [contact] — it's usually encrypted. This
  could mean a network problem, or someone interfering with the call."
  Require an explicit tap to continue unencrypted or to cancel the call.
  This is friction by design — the failure mode here should be annoying,
  not invisible.
- Each downgrade event for a previously-encrypted contact is logged
  locally (timestamp, contact) so a person can review whether this has
  happened once (plausibly transient) or repeatedly (plausibly an active,
  ongoing interception) — a single occurrence and a pattern warrant
  different levels of concern, and only the person reviewing their own call
  history is in a position to judge that.

**Open items:**
- [ ] Exact UX for the warning (modal that blocks connecting vs. a banner
      that allows connecting through it) — needs to be obviously
      interruptive, not a small icon change easy to miss.
- [ ] Whether "encryption established" should require more than one
      successful past call before treating a downgrade as suspicious
      (avoids false alarms from a contact's very first call happening to
      fail its handshake for an unrelated reason).
- [ ] Should apply symmetrically to the decentralized-mode (Iroh) transport
      once §14 moves from forward-looking design to implementation.

## 5b. Mid-call security transitions (encrypted -> unsecured)

§5a covers downgrade at call *setup*. This covers the case where a call is
already encrypted and connected, and the secure session is subsequently
lost mid-call — a distinct and arguably more serious event, since it means
something that was working stopped working, rather than never having
started.

**First: what can legitimately cause this, given §3 rules out in-call
rekeying?** Two realistic triggers, with different correct responses:

1. **Network handover** (e.g. WiFi calling switches to cellular data mid-call,
   or vice versa). The underlying data path changes, which may force a fresh
   handshake rather than a continuation of the existing one. This is a
   legitimate, non-adversarial event.
2. **Sustained decryption failure** on the media stream (corrupted/dropped
   SRTP frames beyond what replay/loss tolerance handles). This is *not*
   necessarily benign — an attacker able to selectively corrupt or block
   packets could induce exactly this condition on demand, which makes it a
   plausible attack surface if the response to it is "fall back to
   plaintext and keep going."

**Policy (different response per trigger):**

- **Handover-triggered:** treat as a bounded re-handshake window, not an
  immediate downgrade. If the re-handshake completes within **4 s** (same
  hard timeout as setup handshake, §8), the call continues encrypted with no
  visible interruption. If
  it fails, this is treated identically to §5a's downgrade case (this
  contact has an established encryption expectation) — warning triggers,
  not silent continuation.
- **Decryption-failure-triggered:** default to **terminating the call
  rather than falling back to plaintext RTP.** Silently accepting
  plaintext media after the secure session breaks down is the same shape
  of attack surface as the setup-time downgrade in §5a, except mid-call —
  and arguably easier for an attacker to trigger deliberately (corrupt a
  few packets) than blocking a handshake outright. This needs explicit
  sign-off as a design decision, since it trades call continuity for
  security; the alternative (continue unsecured with warning, same as the
  handover case) is usable but reopens the attack surface §5a was written
  to close.

**Audio warning (this message's ask):**

- A short, distinct audio tone plays at the moment a call's security state
  actually transitions from encrypted to unsecured mid-call (i.e. the
  handover case resolves to "downgraded," per the policy above) — not
  during the brief re-handshake window itself, and not repeated for the
  remainder of the call.
- Toggle: **Settings > "Play audio warning on mid-call security loss"**,
  independent of the always-on visual warning icon (§13's "Downgraded"
  state) and the logging behavior (§5a) — the tone is an additive alert on
  top of those, not a replacement for either, since a mid-call tone is easy
  to miss in a noisy environment or if the phone isn't at the ear at that
  exact moment.
- Deliberately a distinct sound, not reused from any existing call/system
  sound, to avoid ambiguity about what triggered it.

**Open items:**

- [ ] Decide the decryption-failure policy (terminate vs. downgrade-with-
      warning) — this is a real security-vs-usability tradeoff, not a
      detail to default silently.
- [x] Re-handshake timeout for the handover case: **4 s**, aligned with the
      setup handshake hard timeout (§8).
- [ ] Confirm default state for the audio-warning toggle (on by default is
      recommended given the stakes, but "optional" implies user control
      either way).

## 6. Cryptography

| Stage | Primitive | Notes |
|---|---|---|
| Key agreement | Token: ephemeral ECDH, long-term key signs ephemeral offer | On-device TRNG, forward secret per call |
| Transcript binding | HKDF `info` = hash of handshake transcript | Hand-rolled 3-message handshake, not full Noise — revisit if session shapes grow (see §7.4) |
| Session cipher | ChaCha20-Poly1305 (default profile) | Per-RTP-frame, nonce derived from sequence number |
| Media framing | SRTP | Handles replay protection, per-packet IV derivation |

Cipher profile selection reuses the token's existing named-profile concept
(`standard`, `conservative`, etc.) for *configuration*, not layering — voice
uses a single fast AEAD per frame, not cascaded ciphers, due to latency
sensitivity.

## 7. Rust crates — minimum self-rolled design criteria

**Design rule:** Saga writes as close to zero original cryptographic or
protocol-framing code as possible. Every primitive and every handshake
message format is either (a) a crate already used by Galdralag-firmware, or
(b) an existing, audited crate for a need the firmware doesn't cover
(RTP/SRTP transport, decentralized networking). No self-rolled crypto, no
self-rolled handshake state machine where the firmware already defines one.

### 7.1 Reused directly from Galdralag-firmware's own dependency set

Galdralag-firmware's cryptographic dependency policy draws everything from
audited RustCrypto/dalek crates, nothing implemented in-tree. Saga adopts
the same crates for the overlapping needs, rather than picking equivalents:

| Crate | Used for in Saga | Matches firmware's use |
|---|---|---|
| `x25519-dalek` | ECDH half of the handshake | Same crate, same curve (X25519, RFC 7748) |
| `ed25519-dalek` | Signing the ephemeral offer with the long-term token key | Same crate, same signature scheme (RFC 8032) |
| `chacha20poly1305` | Per-frame AEAD for SRTP media and the group-call frame-encryption layer (§14) | Same crate, same cipher (RFC 8439) |
| `hkdf` | Session key derivation from the ECDH shared secret + transcript | Same crate |
| `blake3` | Transcript hashing / integrity binding | Same crate |
| `zeroize` | Clearing session keys and derived secrets from memory on drop | Same crate, same pattern (`Zeroize`/`ZeroizeOnDrop`) |
| `subtle` | Constant-time comparison anywhere Saga compares secret-derived values (e.g. transcript MAC verification) | Same crate (`ConstantTimeEq`) |

`aes-gcm`, `sha2`, `sha3`, `blake2`, `pbkdf2`, `hmac`, `p256`, `p384` are in
the firmware's dependency set but not currently needed on the Saga side —
listed here only so a future cipher-profile choice (e.g. matching a
`conservative` or Brainpool-based token profile) doesn't introduce a new
crate where one is already vetted and in use.

`vsss-rs` (Shamir) is a firmware/vault concern, not a calling-app concern —
excluded from Saga's dependency list unless a future feature needs the phone
app itself to handle key shares.

### 7.2 Existing crates for needs the firmware doesn't cover

| Crate | Used for | Why not self-rolled |
|---|---|---|
| `webrtc-srtp` / `webrtc` | RTP/SRTP packet framing, replay protection, per-packet IV derivation | Getting SRTP framing subtly wrong by hand is a known footgun; use the existing pure-Rust implementation |
| `iroh` | Decentralized-mode transport (§14.4): NAT traversal, relay fallback, dial-by-public-key | Solves QUIC/NAT-traversal/relay correctly; not something to reimplement |

### 7.3 Explicitly cut

- **`snow` (Noise Protocol Framework)** — removed entirely, not just
  deferred. Since Galdralag-firmware already ships an authenticated
  ephemeral ECDH session protocol (`ephemeral-session` crate, documented in
  `docs/EPHEMERAL_SESSION.md`), adopting a second, independently-designed
  handshake framework (Noise) alongside it would itself be the kind of
  redundant, harder-to-audit surface this design criteria exists to avoid.
- Any bespoke SRTP, ICE, or DTLS code — covered by `webrtc`/`webrtc-srtp`.

### 7.4 Open item superseding §12

**§12's 3-message handshake format was drafted before this repository was
located and is very likely redundant.** Galdralag-firmware already defines
and tests an authenticated ephemeral ECDH session protocol with its own
wire format (`docs/EPHEMERAL_SESSION.md`, crate `ephemeral-session`). The
minimum-self-rolled rule means Saga should transport *that* protocol's
messages over SIP/Iroh rather than defining a new one:

- [ ] Read `docs/EPHEMERAL_SESSION.md` and the `ephemeral-session` crate
      directly; confirm whether its wire format already includes transcript
      confirmation and cipher-profile negotiation (§12's messages 2–3), or
      whether Saga still needs a thin wrapper for those two things.
- [ ] If the firmware's protocol covers it end-to-end, retire §12 in favor
      of a reference to that protocol rather than maintaining a parallel
      spec.
- [ ] If a wrapper is still needed, keep it to the smallest possible
      addition (e.g. just the cipher-profile-ack field), not a rewrite of
      the handshake itself.

## 8. Decisions (formerly open questions)

### Handshake budget

- [x] **Planning budget (pending hardware measurement):** allocate **2 s**
  target / **4 s** hard timeout from data-path availability before downgrade
  logic applies (§5 step 4 / §5a). UI shows "Securing…" throughout; the probe
  runs in parallel with call setup and does not block ringback/connect.
  Per-side token work (ephemeral ECDH generate + sign) is estimated at
  ~100–600 ms; full-exchange critical path is ~300 ms–1.5 s crypto plus 1–2
  SIP RTTs.
- [ ] **Verify on hardware:** benchmark Galdralag token APDU latency for one
  ECDH + one sign per direction (`T_generate + T_sign + T_verify + T_ecdh`,
  end-to-end both ways); adjust the 2 s / 4 s numbers if measurement
  exceeds estimate.

### VoLTE / VoWiFi fit

- [x] Handshake uses the same IP/SIP path liblinphone already employs (LTE
  data or WiFi), not the circuit voice bearer. Expect completion inside
  normal call-setup windows on both VoLTE (simultaneous data) and VoWiFi when
  an IP route to the peer exists; add **+500 ms** slack on VoWiFi for
  IPsec/ePDG overhead. Primary failure mode is voice-up/data-down (handshake
  times out → downgrade per §5).
- [ ] **Verify in the field:** carrier test matrix on both VoLTE and VoWiFi
  paths; confirm ≥95% handshake completion before callee answers.

### Fallback UX ("not encrypted")

- [x] Adopt §13 icon states and paired status text: open padlock +
  "Securing…" (in progress), closed padlock + "Encrypted", key-with-red-X +
  "Not encrypted" (neutral, never-encrypted contact), broken-padlock/warning
  icon + §5a modal (previously encrypted, handshake failed). Do not merge
  "key not found" and "downgraded" visuals — they represent different
  situations and must stay distinct.
- [ ] Confirm license of the gnupg-hamradio icon set (§13 TODO) or redraw
  equivalent glyphs in-house.

### Rekeying (long calls)

- [x] **v1: no in-call rekeying** — one ephemeral ECDH handshake yields one
  session key for the entire call (§3). Mid-call path change gets a bounded
  re-handshake (§5b), not periodic rekey. Known ceiling: the SRTP replay
  window (~2¹⁵ frames) imposes a practical limit of roughly 15–20 minutes at
  typical VoIP packet rates on a single key; accept for v1, revisit in v2 if
  long-call use cases matter.

### Linux peer scope

- [x] **Reference implementation only** — a `liblinphone`-based CLI or
  minimal test harness is sufficient to register SIP, place/receive calls,
  run the Saga handshake, and verify encrypted media (with a token bridge for
  crypto parity with Android). Not a full softphone product: no contacts
  app, dialer polish, or end-user packaging. Purpose: dev/test peer for
  Android work and CI interoperability checks.

## 9. Explicitly rejected approach

Acoustic data-over-sound modem (e.g. `ggwave`) carrying the handshake/media
over the actual voice-band audio channel was considered and rejected: modern
calls (VoLTE/VoWiFi) are already IP-based under the hood, so there's no need
to fight lossy speech codecs, AGC, noise suppression, and VAD clipping when
the data path is directly available. Native Telecom integration
(`ConnectionService`) achieves the "looks like a normal call" requirement
without needing to literally route through the carrier's voice codec.

## 10. Codebase base (fork candidate)

**Recommendation: fork `linphone-android` + `linphone-sdk` (liblinphone).**

Rationale:
- Already implements a self-managed `ConnectionService` (via `androidx.core.core-telecom`)
  for native in-call UI integration — the exact piece we'd otherwise build from scratch.
- Already implements ZRTP with SAS-based contact trust (blue/red trust indicators
  per contact) — gives us a reference SRTP/media-encryption pipeline
  (`mediastreamer2` + `bzrtp`/`bctoolbox`) instead of writing RTP/SRTP handling
  from zero.
- Cross-platform SDK: same `liblinphone` core covers Linux (and desktop generally),
  satisfying the Linux SIP/RTP peer requirement without a second implementation.
- Actively maintained (Belledonne Communications), GPLv3 open-source repo, with a
  paid proprietary-license option if a closed-source distribution is ever needed.

**License note:** linphone-android and liblinphone are GPLv3. A distributed fork
must remain GPLv3 unless a commercial license is purchased from Belledonne
Communications — plan branding/distribution accordingly.

**Alternative considered:** Jami (GPLv3, P2P/DHT-based, Android + Linux clients).
More decentralized (no SIP server needed) but heavier (OpenDHT) and less directly
suited to "looks like a normal call through the carrier" framing than Linphone's
SIP-account model. Kept as a fallback option if P2P discovery becomes a
requirement later.

### Integration strategy: layer, don't replace

Rather than ripping out `bzrtp` (C, non-trivial to modify safely), run the
token-based handshake as a second channel alongside ZRTP:

1. Keep ZRTP doing what it already does (its own DH, SAS phrase for human
   verification).
2. In parallel, run the token's ephemeral-ECDH-plus-signature handshake over
   SIP `MESSAGE`/`INFO` as an application-level exchange.
3. Final session key = `HKDF(zrtp_secret || token_secret, transcript_hash)` —
   combining both means an attacker must break *both* ZRTP's DH and the
   token's hardware-backed ECDH to recover the session key.
4. If the token/peer doesn't support Saga's handshake, ZRTP alone still runs
   as Linphone normally does — this is a clean, low-risk fallback path that
   requires no changes to the C core, only to the Kotlin/Java call-setup
   layer and a small JNI bridge to the token.

This avoids invasive surgery on `mediastreamer2`/`bzrtp` for v1, at the cost
of the token's key not being the *sole* trust anchor for the media key (it's
combined with ZRTP's). Revisit once the layered approach is proven, if a
token-only trust model becomes a hard requirement.

## 11. Android ConnectionService sketch

Linphone already registers a `PhoneAccount` and self-managed `ConnectionService`
(`org.linphone.telecom` package, roughly). A Saga-specific overlay looks like:

```kotlin
// Registering the PhoneAccount (once, e.g. in Application.onCreate)
val phoneAccountHandle = PhoneAccountHandle(
    ComponentName(context, SagaConnectionService::class.java),
    "saga_account"
)
val phoneAccount = PhoneAccount.builder(phoneAccountHandle, "Saga")
    .setCapabilities(
        PhoneAccount.CAPABILITY_SELF_MANAGED or
        PhoneAccount.CAPABILITY_SUPPORTS_VIDEO_CALLING
    )
    .build()
telecomManager.registerPhoneAccount(phoneAccount)

// The ConnectionService itself
class SagaConnectionService : ConnectionService() {
    override fun onCreateOutgoingConnection(
        connectionManagerHandle: PhoneAccountHandle?,
        request: ConnectionRequest
    ): Connection {
        val connection = SagaConnection()
        connection.setInitializing()
        // Kick off call setup: SIP INVITE via liblinphone core,
        // Saga handshake probe over the same data path, in parallel.
        sagaCore.startOutgoingCall(request.address, onHandshakeUpdate = { state ->
            connection.setStatusHints(
                StatusHints(if (state.encrypted) "Encrypted" else "Not encrypted", null)
            )
        })
        connection.setActive()
        return connection
    }

    override fun onCreateIncomingConnection(
        connectionManagerHandle: PhoneAccountHandle?,
        request: ConnectionRequest
    ): Connection {
        val connection = SagaConnection()
        connection.setRinging()
        // Wire to liblinphone's incoming INVITE handling + handshake responder
        return connection
    }
}

class SagaConnection : Connection() {
    override fun onAnswer() {
        setActive()
        sagaCore.acceptCall()
    }
    override fun onDisconnect() {
        setDisconnected(DisconnectCause(DisconnectCause.LOCAL))
        sagaCore.endCall()
        destroy()
    }
}
```

`sagaCore` here is the JNI-bridged Rust layer doing the actual handshake
(talking to the token over USB/CCID) and feeding the derived key into
`mediastreamer2`'s SRTP configuration once ready.

## 12. Handshake wire format (draft)

Carried as SIP `INFO`/`MESSAGE` body (or a dedicated media-adjacent channel),
`application/saga-handshake+cbor` content type. Three messages:

| # | Direction | Fields | Purpose |
|---|---|---|---|
| 1 | A -> B | `version:u8`, `ephemeral_pubkey:32B`, `signature:var`, `nonce_a:16B` | Offer: ephemeral ECDH pubkey signed by A's long-term token key |
| 2 | B -> A | `ephemeral_pubkey:32B`, `signature:var`, `nonce_b:16B`, `profile_choice:u8` | Counter-offer + proposed cipher profile |
| 3 | A -> B | `transcript_mac:32B` | Confirms A derived the same key over `hash(msg1 || msg2)`; B replies with its own `transcript_mac` to complete mutual confirmation |

`profile_choice` indexes into the token's existing named-profile table (e.g.
`standard` = ChaCha20-Poly1305 single layer). Both sides must agree on
`version` and `profile_choice`; mismatch aborts the handshake and falls back
to unencrypted, per Section 5 step 4.

**Open item:** whether `nonce_a`/`nonce_b` are needed given the ephemeral
keys themselves provide freshness -- likely redundant and removable once the
transcript-hash construction is finalized; kept here as a placeholder pending
review.

## 13. Call-state icon (padlock)

Reuses Galdra's existing human-readable name-tag / keyring model: the icon
state is driven by whether a resolved key exists for the current contact's
name tag, and whether a handshake has completed for the active call.

| State | Condition | Icon |
|---|---|---|
| **Encryption possible** | Peer's key resolves from the contact's Galdra name tag; handshake not yet run or in progress | Open padlock |
| **Encrypted** | Handshake completed, transcript confirmed (Section 12 message 3), session key active | Closed padlock |
| **Key not found** | No key resolves for the contact's name tag (unknown contact, revoked/missing entry, or signature verification fails); no prior expectation of encryption | Key-with-red-X, styled after [`key-error.svg`](https://github.com/Supermagnum/gnupg-hamradio/blob/main/icons/key-error.svg) from the gnupg-hamradio icon set |
| **Downgraded (warning)** | A key resolves, or this contact has been encrypted before (§5a), but this call's handshake failed/timed out | Distinct broken-padlock/warning icon (not the same as "key not found") — paired with the interruptive warning from §5a, not passive icon-only signaling |

Notes:
- "Key not found" takes priority for display over "encryption possible" —
  i.e. if the contact has no resolvable key, don't show an open padlock
  implying encryption is one tap away when it isn't.
- "Downgraded (warning)" takes priority over every other state — it must
  never be visually confusable with "key not found," since the two
  represent very different situations (never expected encryption, vs.
  expected it and didn't get it this time). Collapsing them into one icon
  is exactly the failure mode §5a exists to prevent.
- Transition from open -> closed padlock should be the same point the
  in-call status hint text flips from "Securing..." to "Encrypted"
  (Section 5, step 6) -- one state change, not two independent signals.
- **TODO:** confirm license of the gnupg-hamradio icon set before vendoring
  any SVGs into the Saga app; no LICENSE file was visible when checked. If
  incompatible or unclear, redraw equivalent open/closed/error padlock
  glyphs in-house using the same visual language (padlock shape + red X
  overlay for the error state) to avoid provenance issues.

## 14. Group calls & decentralized routing (forward-looking design)

Not part of the v1 build (see §3), but captured here so the two-party
architecture doesn't foreclose it.

### 14.1 Why carrier conference-merge is unusable

Carrier-side call merging routes audio mixing through the carrier's network,
giving the carrier cleartext access to the mixed stream. Incompatible with
end-to-end encryption. Group calls with encryption intact must be handled
entirely at the app/data layer, following the pattern used by Signal and
Messenger:

- **SRTP/DTLS** secures the hop to whatever relay forwards media.
- **A second, frame-level encryption layer** (SFrame-style) secures the
  actual audio/video content end-to-end regardless of what the relay does —
  the relay only ever forwards ciphertext frames, never decrypts them.
- **Sender Keys** distribute a per-participant symmetric key once, over
  existing pairwise sessions (O(N) key distribution), rather than
  re-encrypting media per-recipient.

### 14.2 Mesh vs. relay threshold

- **Small groups (roughly ≤4–5 participants):** full mesh, no relay. Each
  device runs the §12 pairwise handshake with every other participant
  (O(N²) handshakes) and sends/receives N−1 direct SRTP streams. No new
  key-management scheme needed — pure extension of the two-party design.
- **Larger groups:** O(N²) handshakes/bandwidth stop scaling; a relay
  (SFU-equivalent) is needed. Media stays opaque to the relay via the
  frame-level encryption layer above; the token's role stays pairwise
  ECDH+signature, now bootstrapping sender-key distribution instead of a
  session key used directly for media.
- Exact threshold is untested — deferred until real bandwidth/latency
  numbers are available (do not implement until then).

### 14.3 Two deployment modes

| Mode | Signaling/discovery | Relay | Trust model |
|---|---|---|---|
| **Managed** | SIP via Flexisip (self-hosted or provider-run) | Flexisip conference focus | Traditional SIP account; familiar to Linphone base; single operator per deployment, but anyone can run one |
| **Decentralized** | Iroh (peer identity = public key, built-in NAT traversal + relay fallback) | Ad-hoc peer relay, or federated relay discovered via Iroh | No mandatory operator; encrypted even across relay hops |

Both modes reuse the same §12 handshake and §6 cipher stack — only the
signaling/transport substrate differs.

### 14.4 Decentralized transport: Iroh (recommended)

**Iroh** (Rust-native, v1.0 released June 2026) is the recommended transport
for decentralized mode, over the previously-considered OpenDHT/Jami stack:

- Dial-by-public-key model: connects to a peer's identity key directly,
  handling NAT traversal and relay fallback internally (QUIC + TLS 1.3).
  Maps cleanly onto Saga's token-based identity — the token's long-term key
  can plausibly serve as the Iroh node identity.
- When a direct path can't be established, traffic still routes through an
  encrypted relay that cannot decrypt it — decentralization survives the
  fallback case, unlike a plain TURN relay.
- Existing reference implementations validate the approach directly:
  `callme` (n0's own peer-to-peer audio call tool, Opus over `iroh-roq`,
  `cpal` for cross-platform audio I/O) and `iroh-live` (media livestreaming
  over Media-over-QUIC, with a working Android demo doing bidirectional
  camera/mic capture through a Rust core + JNI bridge — the same
  architecture shape as §11's `ConnectionService` sketch).
- `iroh-live` already has a room/ticket concept for multi-party sessions,
  and MoQ transports each audio/video track as an independent QUIC stream,
  so a dropped video packet never blocks audio — a reasonable primitive if
  group calls need per-participant streams through a relay.

**Gap:** Iroh solves live connectivity, not store-and-forward presence — it
doesn't help reach a peer whose device is currently offline/asleep. If
missed-call notification via a decentralized store is ever required,
OpenDHT (Jami's stack) remains the fallback option for that specific need,
kept as a secondary consideration rather than the primary transport.

**Licensing interaction:** Iroh is typically permissively licensed
(MIT/Apache-2.0 — confirm exact terms before depending on it), while
Linphone/Flexisip (§10) are GPLv3. No conflict combining them in one binary
(permissive code can be included in a GPL project), but if a fully
permissive, decentralized-only build is ever wanted as a separate product,
keep it distinct from the GPLv3-encumbered managed-mode build.

### 14.5 Open items

- [ ] Confirm Iroh's exact license before dependency lock-in.
- [ ] Determine mesh/relay participant-count threshold empirically.
- [ ] Decide whether missed-call/offline presence is a v-next requirement;
      if so, evaluate OpenDHT integration effort at that point.
- [ ] Prototype token identity <-> Iroh node identity binding.
