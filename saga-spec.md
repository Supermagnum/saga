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
connection; media is Opus over `iroh-roq` with SRTP AEAD_AES_256_GCM.

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

- No in-call rekeying / ratcheting — one handshake per call (see §8). SRTP
  replay protection uses a sliding window (RFC 3711 §3.3.2); it is **not** a
  call-length ceiling. The pre-defined transform master-key lifetime is
  2⁴⁸ SRTP packets per stream (RFC 3711 §9.2 / default parameters) — far
  beyond practical call duration at VoIP rates.
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

### Media pipeline (Iroh)

```
Capture (AudioRecord VOICE_COMMUNICATION, 48 kHz mono PCM16)
   → disguise_pcm_inplace (no-op until Phase 5)
   → Opus encode (20 ms / 960 samples, VBR ~24–32 kbps, in-band FEC)
   → RTP + SRTP AEAD_AES_256_GCM  —or—  plain RTP (not-encrypted Iroh)
   → iroh-roq datagram (non-blocking; drop + count on full)
        ⋮  QUIC / relay
   → RoQ receive → adaptive jitter buffer (target ~40–60 ms, cap 300 ms)
   → Opus decode / PLC / FEC
   → Playback (AudioTrack USAGE_VOICE_COMMUNICATION)
```

**Adaptive jitter buffer:** playout delay is separate from loss detection.
The target starts low (~50 ms), tracks a high percentile of inter-arrival
deviation plus a small margin, grows on late arrivals / underruns, and
shrinks slowly when stable (shed excess depth one frame at a time). A
packet is lost only when its playout deadline passes while a hole remains
(then PLC/FEC). Empty underruns wait without advancing so a late packet is
not marked already-played. Hard cap **300 ms**.

**Latency:** mouth-to-ear on emulators uses a capture-side click marker
(`LATENCY_MARKER` inject + dump peak). Cross-emulator
`elapsedRealtimeNanos` clocks are not synced — report dump-frame / JB
`target_ms` and stage budgets rather than a single subtracted nanos delta.
Cross-correlation `lag_ms` from speech scoring is **not** a latency
measurement — it is the delay alignment used for corr/SI-SNR/STOI.
Emulator timings are **not** representative of phones.

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
5. **Media (Iroh):** Opus frames in RTP over `iroh-roq`. **Encrypted** calls
   use SRTP AEAD_AES_256_GCM (keys from the ephemeral-session shared secret
   via HKDF). **Not-encrypted** Iroh calls (after §5 step 4 / §5a continue)
   use **plain RTP inside RoQ** — QUIC still encrypts the transport via iroh
   endpoint keys, but without token authentication or SRTP; the UI stays
   "Not encrypted" and must not imply protection. Never mix: once a call is
   encrypted, a later failure takes the §5b path, never a silent switch to
   plain RTP. If the user ends the call at the §5a modal, no media at all.
   Cellular: carrier voice path.
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
| Key agreement | Token: ephemeral ECDH on **BrainpoolP256r1 or BrainpoolP384r1**; long-term key signs the ephemeral offer (ECDSA) | Matches Galdralag `ephemeral-session` (`docs/EPHEMERAL_SESSION.md`). Default profile **P384r1** (CESS preference). On-token TRNG for Galdralag; Nitrokey path may keep ephemeral sk in phone RAM (weaker; Phase 3) |
| Transcript / KDF | HKDF-SHA256 from ECDH shared secret + ordered-EPK salt as defined by `ephemeral-session` | Not Noise; not the draft §12 table. Output retained for Phase 2: `shared_secret` + `transcript_hash` + `profile_prk` (`HandshakeOutput`) |
| Saga channel binding | HKDF-Expand(`profile_prk`, info = `saga-iroh-channel-v1` \|\| min(EndpointId)\|\|max(EndpointId)) | **No wire change.** ephemeral-session does not bind Iroh EndpointIds; Saga derives `saga_session_key` so handshake material cannot be reused on a swapped QUIC identity pair (confused-deputy / endpoint-swap). Prevents applying a valid LT-authenticated transcript to the wrong Iroh path when LT keys are shared or confused across devices |
| Session cipher (media) | SRTP **AEAD_AES_256_GCM** (`webrtc-srtp` `AeadAes256Gcm`) when Encrypted — Phase 2 | Master key + salt from HKDF over Phase 1.5 `HandshakeOutput` (bound session key + transcript); separate keys per direction; `Zeroize`/`ZeroizeOnDrop`. QUIC encrypts the hop; SRTP is the end-to-end layer. Not-encrypted Iroh media is plain RTP (no SRTP): transport is still QUIC-encrypted by iroh endpoint keys, but without token authentication |
| Media framing | RTP over `iroh-roq` + SRTP (Encrypted) or plain RTP (not-encrypted Iroh) | Saga 1-byte media tag distinguishes modes (no silent mix). SRTP replay via sliding window (RFC 3711) |

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

