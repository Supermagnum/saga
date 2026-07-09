# Saga Test Results

**Repo:** `/mnt/2e9a1e9f-2097-408c-ab9a-a01b32f11d28/github-projects/saga`  
**Last updated:** 2026-07-09

**Status:** Not ready to ship. Contacts + contact-based Iroh dial path work on emulators. Encrypted E2E is partially green (probe + Case 1/2c/6); Cases 2-5 need a full uninterrupted suite run after harness regex fix.

---

## 2026-07-09 — E2E emulator session (5554=alice, 5556=bob)

### Contact / dial path (bobpeer12 removed)

| Item | Status |
|------|--------|
| Contact keys | Phone-digit endpoint labels (`15550100010` / `15550100011`); thor has no key |
| Dial harness | `TEST_CONTACT_CALL` by name (`bob`/`alice`/`thor`); raw `bobpeer12` removed |
| APK marker | `contact-keys=phone-labels-v3` on both emulators |
| `verify-contact-keys.sh` | PASS on both emulators |

### Native / transport fixes

| Fix | File |
|-----|------|
| JNI crash on `notifyFailed` — cache jclass via `find_class`, Kotlin `object` uses `JObject` not `JClass` | `saga-iroh-core/src/lib.rs` |
| Connect retries: 8 attempts, 45s each, 5s backoff; 60s initiator relay wait | `saga-iroh-core/src/iroh_transport.rs` |
| Callee listen on app start (`ensure_listening`) | `saga-iroh-core/src/iroh_transport.rs` |
| InCall call-id alignment (`15550100010` → `+15550100010`) so `call_secure` cue fires | `SagaCallRegistry.kt` |

### E2E harness

- `run-e2e-full.sh`: relay wait (non-blocking WARN), caller warm-start before encrypted dials, `enc_flag` uses `rg -F` (fixes `+` in E.164), Case 2c log pattern, Case 6 keeps WiFi for relay on emulator
- `seed-test-contacts.sh`: verify via `verify-contact-keys.sh`
- `run-alice-bob-probe.sh`: focused alice→bob probe

### E2E results (latest)

| Case | Result | Notes |
|------|--------|-------|
| Probe alice→bob | **PASS** | Encrypted handshake, media round-trip, `Playing call_secure exactly once` |
| 1 — Cellular thor | **PASS** | `call_unsecure` + CELLULAR origin |
| 2 — Alice→bob | **FAIL** (prior runs) | `enc_flag` regex bug on `+15550100010` — fixed, not re-run to completion |
| 2b — Bob→alice | **FAIL** (prior runs) | Same `enc_flag` / relay timing |
| 2c — Bob→thor | **PASS** | Cellular, no Saga key |
| 3–5 | **FAIL** (prior runs) | Depend on Case 2 encrypted call |
| 6 — Carrier props | **PASS** | Encrypted with emulator WiFi kept for relay |
| Run 5 (aborted) | **Incomplete** | Stopped during Case 2 dial (user request) |

### Blockers before ship

1. Complete full `run-e2e-full.sh` after `enc_flag` / `PHONE_*_RG` regex fix (Cases 2, 2b, 3, 4, 5).
2. Bob relay registration is intermittent on cold start (30–90s); harness uses WARN-not-fail + native connect retries.
3. pkarr / dns.iroh.link publish timeouts on emulator (non-fatal; relay path still works).

**Ready to ship:** No.

---

## Part A — Root cause: "enter a valid number"

### Investigation

| Location | Finding |
|----------|---------|
| `MainActivity.placeIrohCall()` (before fix) | Only accepted input passing `IrohNodeId.parse()` (8+ alphanumeric). Phone numbers and contact URIs were rejected with `invalid_peer_id` toast. |
| `MainActivity` intent handling (before fix) | `ACTION_DIAL` / `ACTION_VIEW` / `ACTION_CALL` were not handled. Contact-picker and `tel:` intents were ignored; dial field stayed empty. |
| `ContactKeyStore` (before fix) | Did not exist. No `ContactsContract` read path; keys could not be resolved at call time. |
| System Telecom layer | Standard Android "enter a valid number" can appear when `placeCall` receives an empty/invalid `tel:` URI. Saga never reached `placeCall` for contact-resolved targets because validation failed upstream. |

**Hypothesis:** dial validation assumed Iroh peer-id shape only, blocking contact-resolved calls.  
**Verdict:** **Confirmed** with code evidence. Secondary factor: missing `ACTION_DIAL` handling caused empty dial field, producing the system "valid number" message when placing from the contact picker.

**Not the cause:** `PhoneNumberUtils` running on a public key string (keys were rejected earlier by `IrohNodeId.parse`).

### Fix

- Added `DialTargetResolver` — accepts phone number OR resolved Saga contact key OR direct peer label.
- Added `ContactKeyRepository` + MIME type `vnd.android.cursor.item/vnd.saga.galdralag_pubkey`.
- `MainActivity` handles `ACTION_DIAL` / `ACTION_VIEW` / `ACTION_CALL`; unified Call button uses `DialTargetResolver`.
- Cellular calls use `TelecomManager.placeCall(tel:...)` without Saga phone account; Iroh calls use `saga:` URI + Saga `PhoneAccount`.
- **Manual peer entry:** 8+ alphanumeric peer labels accepted; 32-byte keys stored base64 in Contacts.

---

## Part B — Contacts integration (Cases A1-A6)

Run: `./gradlew connectedDebugAndroidTest -Pandroid.testInstrumentationRunnerArguments.class=org.saga.contacts.ContactKeyIntegrationTest`  
Devices: two Pixel 9 Pro AVDs

| Case | Assertion | Result |
|------|-----------|--------|
| **A1 — Write** | Insert custom MIME row; queryable from `ContactsContract.Data` | **PASS** |
| **A2 — Read-back integrity** | Decoded key bytes match written bytes exactly | **PASS** |
| **A3 — Call-time resolution** | Contact URI resolves to `DialTarget.Iroh`; key found at call time | **PASS** |
| **A4 — No key present** | Phone-only contact resolves to `DialTarget.Cellular`; no validation error | **PASS** |
| **A5 — Malformed row** | Corrupt base64 fails closed to null key | **PASS** |
| **A6 — Cellular regression** | Plain phone number resolves to `DialTarget.Cellular` | **PASS** |

**A1-A6 total:** 6 run, 6 passed, 0 failed (each emulator).

---

## Prior pass — Cases 1-6 (call behavior)

| Case | Result | Notes |
|------|--------|-------|
| 1 — Unencrypted | **FAIL** | E2E script aborted; intent delivery issue on `singleTop` |
| 2-6 | **FAIL/SKIP** | Not completed in prior run |

---

## Summary

| Suite | Passed | Failed |
|-------|--------|--------|
| Contacts A1-A6 | 6 | 0 |
| Call behavior 1-6 | 0 | 5+ |

**Ready to ship:** No.
