# Saga — Encrypted Voice Calling (Preliminary Spec)

**Status:** Draft / pre-implementation
**Scope:** Android (primary), Linux (secondary peer)

## 1. Summary

Saga lets two phones make what looks and behaves like a normal cellular call,
while transparently negotiating end-to-end encryption over the data path
whenever both ends support it. If the peer doesn't support Saga, or no data
connectivity is available, the call falls back silently to a normal
unencrypted cellular call. Encryption is never required, never blocks the
call, and is not visible as a separate "app" experience — it rides inside
the native dialer UI.

## 2. Goals

- Calls appear in the native Android in-call UI, call log, and Bluetooth/car
  integrations — indistinguishable from a carrier call.
- Encryption activates automatically when both peers support it; otherwise
  the call proceeds as a normal unencrypted call with no user action needed.
- Key exchange uses the hardware token's on-device ephemeral ECDH + signature
  primitive — the long-term key never touches key agreement, only signs the
  ephemeral offer (forward secrecy per call).
- Audio transport uses the existing data channel (LTE data / WiFi), not the
  carrier voice codec — no acoustic modem, no voice-band bitrate ceiling.
- Handshake latency stays inside the existing call-setup window
  (dial → ringback → connected), which users already tolerate as multi-second.

## 3. Non-goals (this iteration)

- No in-call rekeying / ratcheting — one handshake per call.
- No group calls.
- No app-native calling UI on Android — must use system Telecom integration.
- Linux side is a SIP/RTP peer only; no illusion of a "native OS dialer"
  since none exists to blend into.

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
voice call, Saga plays no role.

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
4. **No response / timeout:** call proceeds as normal unencrypted cellular
   call. No error shown to user beyond a neutral "not encrypted" indicator.
5. **Media:** RTP audio encrypted via negotiated AEAD (default:
   ChaCha20-Poly1305, one key per call, frame counter as nonce).
6. **UI state:** in-call UI shows "securing…" until handshake completes,
   then flips to "encrypted." If handshake fails after connect, flips to
   unencrypted rather than dropping the call.

## 6. Cryptography

| Stage | Primitive | Notes |
|---|---|---|
| Key agreement | Token: ephemeral ECDH, long-term key signs ephemeral offer | On-device TRNG, forward secret per call |
| Transcript binding | HKDF `info` = hash of handshake transcript | Hand-rolled 3-message handshake, not full Noise — revisit if session shapes grow (see §8) |
| Session cipher | ChaCha20-Poly1305 (default profile) | Per-RTP-frame, nonce derived from sequence number |
| Media framing | SRTP | Handles replay protection, per-packet IV derivation |

Cipher profile selection reuses the token's existing named-profile concept
(`standard`, `conservative`, etc.) for *configuration*, not layering — voice
uses a single fast AEAD per frame, not cascaded ciphers, due to latency
sensitivity.

## 7. Rust crates (working list)

- `webrtc-srtp` / `webrtc` — RTP/SRTP framing over the data path
- `chacha20poly1305` (RustCrypto) — session cipher
- `x25519-dalek` / `p256` — host-side crypto if any operations happen off-token
- `snow` — deferred; add only if the handshake needs to grow beyond the
  fixed 3-message signed-ECDH pattern (e.g. supporting multiple DH modes)

## 8. Open questions

- [ ] Measured token APDU latency for one ECDH + one sign operation
      (end-to-end, both directions) — determines handshake budget.
- [ ] Confirm handshake reliably completes inside call-setup window on both
      VoLTE and VoWiFi paths; VoWiFi adds IPsec/IMS tunnel overhead on top
      of the same IMS stack.
- [ ] Decide fallback UX wording/iconography for "not encrypted."
- [ ] Rekeying policy for abnormally long calls (out of scope for v1, but
      note the decision).
- [ ] Linux peer: SIP/RTP client scope — full softphone, or reference
      implementation only?

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