| Crate / component | Used for in Saga | Matches firmware's use |
|---|---|---|
| Galdralag `ephemeral-session` (+ vault Brainpool ECDH/ECDSA) | Handshake wire format and session key derivation | Git-pinned crate rev (never a branch tip); same BrainpoolP256r1\|P384r1 + ECDSA as firmware |
| Saga `SessionToken` trait | Produce signed ephemeral offer / verify peer / derive shared secret | Fallible, async-safe, timeout-aware; mock implements now; Galdralag USB CCID and Nitrokey later. No USB backend in Phase 1.5 |
| `hkdf` + `sha2` | HKDF-SHA256 session / media key derivation + Saga EndpointId channel binding | Same crates as firmware |
| `aes-gcm` (via `webrtc-srtp` AES-256-GCM profile) | SRTP media AEAD | Firmware already lists `aes-gcm`; media uses it through `webrtc-srtp`, not a hand-rolled AEAD |
| `chacha20poly1305` | Reserved for §14 group-call frame encryption (SFrame-style), not two-party SRTP | Same crate as firmware |
| `blake3` | Where firmware/CESS uses BLAKE3 (e.g. outer-envelope labels); not a substitute for Brainpool ECDSA | Same crate |
| `zeroize` | Clearing session keys and derived secrets from memory on drop | Same crate, same pattern (`Zeroize`/`ZeroizeOnDrop`) |
| `subtle` | Constant-time comparison anywhere Saga compares secret-derived values | Same crate (`ConstantTimeEq`) |

Do **not** use `x25519-dalek` / `ed25519-dalek` for the Galdralag/Nitrokey
handshake path — the firmware protocol is Brainpool + ECDSA only. Mock-token
must speak the same `InitMessage`/`ResponseMessage` bytes (Phase 1.5).

`sha3`, `blake2`, `pbkdf2`, `hmac`, `p256`, `p384` remain in the firmware set
for other profiles; introduce them on the Saga side only when a named cipher
profile requires them.

`vsss-rs` (Shamir) is a firmware/vault concern, not a calling-app concern —
excluded from Saga's dependency list unless a future feature needs the phone
app itself to handle key shares.

### 7.2 Existing crates for needs the firmware doesn't cover

| Crate | Used for | Why not self-rolled |
|---|---|---|
| `webrtc-srtp` | RTP/SRTP packet framing, replay protection, per-packet IV (AES-256-GCM profile) | Getting SRTP framing subtly wrong by hand is a known footgun; use the existing pure-Rust implementation |
| `iroh` (1.x) | Dial-by-public-key, QUIC, NAT traversal / relay | Solves transport correctly; not something to reimplement |
| `iroh-roq` (fork/port to iroh 1.x) | RTP-over-QUIC media flows (RoQ semantics + ALPN) | Prefer n0's RoQ crate; port kept minimal for upstreaming — do not invent a parallel framing stack |

### 7.3 Explicitly cut

- **`snow` (Noise Protocol Framework)** — removed entirely, not just
  deferred. Since Galdralag-firmware already ships an authenticated
  ephemeral ECDH session protocol (`ephemeral-session` crate, documented in
  `docs/EPHEMERAL_SESSION.md`), adopting a second, independently-designed
  handshake framework (Noise) alongside it would itself be the kind of
  redundant, harder-to-audit surface this design criteria exists to avoid.
- Any bespoke SRTP, ICE, or DTLS code — covered by `webrtc`/`webrtc-srtp`.

### 7.4 §12 superseded

**§12's draft 3-message CBOR/X25519 table is retired.** Canonical handshake
is Galdralag-firmware `docs/EPHEMERAL_SESSION.md` / crate `ephemeral-session`
(Brainpool + ECDSA `InitMessage` / `ResponseMessage`). Saga transports those
bytes over Iroh. Media cipher choice (SRTP AES-256-GCM) is negotiated or
fixed outside that handshake as needed; do not reinvent the token session
protocol.

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
  re-handshake (§5b), not periodic rekey.
- [x] **SRTP lifetime (RFC 3711 corrected):** the replay list is a **sliding
  window** for detecting replays (RFC 3711 §3.3.2). The ~2¹⁵ figure in the
  ROC/index discussion is about receiver synchronization robustness under
  extreme loss/reordering, **not** a key lifetime. Default master-key
  lifetime is **2⁴⁸ SRTP packets** per stream (RFC 3711 §9.2). At 50
  packets/s that is far longer than any practical call — **do not implement
  a 15–20 minute single-key call-length ceiling.** Revisit only if a future
  profile shortens the lifetime or long multi-day sessions appear.

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

## 12. Handshake wire format (superseded draft)

