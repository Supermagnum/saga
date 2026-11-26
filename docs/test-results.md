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

Local Iroh relay: `http://192.168.1.191:3340`.

## Samsung ↔ emulator (acceptance)

| Path | Result | Evidence |
|------|--------|----------|
| Alice → Charlene encrypted text | **PASS** | Sender `status=[delivered]`; Charlene receives over `saga/msg/1` |
| Charlene → Alice encrypted text | **PASS** | Samsung `status=[delivered]`; Alice `notifyMsgDelivered peer=[15550100030]` |
| Alice calls Charlene; Charlene answers | **PASS** | Samsung `STATE_RINGING` + answer; handshake **Encrypted** |
| Charlene calls Alice; Alice answers | **PASS** | Alice RING checkpoints; answer; both sides **Encrypted** / `STATE_ACTIVE` |

Concrete proof artifacts (host): `/tmp/e2e-samsung-green5/results.log`, `/tmp/e2e-samsung-green6/results.log` and matching `logs/`.

## Emulator regression (Part 3 / PR #64 lineage)

| Suite | Result |
|-------|--------|
| API 35 Step 5: Cases 1–10 + 4b + C3–C5 + onboarding / `INBOUND_REJECT` | **PASS** (`/tmp/e2e-step5-part3.log`) |
| RING flake N=20 Cases 6, 10 | **PASS** 20/20 |
| RING flake N=20 Case 4b | **19/20** (one cold miss) |
| Host `saga-msg` unit + `host_msg_roundtrip` | **PASS** |
| CI on merged messaging PR #64 | **PASS** (rust + android) |

## Signed build

See [`compiled/`](../compiled/) for the verified debug APK, `SHA256SUMS`, and GnuPG signature.
