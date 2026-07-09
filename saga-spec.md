# Saga — Encrypted Voice Calling (Preliminary Spec)

**Status:** Draft / pre-implementation
**Scope:** Android (primary), Linux (secondary peer)

## 1. Summary

Saga **is** the phone app — it replaces the stock dialer via Android's
`ROLE_DIALER` grant and draws the in-call screen for every call on the
device through its own `InCallService`. Encrypted Saga-to-Saga calls use
**Iroh only** (dial-by-public-key, QUIC, built-in NAT traversal/relay
fallback): no SIP registrar, no Flexisip, no signaling server. The
Galdralag token handshake runs as an early exchange over the Iroh
connection; media is Opus over `iroh-roq` with frame-level AEAD.

Plain **cellular** calls still work: the carrier's telephony
`ConnectionService` originates and transports them; Saga renders them in
the same `InCallService` UI. Cellular calls are always structurally
unencrypted — they always get the neutral "not encrypted" treatment and
`call_unsecure.wav` at connect time, regardless of whether a Galdralag key
exists for that contact.

Every call — cellular or Iroh, encrypted or not — plays exactly one
connect-time audio cue (`call_secure.wav` or `call_unsecure.wav`) when its
security state resolves (see §5 step 4). Previously-encrypted contacts
whose Iroh handshake fails get the universal cue **plus** the §5a blocking
modal. Encryption is never required to place a call.

## 2. Goals

- Saga becomes the **default dialer** (`ROLE_DIALER`) — not a second app
  coexisting with the system dialer. This is the literal mechanism for
  "indistinguishable from a carrier call": Saga's `InCallService` is the
  system's in-call UI for cellular and Iroh calls alike, with call log and
  Bluetooth/car integrations behaving as users expect from the phone app.
- Iroh-originated calls encrypt automatically when both peers support Saga;
  every call plays an unmistakable connect-time audio cue when security
  state resolves (§5 step 4). Previously-encrypted contacts whose handshake
  fails get the cue **and** the §5a blocking modal — never a silent
  downgrade.
- Key exchange uses the hardware token's on-device ephemeral ECDH + signature
  primitive — the long-term key never touches key agreement, only signs the
  ephemeral offer (forward secrecy per call).
- Iroh media uses LTE data / WiFi (QUIC), not the carrier voice codec — no
  acoustic modem, no voice-band bitrate ceiling. Cellular calls use the
  carrier voice path and are never encryptable by Saga.
- Handshake latency stays inside the existing call-setup window
  (dial → ringback → connected), which users already tolerate as multi-second.

## 3. Non-goals (this iteration)

- No in-call rekeying / ratcheting — one handshake per call; SRTP replay
  window limits single-key call duration to roughly 15–20 minutes at typical
  VoIP rates (see §8).
- No group calls in v1 core flow (§1–§13 describe two-party calling only);
  group calling and decentralized routing options are captured as a
  forward-looking design in §14 but not part of this iteration's build.
- No SIP stack, registrar, or Flexisip deployment — Iroh is the only
  signaling/transport path for Saga-to-Saga calls (§14).
- Linux side is an Iroh reference peer only (§8); no illusion of a
  "native OS dialer" since none exists to blend into.

## 4. Architecture

Saga is the default dialer. Two call origins share one `InCallService` UI:

```
┌──────────────────────────────────────────────────────────────────────────┐
│  Saga (ROLE_DIALER)                                                       │
│                                                                           │
│  InCallService ── draws full-screen call UI for ALL calls on device      │
│       │                    (§13 padlock states + status text + audio cue) │
│       ├── Cellular calls: rendered only; originated by carrier           │
│       │   ConnectionService (telephony). Always "not encrypted".         │
│       │                                                                   │
│       └── Iroh calls: originated by Saga's managed ConnectionService     │
│           (NOT self-managed). Signaling + media over Iroh (QUIC/roq).     │
│                                                                           │
│  Token handshake (ephemeral-session) ── early exchange on Iroh conn    │
│  Hardware Token ◄── APDU (ECDH + sign)                                   │
└──────────────────────────────────────────────────────────────────────────┘

         Iroh path (LTE/WiFi)                    Peer phone (same stack)
┌─────────────────────┐   QUIC / iroh-roq / Opus   ┌─────────────────────┐
│  Android Phone A     │ ◄───────────────────────► │  Android Phone B     │
└─────────────────────┘                            └─────────────────────┘
```