**Canonical:** Galdralag `ephemeral-session` over Iroh after QUIC
establishment — `InitMessage` / `ResponseMessage` as specified in
Galdralag-firmware `docs/EPHEMERAL_SESSION.md` (§7.4), framed as
`u16 BE length || serialised message` on the Saga bi-stream. Message
bytes are crate-identical to firmware.

**Saga-only addition (allowed by §7.4):** after both sides derive
`HandshakeOutput` (including EndpointId channel binding, §6), they exchange
confirmation MACs on the same bi-stream so a relayed/swapped-path handshake
cannot report Encrypted:

| # | Direction | Fields | Purpose |
|---|---|---|---|
| 1 | A → B | `u16 BE len \|\| InitMessage` | ephemeral-session offer |
| 2 | B → A | `u16 BE len \|\| ResponseMessage` | ephemeral-session response |
| 3 | A → B | `confirm_mac_a:32B` | HMAC-SHA256(`confirm_key`, `saga-confirm-initiator-v1` \|\| `transcript_hash`) |
| 4 | B → A | `confirm_mac_b:32B` | HMAC-SHA256(`confirm_key`, `saga-confirm-responder-v1` \|\| `transcript_hash`) |

`confirm_key` = HKDF-Expand(`profile_prk`, info =
`saga-iroh-confirm-v1` \|\| min(EndpointId)\|\|max(EndpointId)) — dedicated
output, **not** the media/`saga_session_key`. Compare with `subtle`
constant-time equality. Security state may only resolve to Encrypted after
both MACs verify, inside the §8 2 s / 4 s budget. Mismatch → handshake
failure → key-not-found or §5a as usual.

The historical draft CBOR/X25519 table below is **retired**:

| # | Direction | Fields | Purpose |
|---|---|---|---|
| ~~1~~ | ~~A -> B~~ | ~~`version:u8`, `ephemeral_pubkey`, `signature`, `nonce_a:16B`~~ | ~~Offer~~ |
| ~~2~~ | ~~B -> A~~ | ~~`ephemeral_pubkey`, `signature`, `nonce_b:16B`, `profile_choice:u8`~~ | ~~Counter-offer~~ |
| ~~3~~ | ~~A -> B~~ | ~~`transcript_mac:32B`~~ | ~~Mutual confirmation~~ |

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
| **Downgraded (warning)** | **Iroh call only.** Key resolves or prior encrypted history, handshake failed | Broken-padlock — [`icons/broken.svg`](icons/broken.svg); plus §5a modal + `call_unsecure.wav` |

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

### 14.1 Carrier conference vs encrypted groups

**Carrier conference (cleartext stock parity — allowed):** as default dialer,
Saga hosts multi-call UI for cellular calls (second incoming, add call,
hold/swap, merge/split when Telecom capabilities allow). Audio mixing stays
on the carrier network; the InCall padlock stays key-not-found / "Not
encrypted". This is an unencrypted feature for dialer parity, not an
encrypted group call.

**Never mix Iroh ↔ carrier in one conference:** hide Merge and refuse
programmatic `conference()` / `mergeConference()` when any leg is Iroh.
Hold/swap across origins is allowed (Iroh media pauses on `STATE_HOLDING`).

**Why carrier merge cannot provide E2E encryption:** carrier-side mixing
gives the carrier cleartext access to the mixed stream. Encrypted group
calls must be handled entirely at the app/data layer (Iroh-only later),
following the pattern used by Signal and Messenger:

- **SRTP/DTLS** secures the hop to whatever relay forwards media.
- **A second, frame-level encryption layer** (SFrame-style) secures the
  actual audio/video content end-to-end regardless of what the relay does —
  the relay only ever forwards ciphertext frames, never decrypts them.
- **Sender Keys** distribute a per-participant symmetric key once, over
  existing pairwise sessions (O(N) key distribution), rather than
  re-encrypting media per-recipient.

### 14.2 Mesh vs. relay threshold

- **Small groups (roughly ≤4–5 participants):** full mesh, no relay. Each
  device runs the ephemeral-session pairwise handshake with every other
  participant (O(N²) handshakes) and sends/receives N−1 direct SRTP streams.
  No new key-management scheme needed — pure extension of the two-party design.
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

The mock token **is** a `SessionToken` backend: it calls
`InitiatorSession` / `ResponderSession` from the git-pinned
`ephemeral-session` crate and emits byte-identical `InitMessage` /
`ResponseMessage` serialisations. Long-term keys are software Brainpool
ECDSA (default P384r1); ephemeral ECDH runs in-process via `FakeTrng`,
mirroring on-device steps. There is no dual path and no leftover
X25519/Ed25519 hello/confirm layout.

Contact MIME rows store the 64-char lowercase hex fingerprint
(`InitMessage::encode_fingerprint_hex`). Iroh EndpointIds remain separate
(`saga-dev-v1:{phone-digit-label}`).

