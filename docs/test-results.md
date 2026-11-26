# Test results

Dated evidence that Saga works for the acceptance paths below. Not a fix log.

**Date:** 2026-11-26  
**Branch tip under test:** `fix/samsung-peer-part3` (Part 3 messaging on main + Samsung peer fixes)  
**Build:** debug APK with packaged `libsaga_iroh_core.so` (arm64-v8a + x86_64)

## Devices

| Role | Device | Serial | Identity label |
|------|--------|--------|----------------|
| Alice | Android emulator API 35 | `emulator-5554` | `15550100011` |
| Charlene | Samsung Galaxy Tab S6 Lite | `R52TB0JQEDE` (SM-P613) | `15550100030` |
| Thor | Seeded contact (cellular-only) | — | none (no Saga key) |

Local Iroh relay: `http://192.168.1.191:3340`.

**Thor** is a fixture contact (`+15550100012`) with **no** Galdralag / Saga
fingerprint row and **no** Iroh EndpointId. Dialing Thor always takes the
cellular path. Encryption cannot succeed: the in-call UI must show the §13
**key not found** icon ([`icons/key-error.svg`](../icons/key-error.svg) →
`ic_saga_key_error`) and status **Not encrypted**, play
`call_unsecure.wav` once, and must **not** show open/closed padlock states
(those are Iroh-only). Screenshot evidence:
[`21-cellular-not-encrypted.png`](../fastlane/metadata/android/en-US/images/phoneScreenshots/21-cellular-not-encrypted.png).

## Samsung ↔ emulator (acceptance)

| Path | Result | Evidence |
|------|--------|----------|
| Alice → Charlene encrypted text | **PASS** | Sender `status=[delivered]`; Charlene receives over `saga/msg/1` |
| Charlene → Alice encrypted text | **PASS** | Samsung `status=[delivered]`; Alice `notifyMsgDelivered peer=[15550100030]` |
| Alice calls Charlene; Charlene answers | **PASS** | Samsung `STATE_RINGING` + answer; handshake **Encrypted** |
| Charlene calls Alice; Alice answers | **PASS** | Alice RING checkpoints; answer; both sides **Encrypted** / `STATE_ACTIVE` |
| Dial Thor (no key) | **PASS** | Cellular only; **Not encrypted** + key-error icon; `call_unsecure` once; Cases 1 and 2c in API 35 suite |

Concrete proof artifacts (host): `/tmp/e2e-samsung-green5/results.log`, `/tmp/e2e-samsung-green6/results.log` and matching `logs/`.

## Emulator regression (Part 3 / PR #64 lineage)

| Suite | Result |
|-------|--------|
| API 35 Step 5: Cases 1–10 + 4b + C3–C5 + onboarding / `INBOUND_REJECT` | **PASS** (`/tmp/e2e-step5-part3.log`; Case 1 cellular Thor + Case 2c Bob→Thor) |
| RING flake N=20 Cases 6, 10 | **PASS** 20/20 |
| RING flake N=20 Case 4b | **19/20** (one cold miss) |
| Host `saga-msg` unit + `host_msg_roundtrip` | **PASS** |
| CI on merged messaging PR #64 | **PASS** (rust + android) |

## Contact key intake (Parts A–C)

| Path | Where | Result |
|------|-------|--------|
| QR / `saga://add-contact` confirm (Part A) | Emulator unit tests | **PASS** (`SagaAddContactLink` / attach probe) |
| Saga keys rename/move/remove (Part B) | Emulator unit tests | **PASS** (`ContactKeyDirectoryTest`) |
| NFC HCE APDU / NDEF (Part C) | Emulator unit tests | **PASS** (`Type4NdefApduHandlerTest`, `NdefUriMessageTest`) |
| NFC UI hidden without hardware | Tablet SM-P613 (no `android.hardware.nfc`) | Expected: Share/Receive NFC buttons hidden |
| Device-to-device NFC (HCE ↔ reader) | Hardware | **Not tested on hardware** (tablet has no NFC; phone-to-phone HCE not available in this setup) |
| Galdra sync import mapping (Part D) | Host Rust unit tests + fixture `testing/galdra-sync/` | **PASS** (`galdra_import` SEC1/OID mapping) |
| Galdra SAF importer UI end-to-end | Hardware / emulator UI | **Not tested on hardware** this pass (unit/schema fixture only) |

## Signed build

See [`compiled/`](../compiled/) for the verified debug APK, `SHA256SUMS`, and GnuPG signature.
