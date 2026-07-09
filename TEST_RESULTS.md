# Saga Test Results

**Last updated:** 2026-07-10 (full suite ringing checkpoints — thread closed)

**Status:** Emulator E2E green on Cases 1–6 with **local relay** + **native relay polling** + **inbound ringing bridge**. All six Iroh callee cases (2, 2b, 3, 4, 5, 6) verified with real ringing checkpoints in **standalone and full suite**. Not ready to ship — needs release build, physical devices, production relay/TLS policy.

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