Scope is enforced at build time, not by convention:

- Lives in `testing/mock-token`, not alongside future USB token backends.
- Gated behind Cargo feature `mock-token` (not for production-shaped
  release hygiene — Phase 6).
- Logs use marker `MOCK_TOKEN`; debug builds may log only a short hash
  prefix of the session key, never raw secrets.
- Host tests: committed vector
  `testing/mock-token/tests/fixtures/p384_vector_c1_d1.json` plus negative
  cases (tampered sig, wrong LT key, wrong curve, truncated, replay).

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

## 16. Phase 5 — Optional voice disguise (off by default)

Optional **voice disguise** for Iroh (Saga) calls only. Off by default. It
changes how the local mic feed sounds to the peer (and in any recording they
make). It is **not** anonymisation: UI and docs must not claim the speaker
cannot be identified. The contact key still identifies the caller; accent,
word choice, and speaking rhythm are unchanged. It does not apply to
cellular / phone-network calls.

Apply on the **sender** side before Opus encoding. It never changes what the
local user hears. Candidate modes for shipping are pitch shift and
McAdams-coefficient LPC formant shift (VoicePrivacy Challenge baseline);
neural voice conversion is evaluated for latency/cost and is not assumed to
ship. Any library must be license-compatible with AGPL-3.0 and F-Droid.
Added latency budget per mode: **≤ ~20 ms** on the emulator — modes that
exceed that do not ship. Measure and record latency in `TEST_RESULTS.md`.

### 16.1 Settings → Voice disguise (global defaults)

- **Mode:** Off (default) / Pitch shift / Formant shift (McAdams) /
  Formant + pitch.
- **Pitch amount:** slider in semitones, −6 … +6; default −3 ("lower"), or
  whichever value listening tests prefer. Shown only for modes that use
  pitch.
- **Formant strength:** Mild / Medium / Strong, mapped to McAdams α ranges
  tuned by listening tests and the intelligibility metric. Shown only for
  formant modes.
- **"New random voice each call":** ON by default. A random α (and a random
  pitch offset inside the chosen range) is drawn from a CSPRNG once per
  call and zeroized at call end. Turning it OFF uses a fixed α and must
  show the warning: "A fixed setting is easier to reverse."
- **"Test my voice":** records ~5 s locally and plays it back through the
  chosen mode. Nothing leaves the phone; the recording is deleted
  afterwards.
- **Help text (short):** what it does (changes how your voice sounds to the
  other person and in any recording they make); what it does not (it does
  not hide accent, words, rhythm, or who you are — your contact key
  identifies you); and that it works on Saga calls only, not normal phone
  calls.

### 16.2 Per-contact override (contact detail screen)

- **Voice disguise:** Use default / Always on / Always off.

### 16.3 In-call screen (Iroh calls only)

- A quick on/off toggle using the mode from settings or the contact
  override. It may be switched mid-call; the change applies at the next
  audio frame boundary with a short crossfade (no clicks).
- A visible indicator chip while disguise is active (local side).
- For cellular calls: the toggle is shown disabled with
  "Not available on phone-network calls".
- **Peer chip (decided):** when the remote party has disguise active, show
  a neutral informational chip: "Caller is using voice disguise". See
  §16.6. It must not change the padlock or security state.

### 16.4 Behaviour

- Sender-side only, before Opus encoding; local sidetone / what you hear is
  unchanged.
- Settings persist across calls; per-call random α / pitch offsets do not.
- Latency for each mode is measured and shown in `TEST_RESULTS.md`. A mode
  that exceeds ~20 ms on the emulator does not ship.

### 16.5 Verification

- **Host:** each mode × strength on the real speech clip → before/after
  WAVs, latency, and intelligibility metric versus the original.
- **Randomisation:** two calls with "New random voice each call" ON must
  yield different α values; those values must never be logged in release
  builds.
- **Emulator screenshots** (look at each before accepting): settings page
  for each mode; contact override; in-call toggle + active chip; peer
  "Caller is using voice disguise" chip; disabled cellular-call state.

### 16.6 Decisions and open items

- [x] **Peer "voice disguise" chip — YES, tell the peer.** Add a field to
  the **Saga call-setup messages** (not the ephemeral-session wire format)
  that indicates disguise is active. Send it when the call starts and
  whenever disguise is toggled mid-call. The peer shows a neutral chip:
  "Caller is using voice disguise". Informational only — must not change
  the padlock or security state. Implementation stays in Phase 5.
- [ ] Confirm chosen DSP libraries' licenses against AGPL-3.0 and F-Droid
  before dependency lock-in.
- [ ] Report neural voice-conversion latency/cost on mid-range targets;
  exclude from shipping modes if over the ~20 ms budget or otherwise
  unsuitable.
