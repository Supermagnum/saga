# Saga Test Results

**Last updated:** 2026-07-09

**Status:** Emulator E2E green on Cases 1–6 with **local relay** + **native relay polling**. Not ready to ship — needs release build, physical devices, and production relay/TLS policy (local dev relay uses HTTP + skip-verify for tests only).

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
| 2 — Alice→bob | — | PASS | Secure cue + media + `enc_flag` |
| 2b — Bob→alice | — | PASS | Reverse direction |
| 2c — Bob→thor | — | PASS | Cellular, no key |
| 3 — First-time trust | **PASS** | PASS | Encryption flag after handshake |
| 4 — Downgrade | **PASS** | PASS (public: intermittent alice relay) | `saga_test_true.xml` + pre-seeded encryption |
| 5 — Mid-call | **PASS** | PASS | 5a success, 5b crypto failure downgrade |
| 6 — Carrier props | **PASS** | PASS | WiFi kept on emulator for Iroh |

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