Cellular fallback: user places a normal `tel:` call; Saga renders it but
does not encrypt it. Universal `call_unsecure.wav` at connect; no open
padlock even if a Galdralag key exists for the contact (§13). Iroh
handshake failure on a previously-encrypted contact triggers §5a.

## 5. Call flow

1. **Dial.** User places call via `tel:` (cellular) or Saga's Iroh dialer
   (peer public key / contact). Saga's `InCallService` call screen appears
   immediately for both origins.
2. **Probe (Iroh calls only).** Saga attempts a handshake over the Iroh
   connection in parallel with call setup (does not block ringback/connect).
   Cellular calls skip handshake — transport cannot be encrypted.
3. **Handshake (Iroh, if peer responds):**
   a. Each side's token generates a fresh ephemeral ECDH keypair, signs the
      ephemeral pubkey with the long-term key.
   b. Exchange `(ephemeral_pubkey, signature)` over the Iroh connection
      (Galdralag `ephemeral-session` wire format, §7.4).
   c. Each side verifies peer's signature, computes ECDH shared secret
      on-token.
   d. HKDF over shared secret + transcript hash → session key.
   e. Transcript-confirmation message exchanged (MAC over full handshake
      transcript) before switching to encrypted frames.
   f. Cipher profile ack (which AEAD/profile to use for this call).
4. **Security state resolves — universal connect-time acknowledgment:**
   The moment a call's security state is known, play exactly one audio cue
   via `MediaPlayer.create(context, R.raw.call_secure)` or
   `R.raw.call_unsecure` — once, audibly, at/near connect time. **Every
   call, every contact, every origin — no exceptions.** Files live at
   `app/src/main/res/raw/call_secure.wav` and `call_unsecure.wav`.

   | Outcome | Cue | Additional UI |
   |---|---|---|
   | Handshake succeeded (Iroh) | `call_secure.wav` | Closed padlock + "Encrypted" |
   | Handshake failed / never attempted, no prior encrypted history | `call_unsecure.wav` | Key-not-found icon + "Not encrypted" |
   | Handshake failed, prior encrypted history (§5a) | `call_unsecure.wav` | Downgraded icon + blocking modal |
   | Cellular call (any contact) | `call_unsecure.wav` | Key-not-found treatment always |

   The audio cue is informational/concurrent for the baseline case — it
   does not block connection. The §5a modal is **additive** for the
   downgrade case only. Connect-time cues are **structurally distinct**
   from the §5b mid-call security-loss toggle — separate prefs, separate
   code paths, separate audio assets; verify by grep, not assumption.
5. **Media (Iroh):** Opus frames encrypted via negotiated AEAD (default:
   ChaCha20-Poly1305) over `iroh-roq`. Cellular: carrier voice path.
6. **UI state:** `InCallService` call screen shows padlock + status text
   (§13) for the full call duration. Iroh: "Securing…" until handshake
   completes, then "Encrypted" or downgrade states. Cellular: always neutral
   not-encrypted.

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
  on an **Iroh** call (a local "encryption established" flag/counter, not
  a server-side record).
- **Downgrade on a never-encrypted contact (Iroh):** `call_unsecure.wav`
  plays; call proceeds with neutral key-not-found icon. No §5a modal.
- **Downgrade on a previously-encrypted contact (Iroh):** `call_unsecure.wav`
  plays **and** a blocking modal requires explicit tap to continue
  unencrypted or cancel:
  "Couldn't secure this call with [contact] — it's usually encrypted. This
  could mean a network problem, or someone interfering with the call."
  Friction by design — not invisible.
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
messages over Iroh rather than defining a new one:

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

