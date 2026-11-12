# Saga Test Results

**Last updated:** 2026-11-12 (Phase 2 Step 3b — Android RoQ / PR #27 verification)

**Status:** Step 3b Android capture/playback uses shared `saga-media` over iroh-roq (real network media; loopback removed). E2E Cases 1–6 + 4b PASS on API 35 after media-regex + Case 2 cue/media poll fixes. PCM inject/dump tooling added for speech metrics. Emulator↔emulator RoQ speech fidelity still open (inject OK, decoded dump noise). Emulator↔saga-peer WAV blocked on relay URL advertisement (`127.0.0.1` inside emulator). Do not merge until open items below are decided.

---

## 2026-11-12 — Phase 2 Step 3b: Android RoQ verification (#27)

### What is real network audio vs not

| Path | Real network RoQ? | Result |
|------|-------------------|--------|
| Host `saga-peer` A↔B WAV (`host_roundtrip`) | Yes (in-process iroh, no loopback) | **corr=0.961 SI-SNR=11.1 dB** (encrypted + plain) |
| Emulator↔emulator PCM inject/dump | Yes (two API 35 emulators + local relay) | Inject pre-network **corr=1.000**; decoded dump **corr≈0.07–0.11**, high-ZCR noise (not speech) |
| Emulator↔saga-peer WAV (laptop `15550100020`) | Intended real network | **Blocked**: host peer advertises relay `http://127.0.0.1:3340/`; emulator tries that address and gets connection refused (needs `10.0.2.2` or LAN IP both can use) |
| Local AudioRecord mic (no inject) | N/A | Not used for scored metrics |
| CI `LIVE_AUDIO` | Off | Host WAV only |

### Emulator↔emulator PCM metrics (API 35, real RoQ)

Harness: `testing/e2e/run-pcm-roq-metrics.sh` + `pcm_score.py` (broadcast `TEST_PCM_*`, app-files staging).

| Direction | corr | SI-SNR | STOI | Notes |
|-----------|------|--------|------|-------|
| alice→bob (decode on bob) | 0.105 | −19.5 dB | 0.14 | Energy present; spectral centroid ~11 kHz (noise-like) |
| bob→alice | (short/failed media wait on reverse) | — | — | One run: dump near-silence |
| inject_verify (alice capture, pre-network) | **1.000** | — | — | Proves inject→JNI capture path |

Host baseline for same fixture over RoQ: corr **0.961**, SI-SNR **11.1 dB**.

### E2E (API 35, `/tmp/e2e-full-3b-rerun.log`)

| Gate | Result |
|------|--------|
| Cases 1–6 + 4b + Phase 3 cue grep | **PASS** (`E2E_FULL_EXIT=0`) |
| Case 2 / 2b media | **PASS** (escaped `\[Saga Media\]` regex + polled cue/media waits; handshake ~350–393 ms) |
| PhoneAccount onboarding | **Partial**: Case A/B PASS; `INBOUND_REJECT:FAIL` missing reject log (flake); `ONBOARDING_EXIT=1` |

### In-call screenshots

Looked at `docs/screenshots/step3/` (`incall_base`, muted, speaker, held). Fresh mid-call captures during PCM runs often showed MainActivity or “Securing… / Audio idle” because test intents / task focus cover `SagaInCallActivity`. Broadcast PCM path avoids focus steal for audio; reliable Encrypted+live-audio UI capture still needs a dedicated foregrounding helper.

### Tooling added (this verification)

- `SagaPcmDebug` + `SagaPcmDebugReceiver` (inject WAV / dump playback without loopback)
- `testing/e2e/run-pcm-roq-metrics.sh`, `pcm_score.py`, `run-peer-wav-roundtrip.sh`
- Case 2/2b: wait for secure cue + media start (both sides)

### Remaining open decisions

1. **Emulator RoQ speech fidelity** — why decoded PCM is noise while host A↔B is clean (SRTP/role, datagram path, or emulator network). Inject path is ruled out.
2. **Emulator↔host peer relay URL** — advertise a relay URL reachable from both host and emulator (not `127.0.0.1` inside the guest).
3. **Onboarding `INBOUND_REJECT` flake** — whether to harden reject logging or treat as known flake.
4. **Merge #27?** — hold until (1) and preferably (2) have an explicit accept/defer.

---

## 2026-11-12 — Phase 2 Step 3: Android audio I/O + Telecom

### Pre-Step-3 checks (#23)

| Item | Result |
|------|--------|
| LibriSpeech fixture | **Committed** at `saga-media/fixtures/speech_clip_48k.wav` + `ATTRIBUTION.md` (CC BY 4.0). Tests load the local WAV — no OpenSLR network fetch. |
| Warm-cache GHA rust job | **~177 s** on attempt 2 of [run 43225104217](https://github.com/Supermagnum/saga/actions/runs/43225104217/job/131211748804) (`rust (fmt / clippy / test)` 13:22:56Z→13:25:53Z) |

### Audio API choice

**AudioRecord + AudioTrack** with `VOICE_COMMUNICATION` / `USAGE_VOICE_COMMUNICATION`, PCM via JNI direct `ByteBuffer`s.

| Option | Why not / why |
|--------|----------------|
| **AudioRecord/AudioTrack (chosen)** | Session IDs for `AcousticEchoCanceler` / `NoiseSuppressor` / AGC; matches Telecom-managed call audio; no per-frame heap alloc; `THREAD_PRIORITY_URGENT_AUDIO` |
| Oboe | Extra NDK surface; platform AEC attach is still AudioRecord-session oriented for voice-comm |
| cpal | Reserved for Linux peer (spec §10); weak VOICE_COMMUNICATION / Telecom integration on Android |

### Platform AEC/NS (emulator probe)

Instrumented `VoiceEffectsProbeTest` on `sdk_gphone64_x86_64` (API 35):

```
aecAvailable=false aecAttached=false nsAvailable=false nsAttached=false agcAvailable=false agcAttached=false
```

Engine still starts on `VOICE_COMMUNICATION` and logs attach results at runtime. Physical devices typically report available=true.

### Telecom

- Mute: `InCallService.setMuted` + engine gates send (capture/playback streams stay up)
- Route: `CallAudioState` / `setAudioRoute`; API 34+ `requestCallEndpointChange` when endpoints known
- Hold: `Connection.onHold` / `onUnhold` + `Call.STATE_HOLDING` → pause Iroh media; cellular can hold/resume an Iroh call
- Media starts only after security resolves (Encrypted→SRTP mode 1; NeverEncrypted / §5a Continue→plain RTP mode 2)

### §5b re-key

Media **keeps flowing** during §5b re-handshake: the call is not paused for the
crypto round-trip. On successful re-key, `rekey_srtp` installs new send/receive
SRTP contexts immediately and keeps the previous **receive** context
(`decrypt_prev`) briefly so in-flight packets on the old keys still decrypt.
Send switches to the new keys at the swap; there is no “hold media until
handshake completes” gate.

Host test `tests/rekey.rs` measures only the **key-swap** duration
(**rekey_swap_us ≈ 6–12 µs**). That is not the audible gap. **Step 5 Case 8**
must measure the gap in **decoded PCM at the receiver** across the re-key
(target under ~100 ms), not the key-swap timer.

### §16 disguise

`disguise_pcm_inplace` remains a no-op before Opus encode.

### In-call UI screenshots (looked at)

`docs/screenshots/step3/`: `incall_base.png`, `incall_muted.png` (Muted / UNMUTE), `incall_speaker.png` (Muted · Speaker / EARPIECE), `incall_held.png` (Muted · Speaker · On hold / RESUME).

### E2E

| Gate | Result |
|------|--------|
| Cases 1–6 + 4b + Phase 3 | **PASS** `/tmp/e2e-step3-full2.log` |
| Case 5a/5b mid-call rekey | **PASS** (rekey_swap_us≈1 µs on device bridge; host pipeline ≈6–12 µs) |
| PhoneAccount onboarding | **PASS** `/tmp/e2e-onboarding-step3.log` |

Root cause for Case 5 regression: JNI pull wrote PCM without advancing `ByteBuffer` position; `flip()` then `get()` threw `BufferUnderflowException` on `saga-playback`, killing the process before mid-call rehandshake.

---

## 2026-11-12 — Phase 2 Step 2 follow-ups (LibriSpeech, STOI, CI, unsecured)

### Speech fixture (LibriSpeech CC BY 4.0)

| Field | Value |
|-------|--------|
| File | `saga-media/fixtures/speech_clip_48k.wav` |
| Utterance | `5694-64038-0005` (`dev-clean-2`, OpenSLR 31) |
| Transcript | HE WALKED UP AND SAYS HELLO BOYS WHAT IS IT BOSS |
| Attribution | `fixtures/ATTRIBUTION.md` + NOTICE |

espeak fixture **not kept** — LibriSpeech is the CI gate; espeak only useful as a zero-download smoke generator when OpenSLR is unreachable (do not commit).

### Re-baselined speech CI gates

| Test | corr | SI-SNR | Gate |
|------|------|--------|------|
| Tone full pipeline | 0.987 | 15.7 dB | corr ≥ 0.85, SI-SNR ≥ 15 dB |
| LibriSpeech speech | **0.961** | **11.1 dB** | corr ≥ 0.85, SI-SNR ≥ 10 dB |

### STOI (local, not CI) — pystoi MIT

Speech-mode STOI at 16 kHz on the LibriSpeech clip after Opus/SRTP/JB path:

| Condition | STOI |
|-----------|------|
| clean | **0.9312** |
| 5% loss | **0.8847** |
| 15% loss | **0.8436** |

**Exact command (reuse in Phase 5):**

```bash
cd saga-media
cargo test --test stoi_export -- --ignored --nocapture
python3 scripts/stoi_eval.py \
  --ref fixtures/speech_clip_48k.wav \
  --deg target/stoi/deg-clean.wav \
  --deg target/stoi/deg-5pct.wav \
  --deg target/stoi/deg-15pct.wav \
  --labels clean,5pct,15pct
```

Deps: `pip install pystoi soundfile numpy`. ViSQOL not used (binary unavailable); STOI chosen instead.

### CI speed (saga-media)

| | Wall |
|--|------|
| Cold (`cargo clean` then `cargo test -j 2`) | **354 s** (local; dominated by libopus cmake + dep compile) |
| Warm (target populated, `[profile.test] opt-level = 2`) | **~6.8 s** |

CI: Swatinem rust-cache `shared-key: saga-rust-host-v2` + dedicated `actions/cache` on `saga-media/target` keyed by `Cargo.lock`.

### Unsecured Iroh media decision

Plain RTP inside RoQ after not-encrypted resolution (QUIC still encrypts transport; no token/SRTP). Spec §5/§6 updated. Host tests in `tests/unsecured.rs` (policy, §5a decline, no silent downgrade, plain↔SRTP tag reject).

---

## 2026-11-12 — Phase 2 Step 2: saga-media host pipeline (initial)

### Crate / stack

| Item | Choice |
|------|--------|
| Crate | `saga-media/` (host-testable; not inside `saga-iroh-core`) |
| Opus | `opus` **0.3.1** + `audiopus_sys` (cmake/cc from source — F-Droid-safe; same stack as iroh-roq tests). 48 kHz mono, 20 ms, VBR **32 kbps**, in-band FEC on, **DTX off**, `Application::Voip`, complexity 10 |
| SRTP | `webrtc-srtp` **0.17.3** `ProtectionProfile::AeadAes256Gcm` |
| RoQ | fork `c9b02963…` — `try_send_payload` (non-blocking; never `send_rtp_wait` on media path), `read_bytes`, `max_datagram_size` |
| Keys | HKDF-SHA256 over `saga_session_key`; info = direction label + `transcript_hash`; labels `saga-srtp-caller-to-callee-v1` / `saga-srtp-callee-to-caller-v1` |

### Loss / security / limits

| Test | Result |
|------|--------|
| 5% loss + jitter/reorder | corr ≈ 0.99; JB bounded |
| 15% loss + jitter/reorder | corr ≈ 0.77 (≥ 0.30); no panic |
| Tamper / replay / wrong-key | counters; all wrong-key rejected |
| Tiny `max_datagram_size` | `DatagramTooLarge`; `media_failed` |
| Forced blocked drops | `datagram_blocked_drops` |

---

## 2026-11-12 — Phase 2 Step 1: iroh-roq → iroh 1.x

- Fork: https://github.com/Supermagnum/iroh-roq @ `408d8ccccd3c6ee6a44d7da4a960d34e0c36e971` (superseded by `c9b02963…` in Step 2)
- Upstream draft (not opened): `docs/upstream/iroh-roq-port.md`
- Source diff vs upstream: +118 / −48 (7 files, excl. lock)
- Fork tests: datagram (10 + 1000), stream, opus — PASS
- Case 6 RING flake tracked: https://github.com/Supermagnum/saga/issues/20 (0/3 in dedicated full runs; 1 prior Step0 FAIL)

---

## 2026-11-12 — Phase 2 Step 0: saga-session production crate

### Layout

| Crate | Role |
|-------|------|
| `saga-session/` | `SessionToken`, `HandshakeOutput`, channel binding, confirmation MAC + stream driver |
| `testing/mock-token/` | `MockSessionToken`, `MockIdentity`, synthetic media probe (debug/E2E) |
| `saga-iroh-core/` | Depends on `saga-session` always; `mock-token` feature for debug/E2E only |

Release Cargo features: `--features iroh-transport` (no `mock-token`, no iroh `test-utils`). Debug/E2E: `iroh-transport,mock-token`.

`cargo tree -e normal --features iroh-transport`: includes `saga-session` + `ephemeral-session` + `galdr-core`/`galdr-vault` (firmware types for production handshake). **No** `mock-token`. `vsss-rs` appears via `galdr-vault` (firmware dep, not the mock).

Release arm64 `.so` hygiene (`scripts/check-release-so.sh`): absent `MockSessionToken`, `saga-dev-v1`, `insecure_skip_verify`, `saga-synthetic-audio-v1`.

### Expected release behaviour

No production `SessionToken` backend wired yet (FakeTrng / USB token still behind `mock-token` or future Galdralag) → every Iroh call handshake fails → "not encrypted" + `call_unsecure` (and §5a when prior encrypted history). Documented; not a regression of the UX rules for when a token *is* present (debug/E2E with mock).

### Emulator E2E (API 35, after move)

| Case | Result | Notes |
|------|--------|-------|
| 1–5, 2b, 2c, 4b | PASS | Full suite `/tmp/e2e-phase2-step0.log` |
| 6 | PASS | Suite timed out RING once (flake after Case 5); standalone re-run PASS `/tmp/e2e-case6-retry.log` |
| Phase 3 | PASS | |
| Handshake budget | PASS | case2 372 ms; case2b 336 ms (≤2 s target) |

---

## 2026-11-12 — Pre-merge #16 (confirmation MAC, deps, NOTICE)

### Confirmation MAC (spec §12)

After ephemeral-session key derivation + EndpointId channel binding, both sides exchange HMAC-SHA256 confirmation MACs on the Iroh bi-stream (`confirm_key` from dedicated HKDF info `saga-iroh-confirm-v1`, role labels `saga-confirm-initiator-v1` / `saga-confirm-responder-v1`). Compare with `subtle` constant-time. Security state reaches Encrypted only after both MACs verify (inside §8 2 s / 4 s). Mismatch → `TokenError::ConfirmationFailed` → key-not-found / §5a.

| Host test | Result |
|-----------|--------|
| Same handshake, different EndpointId sets | confirmation fails |
| Tampered MAC / wrong role | `ConfirmationFailed` |
| `force_binding_perturb` on one side | both fail |

#### a) Emulator fault-injection (Case 4b)

Debug-only `force_binding_perturb` SharedPreferences on one side (perturbed
EndpointId binding input). Expected: confirmation MAC fails → NOT Encrypted,
`call_unsecure.wav` once, §5a modal for a previously-encrypted contact.

Re-run log: `/tmp/e2e-phase15-budget.log` — **PASS** (see Full E2E below).

### Zeroize + debug-only hash-prefix logs

- `HandshakeOutput` / `ConfirmationKey`: `Zeroize` + `ZeroizeOnDrop` (confirmation key zeroised after successful exchange).
- Hash-prefix logs are compiled only under `#[cfg(debug_assertions)]` — sites in `testing/mock-token/src/handshake_io.rs`, `mock_session.rs`, and `saga-iroh-core/src/iroh_transport.rs`. Release/`ci` profiles omit them; E2E relies on media round-trip.

### Contact-key migration

No Room schema. Debug builds: `TestContactSeeder` delete+reinsert bob/alice/thor on launch (replaces legacy phone-label DATA1 with 64-hex fingerprints). Documented on `ContactKeyRepository`. Production: wipe app data / clear Contacts manually if upgrading a debug install with label rows.

### Dependencies (galdr-vault / galdr-core)

Not type-only helpers. `ephemeral-session` alone is insufficient for the software mock: `InitiatorSession`/`ResponderSession` need `HardwareTrng` + `VaultStorage`, and LT Brainpool key gen uses `galdr-vault`. `galdr-core` with `test-hal` supplies `FakeTrng` / `FakeVaultStorage` (not re-exported by `ephemeral-session`). Keep both direct deps on `mock-token`.

`cargo tree -e normal --features iroh-transport` (production-shaped; no `mock-token`):

```
saga-iroh-core
├── blake3 …
├── iroh …
├── jni …
├── log …
└── tokio …
```

**`vsss-rs` is not in that tree.** It appears only when `--features mock-token` pulls `galdr-vault` (E2E / current Android build still enables `mock-token` until Phase 6 hygiene).

### arm64-v8a release link

```bash
cd saga-iroh-core && cargo ndk -o /tmp/saga-ndk-release -t arm64-v8a \
  build --release --features iroh-transport,mock-token
```

Result: `libsaga_iroh_core.so` — ELF aarch64 shared object, links (`liblog`/`libdl`/`libm`/`libc`), exports JNI including `nativeSetForceBindingPerturb`.

### NOTICE

Root `NOTICE` lists Galdralag GPL-3.0, iroh, RustCrypto, blake3, icons AGPL, Android/Jetpack. In-app licences screen deferred to Phase 6.

### Onboarding / guard suite (API 35 re-run)

```bash
SERIAL_PA=emulator-5558 SERIAL_PEER=emulator-5556 \
  bash testing/e2e/run-e2e-phone-account-onboarding.sh
```

Log: `/tmp/e2e-pa-api35-rerun.log`. **PASS** — BLOCK / SETTINGS_UI enable / CASEB / URI `tel:,` / inbound reject while disabled / caller not Encrypted.

### P256r1 vector

Committed: `testing/mock-token/tests/fixtures/p256_vector_c0_d0.json` (FakeTrng `0xC0`/`0xD0`) alongside `p384_vector_c1_d1.json`. Host `vectors.rs` covers both.

### Full E2E (API 35, with Case 4b) + handshake budget

```bash
SERIAL_A=emulator-5554 SERIAL_B=emulator-5556 bash testing/e2e/run-e2e-full.sh
```

#### b) Re-run after key confirmation (API 35)

Log: `/tmp/e2e-phase15-budget.log` — **PASS** Cases 1–6 + Case 4b + Phase 3.

| Case | Result |
|------|--------|
| 1–3, 5–6 | PASS |
| 4 | PASS (force_fail downgrade) |
| **4b** | **PASS** — binding perturb → confirmation MAC failed → not Encrypted → `call_unsecure` once → §5a (previously-encrypted contact; modal title asserted when UI dump available) |

Handshake wall time (InitMessage → `Handshake settled to [Encrypted]`, includes confirmation MACs). Spec §8: **2 s target / 4 s hard**:

| Direction | Time | Budget |
|-----------|------|--------|
| Case 2 alice→bob | **346 ms** | PASS ≤2 s target / ≤4 s hard |
| Case 2b bob→alice | **328 ms** | PASS ≤2 s target / ≤4 s hard |

Harness: `record_handshake_budget` in `testing/e2e/e2e-common.sh`.

---

## 2026-11-12 — Phase 1.5 ephemeral-session in mock-token

### Pin

| Field | Value |
|-------|--------|
| Repo | `https://github.com/Supermagnum/Galdralag-firmware` |
| Rev | `b781bf6fa5ccf8700a5fb061583c31cfa00241a1` |
| Date | 2026-10-22 |
| Crates | `ephemeral-session`, `galdr-core` (`test-hal`), `galdr-vault` |
| Licence | GPL-3.0 (compatible with Saga AGPL-3.0) |
| Host build | `cargo test` in `testing/mock-token` — OK |
| Android ABIs | `cargo ndk -t arm64-v8a -t x86_64` smoke of the same crates — OK |

Default curve: **BrainpoolP384r1**. Both P256r1 and P384r1 covered by host tests.

Channel binding: Saga HKDF over `profile_prk` + sorted Iroh EndpointIds (`saga-iroh-channel-v1`); no wire change. See saga-spec.md §6.

### Host verified

```bash
cd testing/mock-token && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
cd saga-iroh-core && cargo clippy --all-targets --features iroh-transport,mock-token -- -D warnings && cargo test --features iroh-transport,mock-token
```

Test vector source: `testing/mock-token/tests/fixtures/p384_vector_c1_d1.json` (FakeTrng `0xC1`/`0xD1`, labels `vector-alice`/`vector-bob`). Firmware citation: `ephemeral-session/tests/session_protocol.rs` and `examples/export_session_kdf_vectors.rs` on the same rev.

### Emulator verified (API 35)

```bash
SERIAL_A=emulator-5554 SERIAL_B=emulator-5556 bash testing/e2e/run-e2e-full.sh
```

Cases 1–6 + Phase 3 **PASS** (`/tmp/e2e-phase15-full.log`). Case 2 media round-trip OK; session_key_hash_prefix matched on both peers (e.g. `6c8e`).

### CI

PR: https://github.com/Supermagnum/saga/pull/16  
Green run: https://github.com/Supermagnum/saga/actions/runs/43155401742 — rust ~2m8s, android ~1m51s (wall ~2m).

### Needs real hardware / blocked on xous-core#937

- a real Galdralag handshake over USB CCID on Android
- byte-identity of real-token output vs the software mock
- the §8 handshake latency benchmark (T_generate + T_sign + T_verify + T_ecdh)
- PIN/unplug/timeout behaviour with a real token

### Known gaps before Phase 2

- No Opus / iroh-roq / SRTP media path yet (consumes `HandshakeOutput`)
- Release still carries `test-utils` / `mock-token` / `TEST_*` (Phase 6 hygiene)

---

## 2026-11-12 — PhoneAccount security fix (#2)

Treated as a security bug: disabled Saga `PhoneAccount` must never let an Iroh dial fall through to SIM telephony.

### Fix summary

| Area | Change |
|------|--------|
| Gate | `SagaDialer` refuses `placeCall` unless account registered **and** enabled; launches onboarding |
| URI | Managed account uses `tel:,` (PAUSE). After `convertKeypadLettersToDigits` + `stripSeparators` there are **no ASCII digits 0-9**. Legacy `tel:iroh` → `4764` / modem `ATD4764` — must not be used. `tel:@` rejects even enabled Saga dials (`ErrorDialogActivity`). `tel:+` triggers modem USSD `AT+CUSD`. Evaluated: `saga:` placeCall does **not** invoke ConnectionService on API 35; `tel:` + explicit handle does. Removed `+15550100999` placeholder |
| SIM abort | `SagaInCallService` disconnects any non-Saga account call that follows a recent Iroh dial (`IROH_ABORT_SIM_SUBSTITUTION`) — **backstop only**; too late to stop modem dial when digits exist (see forensics) |
| Incoming | Early Rust session insert; Kotlin `disconnect` on disabled / `SecurityException`; caller does not hang Encrypted |
| UI | `PhoneAccountOnboardingActivity` + persistent MainActivity banner; re-check on resume |

### Modem / RIL forensics (API 35 `Saga_PA_Test`, forced SIM path)

Debug-only: account left disabled; `TEST_FORCE_IROH_SIM_PATH` bypasses the pre-dial enable gate so Telecom substitutes the SIM account. Script: `testing/e2e/run-modem-ssp-forensics.sh` (`SSP=...`).

| SSP | Digits after keypad+strip | Telecom handle | Modem / RIL (exact string) | InCall abort |
|-----|---------------------------|----------------|----------------------------|--------------|
| `iroh` (legacy, before) | `4764` | `tel:4764` (dumpsys `tel:**64`) | `RILJ > DIAL`; **`AT> ATD4764;`**; `+CLCC: …,4764,129` ACTIVE | `placeCallDeltaMs=39` (re-run 71); handle already `tel:4764` |
| `@` (rejected) | *(empty)* | `tel:%40` → Telecom `ErrorDialogActivity` — also breaks **enabled** Saga dials | No `ATD*` | No abort |
| `+` (rejected) | none (keeps `+`) | `tel:%2B` → SIM START_CONNECTION | **`AT> AT+CUSD=1,"+"`** (USSD) | abort ~75 ms |
| `,` (production, after) | none (keeps PAUSE `,`) | `tel:%2C` → SIM START_CONNECTION then LOCAL disconnect | **No `ATD*`, no `CUSD`**; radio only `AT> AT+CLCC` (hangup poll). Telephony briefly DIALING→DISCONNECTED without `RILJ > DIAL` | `placeCallDeltaMs=68`; handle `tel:%2C` |

**Timing (`iroh` → modem):** placeCall `04:02:35.649` → abort `04:02:35.689` (+39 ms) → `RILJ > DIAL` / `AT> ATD4764;` at `04:02:35.799`–`.802` (~110 ms after placeCall, **~70 ms after abort**). The backstop disconnect does **not** prevent a carrier dial attempt when the SSP yields digits; the pre-dial enable check is the primary guard, and the SSP must leave no dialable digits if substitution ever occurs.

### Onboarding E2E (local one-off, not CI)

Harness: `testing/e2e/run-e2e-phone-account-onboarding.sh` (blocked dial, Settings-UI enable, incoming-while-disabled). Peer: `Saga_Bob` (`emulator-5556`).

| AVD | Serial | API | Result |
|-----|--------|-----|--------|
| `Saga_PA_Test` | `emulator-5558` | 35 | **PASS** — BLOCK / SETTINGS_UI enable / CASEB ring+connect / URI `tel:,` / inbound reject while disabled |
| `Saga_PA_API29` | `emulator-5554` | 29 | **PASS** — same checks (All calling accounts → Saga switch; `tel:,` placeCall) |

Logs: `/tmp/e2e-pa-api35.log`, `/tmp/e2e-pa-api29.log`. Screenshots: `/tmp/saga-phone-account-screens/onboarding.png`, `banner.png`, `bob-ringing-after-enable.png`.

### Full E2E Cases 1–6 with `tel:,` (API 35, local one-off)

AVDs: `Saga_Alice` / `Saga_Bob` (`emulator-5554` / `emulator-5556`). Log: `/tmp/e2e-full-api35-tel-comma.log`.

```
CASE1:PASS … CASE2:PASS … CASE2b:PASS … CASE2c:PASS …
CASE3:PASS … CASE4:PASS … CASE5:PASS … CASE6:PASS … PHASE3:PASS
```

Also fixed: `initialSnapshot` now uses Contacts-aware key lookup (avoids premature `call_unsecure` on phone lookup keys); InCallActivity defers Iroh KEY_NOT_FOUND connect cue until handshake settles; harness polls callee `setActive` and force-stops bob after Case 4/5 for clean mid-suite state.

---

## Harness note: “Cases 1–6 + Phase 3”

In `testing/e2e/run-e2e-full.sh`, **Phase 3 is not product Phase 3** (Nitrokey / long-term key hardware). It is a **static source grep** after the live cases:

- Greps `android/app/src/main/java` for `applyConnectSnapshot` / `maybePlayConnectCue` under `SagaInCallActivity` / `SagaInCallService`
- Records `PHASE3:PASS` or `PHASE3:FAIL` — no emulator, no audio playback

So “Cases 1–6 + Phase 3” means: live E2E cases 1–6 (and 2b/2c), then this audio-cue wiring regression check.

---

## 2026-11-12 — Phase 1: upstream CI (host + Android native) + E2E re-verify

### What changed

| Area | Change |
|------|--------|
| Gating CI | `.github/workflows/ci.yml` — Rust fmt/clippy/`-D warnings`/test for `mock-token` + `saga-iroh-core`; Android `assembleDebug` + `testDebugUnitTest` with `SAGA_REQUIRE_NATIVE=1`; CI APK must contain `libsaga_iroh_core.so` for **`x86_64` only** |
| Native profile | `[profile.ci]` (no LTO, `codegen-units=16`) for debug/CI; `[profile.release]` fat LTO for release/F-Droid (both ABIs) |
| E2E on CI | **Not gated.** Two concurrent AVDs are local-only; documented in `testing/e2e/README.md` (no notice-only workflow) |
| Dependabot | Weekly; cargo / gradle / github-actions; minor+patch grouped per ecosystem |
| Toolchain pins | `rust-toolchain.toml` → `1.94.0`; Gradle `ndkVersion = "27.2.12479018"` (r27c) |
| Native fail-hard | **Release always** requires native (cannot be relaxed). Debug fail-hard via CI / `SAGA_REQUIRE_NATIVE` / `-Psaga.requireNative`. Local debug without cargo-ndk may warn + stub only |
| E2E harness | `run-e2e-full.sh` launches app once before `set-phone-account-enabled` (fresh AVDs otherwise leave Saga PhoneAccount disabled). Real users cannot run that adb command — see PhoneAccount UX gap below |
| Spec | RFC 3711: sliding replay window ≠ call ceiling; master-key lifetime 2⁴⁸ packets. Brainpool+ECDSA + SRTP AES-256-GCM. §12 marked superseded |
| Icons | `icons/broken.svg` for §13 downgraded state |
| Gradle wrapper | Jar updated to official Gradle **8.10.2** wrapper SHA-256 `2db75c40782f5e8ba1fc278a5574bab070adccb2d21ca5a6e5ed840888448046` ([release-checksums](https://gradle.org/release-checksums/)). Earlier CI “wrapper validation” failures were from `gradle/actions/setup-gradle` (`validate-wrappers: true`); CI now uses `actions/cache` for Gradle caches |

### Verified (host)

```
cd testing/mock-token && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
# PASS

cd saga-iroh-core && cargo fmt --check && cargo clippy --all-targets --features iroh-transport,mock-token -- -D warnings && cargo test --features iroh-transport,mock-token
# PASS
```

### Verified (host Android build)

```
export ANDROID_NDK_HOME=$ANDROID_HOME/ndk/27.2.12479018
export SAGA_REQUIRE_NATIVE=1
# Local debug: both ABIs, cargo profile=ci
cd android && ./gradlew assembleDebug testDebugUnitTest verifyNativeLibsInApk --no-daemon -Psaga.requireNative=true
# BUILD SUCCESSFUL — OK: libsaga_iroh_core.so present for arm64-v8a and x86_64

# CI-shaped (x86_64 only):
CI=true ./gradlew assembleDebug --no-daemon -Psaga.requireNative=true
# OK: lib/x86_64/libsaga_iroh_core.so only
```

### PhoneAccount UX (fresh AVD, no `set-phone-account-enabled`)

AVD `Saga_PA_Test` (`emulator-5558`): install debug APK, grant dialer role, launch app, **do not** enable PhoneAccount via adb.

| Check | Result |
|-------|--------|
| Account state | Stays disabled (`[ ]` in `dumpsys telecom`) |
| Outgoing Iroh | Does **not** work — Telecom falls back to SIM telephony account; peer never rings |
| Incoming Iroh | Does **not** ring — `addNewIncomingCall` → `SecurityException: This PhoneAccountHandle is not enabled for this user!` |

Issue: [#2](https://github.com/Supermagnum/saga/issues/2) (onboarding via `TelecomManager.ACTION_CHANGE_PHONE_ACCOUNTS`; fix before Phase 2).

### Verified (emulator E2E — local relay)

AVDs: `Saga_Alice` / `Saga_Bob` (`sdk_gphone64_x86_64`, API 35) as `emulator-5554` / `emulator-5556`.
Relay: `iroh-relay --dev` → `http://10.0.2.2:3340`.

```
SERIAL_A=emulator-5554 SERIAL_B=emulator-5556 bash testing/e2e/run-e2e-full.sh
CASE1:PASS … CASE2:PASS … CASE2b:PASS … CASE2c:PASS …
CASE3:PASS … CASE4:PASS … CASE5:PASS … CASE6:PASS … PHASE3:PASS
```

First attempt on fresh AVDs failed RING (Saga PhoneAccount still `[ ]` disabled — enable ran before `Application` registered the account). After launch-then-enable (harness fix), full suite green. Screencaps under `/tmp/saga-case{2,2b,3,4,5,6}-*-ringing.png`.

### Not verified / needs hardware

- Real phones, Galdralag token, Nitrokey USB
- Production relay TLS (still `test-utils` / insecure path in debug)
- OEM Telecom/telephony handling of `tel:,` (PAUSE SSP): vendor stacks may reject it or behave differently than the emulator RIL; re-verify on a physical device that no `ATD` / USSD is emitted on a real modem if SIM substitution ever occurs

### Known gaps before Phase 2 (historical note from PR #15 era)

- ~~Phase 1.5: replace homemade mock-token wire~~ — done (see Phase 1.5 section above)
- No Opus / iroh-roq / SRTP media path yet
- Release still carries `test-utils` / `mock-token` / `TEST_*` (Phase 6 hygiene)

### CI run link

PR: https://github.com/Supermagnum/saga/pull/1  
Prior green run (dual ABI + release LTO): https://github.com/Supermagnum/saga/actions/runs/43137989958  
Wall ~12.7 min (dominated by cold Android native+Gradle; Rust job ~33s). Earlier failed setup runs: `43137246126`, `43137734182`.

Local CI-shaped timing after `[profile.ci]` + x86_64-only (`CI=true ./gradlew assembleDebug testDebugUnitTest --no-daemon`):

| Run | Wall clock |
|-----|------------|
| Cold (clean `target/.../ci` + app/build) | **72 s** |
| Warm (immediate re-run) | **7.4 s** |

---

## 2026-07-09 — addNewIncomingCall bridge (inbound Iroh ringing)

### Problem (confirmed)

Inbound Iroh connections were handled only in Rust (`handle_inbound_connection`). No `TelecomManager.addNewIncomingCall` bridge existed, so `SagaConnectionService.onCreateIncomingConnection` was never invoked and the callee never rang (observed live on alice during bob→alice).

Prior Case 2/2b PASS results validated **encryption/media logcat only** — not callee incoming-call UX.

### Implementation

| Layer | Change |
|-------|--------|
| Rust `iroh_transport.rs` | On inbound accept: map remote endpoint → dev lookup key, `bridge_notify_incoming_call(sessionId, lookupKey, remoteId)` **before** mock-token handshake; register session after handshake |
| Rust `lib.rs` | `notifyIncomingCall` JNI callback (JavaVM cached at `JNI_OnLoad`); `poll_handshake` returns **Pending** when session not yet registered |
| `IrohNativeBridge.kt` | `@JvmStatic notifyIncomingCall` → `IrohIncomingCallBridge` |
| `IrohIncomingCallBridge.kt` | Main-thread `TelecomManager.addNewIncomingCall` with verified `PhoneAccountHandle`, caller extras (`EXTRA_SESSION_ID`, `EXTRA_LOOKUP_KEY`, `EXTRA_CONTACT_NAME`) |
| `SagaConnectionService.kt` | Fixed incoming path: `startIncoming` (not `startOutgoing`); three CHECKPOINT logs (entered / STATE_RINGING / startIncoming done) |
| `IrohDialManager.kt` | New `startIncoming` + `onIncomingAnswered`; keeps Connection in RINGING until answer |
| `SagaHandshakeCoordinator.kt` | Poll-with-retry until session registered (inbound race) |
| E2E | `wait_for_incoming_ringing`, screencap, `TEST_ANSWER_INCOMING`, `run-case-standalone-inbound.sh` |

### Verification checkpoints (Case 2b, bob→alice, alice callee)

Required signals (all observed on standalone re-run):

```
[Saga Iroh Listen] inbound accepted remote=[...] lookup=[15550100010] session=[inbound-15550100010-...]
[Iroh Native Bridge] notifyIncomingCall session=[...] lookup=[15550100010]
[Saga Incoming Call Bridge] CHECKPOINT addNewIncomingCall returned (no throw)
[Saga ConnectionService] CHECKPOINT onCreateIncomingConnection entered
[Saga ConnectionService] CHECKPOINT Connection STATE_RINGING state=[2] caller=[bob]
[Saga InCallService] CHECKPOINT onCallAdded origin=[IROH] peer=[15550100010] telecomState=[2]
[Iroh Dial Manager] CHECKPOINT incoming Connection setActive key=[15550100010]
[Saga Media Round-Trip] responder decrypted_ok=true
```

Screencap (callee ringing UI): `/tmp/saga-inbound-screencaps/case2b-callee-ringing.png` (51468 bytes)

Case 2 (alice→bob, bob callee): `/tmp/saga-inbound-screencaps/case2-callee-ringing.png` (51468 bytes)

### Standalone re-run results (clean boot, local relay)

```
CASE2b:PASS inbound ring + answer + encrypted + media
CASE2:PASS inbound ring + answer + encrypted + media
CASE3:PASS first-trust ring + answer + encrypted + flag set
CASE4:PASS ring + answer + downgrade state + local log entry
CASE5:PASS ring + answer + mid-call re-handshake paths
CASE6:PASS ring + answer + encrypted with carrier props (WiFi kept for relay)
```

Screencap Case 3 (first-trust, bob callee): `/tmp/saga-inbound-screencaps/case3-bob-ringing.png` (51659 bytes)

### Cases 4–6 — ringing checkpoints (standalone, clean boot, 2026-07-10)

All three cases: **alice caller → bob callee** (`emulator-5554` → `emulator-5556`). Same checkpoint bar as Case 3: `addNewIncomingCall` → `onCreateIncomingConnection` → `STATE_RINGING` → `onCallAdded` → screencap → `TEST_ANSWER_INCOMING` (lookup `15550100011`) → case-specific assertions.

| Case | Ringing | Screencap | Original assertions (after ring + answer) |
|------|---------|-----------|-------------------------------------------|
| **4 — Downgrade** | PASS | `/tmp/saga-inbound-screencaps/case4-bob-ringing.png` (52857 bytes) | PASS — caller `settled to [Downgraded]` + `Saga Downgrade Event`; callee `Handshake settled to [Downgraded]`. Ringing precedes forced handshake failure (`notifyIncomingCall` in Rust before mock-token handshake). |
| **5 — Mid-call** | PASS | `/tmp/saga-inbound-screencaps/case5-bob-ringing.png` (52437 bytes) | PASS — encrypted baseline + `setActive` on callee; **5a** re-handshake success without mid-call tone; **5b** crypto failure → Case 4 downgrade log + distinct mid-call tone (ringing applies to initial setup only, not 5b). |
| **6 — Carrier props** | PASS | `/tmp/saga-inbound-screencaps/case6-bob-ringing.png` (52663 bytes) | PASS — `Playing call_secure exactly once`, caller `settled to [Encrypted]`, callee `setActive`. Carrier props (`CarrierA`/`CarrierB`) applied **before** dial; WiFi kept for Iroh relay (same as pre-change harness). |

```
CASE4:PASS ring + answer + downgrade state + local log entry
CASE5:PASS ring + answer + mid-call re-handshake paths
CASE6:PASS ring + answer + encrypted with carrier props (WiFi kept for relay)
```

**Case 4 assertion note:** `CHECKPOINT incoming Connection setActive` is **not** emitted on the downgrade path when handshake settles to `[Downgraded]` while the Connection is still `STATE_RINGING` — `publishHandshakeState` skips `setActive` for `Downgraded` (`IrohDialManager.kt`). Initial standalone script incorrectly required `setActive` on callee (copied from encrypted cases 3/5/6); corrected to callee `Handshake settled to [Downgraded]`, matching `run-e2e-full.sh` caller-side downgrade checks plus ringing proof.

**Harness change:** `testing/e2e/run-case-standalone.sh` — `ring_answer_bob()` helper; Cases 4–6.

### Full suite re-run (local relay, 2026-07-10)

Ringing checkpoints folded into `run-e2e-full.sh` for Cases 4–6 (Cases 2/2b/3 already had them). Full sequential run after Cases 1–3 and 2c — exercises relay wait / caller force-stop / callee listen carry-over between cases.

```
CASE1:PASS … CASE2:PASS … CASE2b:PASS … CASE2c:PASS …
CASE3:PASS first-trust ring + answer + encrypted + flag set
CASE4:PASS ring + answer + downgrade state + local log entry
CASE5:PASS ring + answer + mid-call re-handshake paths
CASE6:PASS ring + answer + encrypted with carrier props (WiFi kept for relay)
PHASE3:PASS
```

**Case 4 in full suite:** Downgraded-not-`setActive` distinction **survived** suite ordering (Cases 2→3 encrypted calls on bob before Case 4 downgrade). Callee assertion remains `Handshake settled to [Downgraded]` — not `setActive`. No interaction bug from prior-case bob state.

Full-suite screencaps: `/tmp/saga-case{2,2b,3,4,5,6}-*-ringing.png`

**Incoming-call bridge thread:** closed — six cases, real ringing proof on all, Case 4 diagnosed correctly (downgrade path skips `setActive`).

### Step 4 / Case 2c: does explicit unencrypted-to-keyed contact use addNewIncomingCall?

**No — not a gap.** Step 4 routes `force_unencrypted` through `DialTarget.Cellular` → `placeCellularCall()` (plain `tel:` without Saga `PhoneAccountHandle`). No Iroh session, no Rust inbound handler, no callee `addNewIncomingCall`.

Verified (`run-e2e-unencrypted-keyed.sh`):

```
PASS: explicit unencrypted keyed contact -> cellular + call_unsecure, no downgrade, no Iroh inbound on callee
```

Bob callee logcat had **no** `notifyIncomingCall` / `CHECKPOINT addNewIncomingCall` / `inbound accepted`. Case 2c (bob→thor) remains plain cellular for the same reason. There is no current path for "Saga-originated but intentionally unencrypted Iroh" — only encrypted Iroh or explicit cellular.

### Retrospective: prior PASS interpretation

| Case | Prior PASS tested | Callee ringing actually tested? |
|------|-------------------|--------------------------------|
| 2 (alice→bob) | Caller encrypted + bob media logcat | **No** — until this fix |
| 2b (bob→alice) | Caller encrypted + alice media logcat | **No** — alice never rang on screen |
| 3 (alice→bob, first trust) | Handshake + encryption flag on caller | **No** — until ringing added to standalone/full suite |
| 3 (after 2026-07-10) | Ring + answer + encrypted + flag | **Yes** — same checkpoint bar as 2/2b |
| 4–6 (before 2026-07-10) | Handshake/encryption flags on alice (caller) | **No** — incoming UX not asserted |
| 4–6 (after 2026-07-10) | Ring + answer + case-specific outcome | **Yes** — standalone Cases 4/5/6 pass with screencaps |
| 2c | Cellular only (thor has no key) | N/A — no Saga Iroh callee; Step 4 unencrypted also cellular |

---

## 2026-07-09 — Network diagnosis (Steps 1–3), unencrypted keyed dial (Step 4)

### Step 1 — Public DNS / hostname reachability (immediate post cold reboot)

Cold reboot: `adb -s emulator-5554 reboot && adb -s emulator-5556 reboot` (both `sys.boot_completed=1` within ~4s).

```
=== emulator-5554 ===
$ adb -s emulator-5554 shell getprop net.dns1
(empty)

$ adb -s emulator-5554 shell ping -c 3 wikipedia.org
PING wikipedia.org (185.15.59.224) ...
3 packets transmitted, 3 received, 0% packet loss
rtt min/avg/max/mdev = 32.376/33.328/35.196/1.337 ms

$ adb -s emulator-5554 shell ping -c 3 vg.no
PING vg.no (52.84.50.19) ...
3 packets transmitted, 3 received, 0% packet loss
rtt min/avg/max/mdev = 21.090/88.112/219.706/93.056 ms

=== emulator-5556 ===
$ adb -s emulator-5556 shell getprop net.dns1
(empty)

$ adb -s emulator-5556 shell ping -c 3 wikipedia.org
PING wikipedia.org (185.15.59.224) ...
3 packets transmitted, 3 received, 0% packet loss
rtt min/avg/max/mdev = 498.715/499.049/499.307/0.247 ms

$ adb -s emulator-5556 shell ping -c 3 vg.no
PING vg.no (52.84.50.101) ...
3 packets transmitted, 3 received, 0% packet loss
rtt min/avg/max/mdev = 498.490/499.001/499.428/0.903 ms
```

DHCP DNS on both emulators (from `dumpsys wifi`): **`10.0.2.3`**. Hostname resolution works on both instances immediately after cold boot.

**Duplicate MAC check:** `cat /sys/class/net/wlan0/address` returned **Permission denied** on this emulator image (even after `adb root`). Could not confirm or rule out duplicate-MAC via sysfs. Different DHCP addresses (`10.0.2.17` vs `10.0.2.16`) and both reaching the public internet argue against the duplicate-MAC failure mode, but sysfs MAC comparison was **not completed**.

### Step 2 — Cross-emulator L3 reachability

**Immediate post cold boot:**

```
ALICE_IP=10.0.2.17 BOB_IP=10.0.2.16

$ adb -s emulator-5554 shell ping -c 5 10.0.2.16
5 packets transmitted, 5 received, 0% packet loss
rtt min/avg/max/mdev = 0.783/1.029/1.610/0.297 ms

$ adb -s emulator-5556 shell ping -c 5 10.0.2.17
5 packets transmitted, 5 received, 0% packet loss
rtt min/avg/max/mdev = 0.744/1.003/1.249/0.167 ms
```

**+60 seconds after cold boot:** identical — 5/5 both directions, sub-ms RTT. No cold-start routing flakiness observed.

### Step 3 — Relay stall vs network mode (public N0, 8-cycle sweep each)

`SAGA_IROH_RELAY_MODE=public`, `diagnose-relay-timeout.sh` Phase A + Phase B on **bob** (`emulator-5556`).

| Condition | Phase A (suite-start wait) | Phase B (8 restarts) | Notes |
|-----------|---------------------------|----------------------|-------|
| **WiFi enabled** | FAIL 120s (`native=0`) | **7/8 PASS** (restart 7 FAIL 120s) | Baseline |
| **WiFi off, mobile data only** | PASS 0s | **7/8 PASS** (restart 2 FAIL 120s; restart 3 PASS at **96s**) | `svc wifi disable` + `svc data enable` |
| **WiFi off + LTE speed/latency profile** | PASS 0s | **7/8 PASS** (restart 6 FAIL 120s) | `emu network speed lte` + `delay lte` — **simulated throughput/latency only**, not real radio generation behavior |

**Verdict:** Stall rate (~1/8) is **similar across all three network modes**. WiFi-disabled runs did not show a clearly worse stall profile than WiFi-enabled; one mobile-data restart passed only after 96s. Intermittent **public N0 `endpoint.online()` stalls** remain the dominant pattern, not a cellular-path-specific regression. Local relay baseline (prior session): 8/8 PASS at ~3s on WiFi.

### Step 4 — Explicit unencrypted call to keyed contact

**Design choice:** User-initiated unencrypted dial routes through **`tel:` / cellular** (`DialTarget.Cellular`), not a deliberately-unencrypted Iroh session. Rationale: matches §13 cellular rule (always key-not-found + `call_unsecure.wav`), avoids handshake/downgrade state entirely.

**Implementation:**

- `DialTargetResolver` — `preferCellular` / `EXTRA_FORCE_UNENCRYPTED` skips pubkey lookup when set
- MainActivity — **Call unencrypted** button; E2E hook `TEST_CONTACT_CALL --ez force_unencrypted true`
- `CallSecurityStateResolver` unchanged — cellular origin → `KEY_NOT_FOUND`, `call_unsecure.wav`, no downgrade modal

**E2E smoke** (`testing/e2e/run-e2e-unencrypted-keyed.sh`):

```
PASS: explicit unencrypted keyed contact -> cellular + call_unsecure, no downgrade
```

Log markers: `Resolved contact [bob] to cellular (explicit unencrypted)`, `Placing cellular call`, `Playing call_unsecure exactly once`, **no** `Saga Downgrade Event`.

### Carry-forward: alice never showed incoming-call / ringing UI

| Hypothesis | Result |
|------------|--------|
| Network/DNS failure (Steps 1–2) | **Ruled out** — DNS, public ping, and cross-emulator ping clean at cold boot and +60s |
| Relay unreachable (Step 3) | **Not the primary explanation for ringing** — relay stalls are intermittent on public N0 but local relay E2E passes bob→alice in harness; ringing UI absent even when connection might arrive |
| Callee Telecom integration | **Still open — separate bug** |

**Code evidence:** ~~No `TelecomManager.addNewIncomingCall`~~ **Fixed 2026-07-09** — see inbound bridge section above. Root cause for alice-never-rings was missing Telecom bridge, not network (Steps 1–3 ruled out network/DNS).

Step 5 key-sync investigation: see [`docs/key-sync-investigation.md`](docs/key-sync-investigation.md).

---

## 2026-07-09 — Relay harness + local relay

### Problem

Cases 3–6 (and sometimes 2/2b) failed in full suite runs with `RELAY:FAIL` or handshake timeouts. Initial diagnosis pointed at relay “cold start,” but forensics showed:

| Finding | Evidence |
|---------|----------|
| Not logcat false-negative on timeout | At 120s failure: endpoint bound, **zero** `[Saga Iroh Listen] relay online` lines, low logcat volume — `endpoint.online()` genuinely did not complete |
| Not restart-count fatigue | 8-cycle public-relay sweep: restart 1 failed 120s; restarts 2–8 passed at 0s |
| Public N0 variance | `presets::N0` → `*.relay.n0.iroh.link`; intermittent registration under rapid automated restarts |
| Stale logcat pitfall (fixed earlier) | After force-stop, old relay lines could pass grep before fresh `online()` — fixed via logcat clear after stop + `pidof` checks |

### Fixes shipped

| Area | Change |
|------|--------|
| **Local relay** | `testing/iroh-relay/start-local-relay.sh` — `iroh-relay --dev` on port 3340; emulators use `http://10.0.2.2:3340` |
| **Native relay gate** | `nativePollRelayReady()` / `nativeSetRelayUrl()` — JNI exposes `endpoint.online()` completion; harness reads `files/relay_status.txt` via `TEST_RELAY_QUERY` |
| **Shared E2E** | `e2e-common.sh` — poll native relay (3s / 120s), handshake poll, `warm_encrypted_caller`, no logcat relay sync |
| **Case 6** | Removed unnecessary `restart bob` (carrier props only) |
| **Provision** | Both emulators force-stopped once at baseline (intentional) |
| **Standalone** | `run-case-standalone.sh` for Cases 3–6 falsification from clean boot |

### Local relay 8-cycle sweep (native poll)

```
Phase A (Case-2-position bob):  PASS at 0s
Restarts 1–8:                   PASS at ~3s each (native=1, logcat agrees)
```

Public N0 comparison (earlier forensic): 1/8 failed at 120s (genuine stall), 7/8 instant pass.

### E2E results (representative)

| Case | Standalone (clean boot) | Full suite (local relay) | Notes |
|------|-------------------------|--------------------------|-------|
| Probe alice→bob | — | PASS | Encrypted + media round-trip |
| 1 — Cellular thor | — | PASS | `call_unsecure` + CELLULAR |
| 2 — Alice→bob | — | PASS | Ring + secure cue + media + `enc_flag` |
| 2b — Bob→alice | — | PASS | Ring + reverse direction |
| 2c — Bob→thor | — | PASS | Cellular, no key |
| 3 — First-time trust | **PASS** | PASS | Ring + encryption flag after handshake |
| 4 — Downgrade | **PASS** (ring + downgrade) | **PASS** (ring + downgrade, not setActive) | `saga_test_true.xml` + pre-seeded encryption |
| 5 — Mid-call | **PASS** (ring + 5a/5b) | **PASS** (ring + 5a/5b) | 5a success, 5b crypto failure downgrade |
| 6 — Carrier props | **PASS** (ring + encrypted) | **PASS** (ring + encrypted) | WiFi kept on emulator for Iroh |

Full suite on **public** N0 without local relay: **7–10/11** depending on run (relay timeouts at suite start or Case 4).

### Stack used during testing

| Layer | Components |
|-------|------------|
| Host | Linux, OpenJDK 21, Gradle, Rust/cargo-ndk, `iroh-relay` 1.0.2 (`--dev`) |
| Emulators | `emulator-5554`, `emulator-5556` (sdk_gphone16k_x86_64) |
| Transport | `saga-iroh-core` with `iroh-transport` + `mock-token`; ALPN `saga/voice/1` |
| Relay (E2E default) | Local HTTP relay `http://10.0.2.2:3340` |
| Relay (comparison) | Iroh N0 public relays |
| Contacts | `15550100010` / `15550100011` endpoint labels; dial by name `bob`/`alice`/`thor` |
| Harness | `run-e2e-full.sh`, `e2e-common.sh`, `run-case-standalone.sh`, `diagnose-relay-timeout.sh` |

### Native / app fixes (cumulative)

| Fix | File |
|-----|------|
| JNI jclass cache; Kotlin `object` uses `JObject` | `saga-iroh-core/src/lib.rs` |
| Connect retries; callee `ensure_listening` on start | `saga-iroh-core/src/iroh_transport.rs` |
| Custom relay URL + `pollRelayReady` | `saga-iroh-core/src/iroh_transport.rs`, `IrohNativeBridge.kt` |
| Call-id normalization for InCall cues | `SagaCallRegistry.kt` |
| Contact-based dial; `bobpeer12` removed | `DialTargetResolver`, E2E harness |

### Blockers before ship

1. Physical device validation (not emulator-only).
2. Production relay/TLS — local `--dev` HTTP relay is test-only; release must not use `insecure_skip_verify` or `test-utils` relay path.
3. Optional: remove `SAGA_RELAY_RETRY_ON_FAIL` path once local relay is mandatory in CI.

**Ready to ship:** No.

---

## Part A — Root cause: "enter a valid number"

### Investigation

| Location | Finding |
|----------|---------|
| `MainActivity` (before fix) | Only accepted `IrohNodeId.parse()` shape; phones/contacts rejected |
| Intent handling (before fix) | `ACTION_DIAL` / `ACTION_VIEW` not handled |
| `ContactKeyStore` (before fix) | No ContactsContract read path |

**Verdict:** Confirmed — validation blocked contact-resolved calls.

### Fix

`DialTargetResolver`, `ContactKeyRepository`, MIME `vnd.saga.galdralag_pubkey`, unified dial path.

---

## Part B — Contacts integration (Cases A1–A6)

Run: `./gradlew connectedDebugAndroidTest -Pandroid.testInstrumentationRunnerArguments.class=org.saga.contacts.ContactKeyIntegrationTest`

| Case | Result |
|------|--------|
| A1–A6 | **PASS** (each emulator) |

---

## Summary

| Suite | Passed | Failed |
|-------|--------|--------|
| Contacts A1–A6 | 6 | 0 |
| E2E Cases 1–6 (local relay, latest) | 6 | 0 |
| E2E full suite (public N0, variable) | 7–10 | 1–4 |