**Target hardware (Dabao evaluation board / Baochip-1x):** 350 MHz VexRiscv
RV32-IMAC (Zkn scalar crypto extension, AES in-pipeline) with a separate
175 MHz crypto block: an algorithm-agnostic PKE engine (runtime-supplied
256-bit field prime, on-the-fly Montgomery parameters — Brainpool P256r1
already confirmed viable with no PKE RTL changes) covering RSA/ECC/ECDSA/
X25519/GCD, plus a ComboHash block (HMAC, SHA-256/512, SHA3, RIPEMD, Blake2,
Blake3), AES, ALU, TRNG, and SDMA. Transport is USB High Speed over USB-C.

- [x] **Planning budget, revised:** keep **2 s** target / **4 s** hard
  timeout (§5 step 4 / §5a) as the safe upper bound, but the earlier
  100–600 ms per-operation estimate assumed a generic firmware-mediated
  smartcard/secure-element, not a dedicated silicon PKE engine. A
  purpose-built ECC accelerator at 175 MHz doing one X25519 scalar
  multiplication plus one Ed25519 sign is plausibly low-single-digit to
  some tens of milliseconds of raw compute, not hundreds — and USB High
  Speed (vs. Full Speed) means APDU transport is unlikely to be the
  bottleneck either. Net effect: the 2 s/4 s budget almost certainly has
  more headroom than previously assumed, with the dominant remaining cost
  likely shifting to Iroh/network RTT rather than token compute — but this
  is still an estimate pending real measurement, not a number to build
  tight margins around.
- [ ] **Verify on hardware:** benchmark actual Dabao/Baochip-1x APDU
  latency for one ECDH + one sign per direction
  (`T_generate + T_sign + T_verify + T_ecdh`, end-to-end both ways) via USB
  HS; confirm whether the 2 s/4 s budget can be tightened now that
  real silicon numbers are obtainable, rather than treating it as an
  unknown-chip placeholder.

### VoLTE / cellular data fit

- [x] Iroh calls use LTE data or WiFi (QUIC), not the circuit voice bearer.
  Cellular calls use the carrier voice path and are never Saga-encrypted.
  Handshake runs on the Iroh connection in parallel with Iroh call setup.
- [ ] **Verify in the field:** test matrix on LTE and WiFi; confirm ≥95%
  handshake completion before callee answers on Iroh calls.

### Fallback UX ("not encrypted")

- [x] Adopt §13 icon states in Saga's `InCallService` call screen (primary
  surface — not notification-only) plus mandatory connect-time audio cues
  (§5 step 4): open padlock + "Securing…", closed padlock + "Encrypted",
  key-with-red-X + "Not encrypted", broken-padlock/warning + §5a modal.
  Cellular calls always use key-not-found treatment regardless of contact
  keys. Vendored SVGs in [`icons/`](icons/).
- [x] Icon set license confirmed: **AGPL**. Saga is a fresh codebase (§10),
  not a GPLv3 Linphone fork — confirm license combination with AGPL icons
  and Iroh (permissive) dependencies before shipping.

### Rekeying (long calls)

- [x] **v1: no in-call rekeying** — one ephemeral ECDH handshake yields one
  session key for the entire call (§3). Mid-call path change gets a bounded
  re-handshake (§5b), not periodic rekey. Known ceiling: the SRTP replay
  window (~2¹⁵ frames) imposes a practical limit of roughly 15–20 minutes at
  typical VoIP packet rates on a single key; accept for v1, revisit in v2 if
  long-call use cases matter.

### Linux peer scope

- [x] **Reference implementation only** — a minimal Iroh + `cpal` CLI or test
  harness sufficient to dial-by-public-key, run the Saga handshake, and
  verify encrypted media (with a token bridge for crypto parity with
  Android). Not a full softphone product. Purpose: dev/test peer for Android
  work and CI interoperability checks.

## 9. Explicitly rejected approach

Acoustic data-over-sound modem (e.g. `ggwave`) carrying the handshake/media
over the actual voice-band audio channel was considered and rejected: modern
calls (VoLTE/VoWiFi) are already IP-based under the hood, so there's no need
to fight lossy speech codecs, AGC, noise suppression, and VAD clipping when
the data path is directly available. Becoming the default dialer via
`ROLE_DIALER` + `InCallService` achieves the "looks like a normal call"
requirement without SIP or acoustic routing.

## 10. Codebase base (fork decision)

**Decision: build the `InCallService` / `ConnectionService` skeleton fresh.**

Linphone was previously considered as a fork base specifically for its SIP
stack and `mediastreamer2` pipeline. That rationale is **obsolete**: SIP,
Flexisip, ZRTP layering, and `liblinphone`'s `Call`/`Core` objects are
removed from the design (§4). Inheriting a large SIP-oriented codebase
adds weight without benefit now that signaling is Iroh-only and Saga owns
the in-call UI via `InCallService`.

**Why fresh:**
- Android's default-dialer + `InCallService` + managed `ConnectionService`
  APIs are well-documented; reference scope is modest compared to what
  Linphone's SIP stack replaced.
- No `CAPABILITY_SELF_MANAGED`, no `androidx.core:core-telecom` workaround,
  no `liblinphone` JNI surface.
- Clean dependency graph: Iroh + `cpal` + Galdralag handshake crates,
  permissive licenses (confirm Iroh terms, §14.5).

**Deferred alternative:** borrow Linphone's `mediastreamer2` audio pipeline
pieces (echo cancellation, jitter buffer, audio routing) **only if**
`cpal`-based audio quality proves insufficient in testing — do not default
to it preemptively. Evaluate after first Iroh audio prototypes, not before.

**Historical (superseded):** the prior §10 recommended forking
`linphone-android` + `linphone-sdk`, layering a token handshake alongside
ZRTP over SIP `MESSAGE`/`INFO`, and using self-managed `ConnectionService`
via `core-telecom`. None of that applies to the current architecture.

**GrapheneOS OS forking:** explicitly rejected. Default-dialer role grant
on stock Android / GrapheneOS achieves "replace the stock dialer" without
platform signing or OS maintenance burden.

## 11. Android Telecom integration (default dialer)

Saga requests `ROLE_DIALER` and becomes the app handling `tel:` intents and
drawing the in-call screen. **Do not** use `CAPABILITY_SELF_MANAGED` —
that existed for apps coexisting with another dialer's UI; Saga *is* the
dialer's UI.

### 11a. Request the dialer role

```kotlin
val roleManager = context.getSystemService(RoleManager::class.java)
val intent = roleManager.createRequestRoleIntent(RoleManager.ROLE_DIALER)
// launch via Activity Result API; handle grant/deny
```

Standard Android role grant — works on stock Android and GrapheneOS alike.
No bootloader unlock or platform signing.

### 11b. `InCallService` — the call screen for every call

As default dialer, Saga's `InCallService` draws the full-screen call UI for
**all** calls on the device:

| Origin | Who originates | Saga's role | Security UI |
|---|---|---|---|
| Cellular | Carrier telephony `ConnectionService` | Render only | Always key-not-found + `call_unsecure.wav` |
| Iroh | Saga's managed `ConnectionService` (§11c) | Originate + render | Full §13 padlock states + cues |

All four §13 icon states render on this primary call screen for the whole
call duration — not as a notification or secondary indicator. Reuse the
same state-resolution logic and priority ordering (downgraded > key-not-found
> encryption-possible > encrypted); only the rendering surface changed from
the old self-managed sketch.

Status text ("Securing…", "Encrypted", "Not encrypted") renders directly —
no `CallAttributesCompat` / `setStatusHints()` workaround needed.

### 11c. Managed `ConnectionService` for Iroh calls

Register a **normal** (non-self-managed) `PhoneAccount` + `ConnectionService`
for calls Saga originates over Iroh:

```kotlin
val phoneAccount = PhoneAccount.builder(phoneAccountHandle, "Saga")
    .setCapabilities(PhoneAccount.CAPABILITY_CALL_PROVIDER) // NOT SELF_MANAGED
    .build()
telecomManager.registerPhoneAccount(phoneAccount)

class SagaConnectionService : ConnectionService() {
    override fun onCreateOutgoingConnection(
        connectionManagerPhoneAccount: PhoneAccountHandle?,
        request: ConnectionRequest
    ): Connection {
        val connection = SagaIrohConnection()
        connection.setInitializing()
        // Dial peer by Iroh public key; run handshake on Iroh conn in parallel
        sagaIrohCore.startOutgoingCall(request.address, connection)
        return connection
    }
}
```

`sagaIrohCore` is the Rust/JNI layer: Iroh endpoint, `iroh-roq` media,
`ephemeral-session` handshake to token, frame AEAD. No SIP, no `liblinphone`.

### 11d. Connect-time audio cues

Bundled at `app/src/main/res/raw/call_secure.wav` and `call_unsecure.wav`.
Played once per call when security state resolves (§5 step 4). Distinct from
§5b mid-call warning audio and prefs (`saga_mid_call_security` vs connect
cue code in `org.saga.security.connect`).

## 12. Handshake wire format

**Carrier:** Galdralag `ephemeral-session` messages over the Iroh connection
as an early exchange after QUIC establishment — not SIP `INFO`/`MESSAGE`.
Message content and order follow `docs/EPHEMERAL_SESSION.md` in
Galdralag-firmware (§7.4). Only the transport changed.

The draft 3-message CBOR format below is **superseded** if the firmware
protocol covers transcript confirmation and cipher-profile negotiation
end-to-end — prefer the firmware wire format and retire this table once
confirmed:

| # | Direction | Fields | Purpose |
|---|---|---|---|
| 1 | A -> B | `version:u8`, `ephemeral_pubkey`, `signature`, `nonce_a:16B` | Offer |
| 2 | B -> A | `ephemeral_pubkey`, `signature`, `nonce_b:16B`, `profile_choice:u8` | Counter-offer |
| 3 | A -> B | `transcript_mac:32B` | Mutual confirmation |

Mismatch or timeout → unencrypted outcome per §5 step 4 (with universal
audio cue; §5a modal if prior encrypted history).

## 13. Call-state icon (padlock)

Rendered on Saga's **`InCallService` call screen** (primary surface) for
the full call duration, paired with status text and the mandatory connect-time
audio cue (§5 step 4). Reuses Galdra's name-tag / keyring model for Iroh
calls.

| State | Condition | Icon |
|---|---|---|
| **Encryption possible** | **Iroh call only.** Peer's key resolves; handshake in progress | Open padlock — [`icons/open.svg`](icons/open.svg) |
| **Encrypted** | **Iroh call only.** Handshake completed, session key active | Closed padlock — [`icons/locked.svg`](icons/locked.svg) |
| **Key not found** | Cellular call (**always**); or Iroh call with no resolvable key / never-encrypted handshake failure | [`icons/key-error.svg`](icons/key-error.svg) |
| **Downgraded (warning)** | **Iroh call only.** Key resolves or prior encrypted history, handshake failed | Broken-padlock — **not yet in `icons/`**; plus §5a modal + `call_unsecure.wav` |

**Cellular rule:** never show open/closed padlock on a cellular-origin call,
even if the contact has a known Galdralag key from prior Iroh calls —
encryption is structurally unreachable on that transport. Always key-not-found
+ `call_unsecure.wav`.

Notes:
- Priority ordering unchanged: downgraded > key-not-found > encryption-possible
  > encrypted. Do not re-derive per UI surface.
- Open → closed padlock aligns with status text "Securing…" → "Encrypted"
  and `call_secure.wav` at the same moment.
- Connect-time cues (`call_secure.wav` / `call_unsecure.wav`) are distinct
  from §5b mid-call security-loss audio (`SagaMidCallSecurityAudioWarning`,
  separate prefs and assets).
- **Resolved:** icon set in [`icons/`](icons/); AGPL — see §8.

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

### 14.3 Transport mode

| Mode | Signaling/discovery | Relay | Trust model |
|---|---|---|---|
| **Iroh (only)** | Dial-by-public-key; QUIC + built-in NAT traversal/relay fallback | Iroh relay (encrypted hop, cannot decrypt payload) | No mandatory operator; token long-term key maps to Iroh node identity |

The former Managed/Flexisip/SIP row is **removed**. Iroh is the only mode
for two-party Saga calls, not one of two deployment options.

Both-party handshake and cipher stack (§6, §7) run over Iroh; no parallel
SIP signaling path.

### 14.4 Iroh transport

**Iroh** (Rust-native) is the **only** Saga signaling and media transport:

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
  architecture shape as §11's managed `ConnectionService` + `InCallService`).
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
(MIT/Apache-2.0 — confirm exact terms before depending on it). Saga is a
fresh codebase (§10), not a GPLv3 Linphone fork.

### 14.5 Open items

- [ ] Confirm Iroh's exact license before dependency lock-in.
- [ ] Determine mesh/relay participant-count threshold empirically.
- [ ] Decide whether missed-call/offline presence is a v-next requirement;
      if so, evaluate OpenDHT integration effort at that point.
- [ ] Prototype token identity <-> Iroh node identity binding.

## 15. Testing: mock token & two-emulator encryption verification

Test-only material, strictly scoped to verifying the handshake and
encrypt/decrypt pipeline across two emulated Android devices before any real
hardware token or real audio device is involved.

### 15.1 Mock token — scope and enforcement

The mock token implements the same `ephemeral-session` wire format the real
Galdralag firmware defines (§7.4) — same message shapes, same
`x25519-dalek`/`ed25519-dalek` primitives — but backed by an in-memory
software keypair instead of hardware-backed TRNG and secure storage. It
exists to exercise the handshake, managed `ConnectionService` /
`InCallService` lifecycle, and encrypted media pipeline on two emulators
without physical tokens.

Scope is enforced at build time, not by convention:

- Lives in a separate crate path in the repo (e.g. `testing/mock-token`),
  not alongside the real token-integration code.
- Gated behind a Cargo feature (e.g. `mock-token`) that is **not** enabled
  in release build profiles — a mock token must be structurally unable to
  ship in a production build, not merely undocumented for one.
- Mock-issued keys/identities are marked as such wherever they surface
  (logs, test fixtures) so a mock-derived session can never be confused
  with a real hardware-backed one.

### 15.2 Two-emulator setup

- Run two AVD instances concurrently. On Android Emulator 36.5+, shared
  virtual Wi-Fi enables local Iroh peer connectivity. Older emulators: manual
  `adb forward` / host bridging.
- Default-dialer + `InCallService` + managed `ConnectionService` (§11) are
  testable in the emulator — grant `ROLE_DIALER` during test setup.
- Iroh peer-to-peer testing uses the shared Wi-Fi network and, separately,
  relay-fallback paths — no registrar or Flexisip instance required.

### 15.3 No audio hardware access required

Codec behavior (Opus encode/decode, framing, packet loss concealment) is
deterministic and already well-characterized — validating it does not
require live microphone/speaker hardware. Known synthetic input (e.g. a
fixed tone or deterministic bit pattern) is fed directly into the encoder on
one emulator; the decrypted, decoded output on the peer emulator is
compared against the expected result. This confirms the full pipeline
(handshake → frame AEAD → Iroh transport → decrypt → decode) without
emulator audio I/O passthrough.

### 15.4 Encryption/decryption test procedure

1. **Handshake success:** both emulators run mock-token `ephemeral-session`
   over Iroh; assert identical session key; UI transitions open → closed
   padlock (§13); assert `call_secure.wav` plays exactly once (logcat:
   `[Saga Connect Security Cue] Playing call_secure`).
2. **Encrypted media round-trip:** known synthetic input on emulator A;
   assert decrypted output on B matches expected session key.
3. **Never-encrypted-contact fallback:** no prior encrypted history; force
   handshake failure; assert key-not-found state, `call_unsecure.wav` plays
   once, no §5a modal.
4. **Downgrade-on-known-contact:** after step 1 history exists, force
   handshake failure; assert downgraded icon, `call_unsecure.wav` once, §5a
   modal, local log entry.
5. **Cellular call:** place `tel:` call on emulator; assert key-not-found
   treatment always (even if contact has Galdralag key), `call_unsecure.wav`
   once, no open padlock.
6. **Mid-call transition (Iroh):** simulate network-path change (§5b);
   assert re-handshake within 4 s or §5a behavior; §5b mid-call tone only
   if that separate setting is enabled — not the connect-time cue.
7. **CI integration:** headless AVDs; automated regression on handshake
   and media pipeline changes.
