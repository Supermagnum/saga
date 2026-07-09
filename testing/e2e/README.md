# Emulator E2E — Iroh dial path (section 15.4)

No SIP registrar or Flexisip. Requires one or two running AVDs, `adb`, and (by default) a **local Iroh relay** on the host.

## Prerequisites

- Android SDK / `adb` on PATH
- Two emulators (default `emulator-5554` = alice, `emulator-5556` = bob)
- Rust / `cargo-ndk` for `libsaga_iroh_core.so` (see [`saga-iroh-core/README.md`](../../saga-iroh-core/README.md))
- **Local relay:** `iroh-relay` 1.0 with `server` feature (installed automatically by `start-local-relay.sh` if missing)

## What we use during testing (and why)

### Devices and apps

| Item | Value | Why |
|------|-------|-----|
| Emulators | `emulator-5554` (alice), `emulator-5556` (bob) | Two-party encrypted + cellular scenarios without physical hardware |
| APK | `android/app/build/outputs/apk/debug/app-debug.apk` | Debug build with Iroh transport + test hooks |
| APK marker | `contact-keys=phone-labels-v3` | Confirms contact-key schema on device |
| Default dialer role | `org.saga` on both emulators | Required for `TelecomManager` / InCall path |

### Contacts and identities

Tests dial **contacts by display name** (`bob`, `alice`, `thor`), not raw Iroh peer labels.

| Contact | Phone (E.164) | Saga pubkey endpoint label | Role |
|---------|---------------|----------------------------|------|
| bob | +15550100010 | `15550100010` | Encrypted callee on SERIAL_B |
| alice | +15550100011 | `15550100011` | Encrypted callee on SERIAL_A (Case 2b) |
| thor | +15550100012 | (none) | Cellular-only; cannot secure |

Dev identity prefs (`saga_dev_identity.xml`) use the same phone-digit labels (`15550100010` for bob, `15550100011` for alice). Legacy labels such as `bobpeer12` are **not** used in the dial path.

Seeding: `seed-test-contacts.sh` (in-app `TestContactSeeder`) + `verify-contact-keys.sh`.

### Relay infrastructure

| Mode | Configuration | When to use |
|------|---------------|-------------|
| **local** (default) | `testing/iroh-relay/start-local-relay.sh` → `iroh-relay --dev` on `[::]:3340`; emulators use `http://10.0.2.2:3340` via `saga_iroh_relay_local.xml` | Normal E2E and CI — stable ~3s `endpoint.online()`, no load on public N0 relays |
| **public** | `SAGA_IROH_RELAY_MODE=public` — Iroh `presets::N0` (`*.relay.n0.iroh.link`) | Comparison / soak only; intermittent 120s registration stalls observed under rapid restarts |

Forensics on public relay showed **genuine** `endpoint.online()` delays (endpoint bound, relay log never emitted), not harness false negatives. Restart-count sweep: 1/8 cycles failed at 120s, 7/8 passed immediately — intermittent, not cumulative “fatigue.”

### Relay gating (harness)

The harness **does not** use logcat markers for relay synchronization.

1. Rust sets `RELAY_POLL` when `endpoint.online().await` completes (`nativePollRelayReady()`).
2. Harness sends `org.saga.TEST_RELAY_QUERY` → app writes `files/relay_status.txt`.
3. Harness reads via `adb shell run-as org.saga cat files/relay_status.txt`.

Poll interval: **3s**, ceiling: **120s** (`RELAY_POLL_MAX_SECS`). Handshake completion uses similar polling (`wait_for_handshake_log`), not fixed sleeps.

Optional stopgap: `SAGA_RELAY_RETRY_ON_FAIL=1` — one warm restart after native timeout (default **off** when using local relay).

Shared helpers: [`e2e-common.sh`](e2e-common.sh) (sourced by full suite, standalone, probe, diagnostics).

## Scripts

| Script | Purpose |
|--------|---------|
| [`run-e2e-full.sh`](run-e2e-full.sh) | Build, install, seed, Cases 1–6 + Phase 3 |
| [`run-case-standalone.sh`](run-case-standalone.sh) | Single case (3–6) from clean boot |
| [`run-alice-bob-probe.sh`](run-alice-bob-probe.sh) | Focused alice→bob encrypted probe |
| [`diagnose-relay-timeout.sh`](diagnose-relay-timeout.sh) | Case-2-position wait + 8-cycle restart sweep with native/logcat forensics |
| [`seed-test-contacts.sh`](seed-test-contacts.sh) | Seed bob/alice/thor on one emulator |
| [`verify-contact-keys.sh`](verify-contact-keys.sh) | Assert contact pubkey rows |

Relay host:

| Script | Purpose |
|--------|---------|
| [`../iroh-relay/start-local-relay.sh`](../iroh-relay/start-local-relay.sh) | Start local dev relay |
| [`../iroh-relay/stop-local-relay.sh`](../iroh-relay/stop-local-relay.sh) | Stop relay by pid file |

## Run

```bash
chmod +x testing/e2e/*.sh testing/iroh-relay/*.sh

# Local relay (default for run-e2e-full.sh)
bash testing/iroh-relay/start-local-relay.sh

SERIAL_A=emulator-5554 SERIAL_B=emulator-5556 bash testing/e2e/run-e2e-full.sh
```

Standalone verification (Cases 3–6):

```bash
bash testing/e2e/run-case-standalone.sh 4
```

Relay diagnostic (8-cycle sweep):

```bash
bash testing/e2e/diagnose-relay-timeout.sh
# Public N0 comparison:
SAGA_IROH_RELAY_MODE=public bash testing/e2e/diagnose-relay-timeout.sh
```

Environment variables:

| Variable | Default | Meaning |
|----------|---------|---------|
| `SAGA_IROH_RELAY_MODE` | `local` | `local` or `public` |
| `SAGA_IROH_RELAY_URL` | `http://10.0.2.2:3340` | Emulator-facing relay URL (local mode) |
| `RELAY_POLL_MAX_SECS` | `120` | Native relay wait ceiling |
| `RELAY_POLL_INTERVAL_SECS` | `3` | Poll interval |
| `SAGA_RELAY_RETRY_ON_FAIL` | `0` | One warm restart on relay timeout |

## What it tests

| Case | Call | Asserts |
|------|------|---------|
| 1 | alice → thor | cellular `call_unsecure` (no Saga key) |
| 2 | alice → bob | encrypted + media round-trip on bob |
| 2b | bob → alice | encrypted + media round-trip on alice |
| 2c | bob → thor | cellular only (no key) |
| 3 | alice → bob | first-time trust; encryption flag stored |
| 4 | alice → bob | downgrade with pre-seeded encryption + `force_fail` |
| 5 | alice → bob | mid-call re-handshake (success + crypto failure paths) |
| 6 | alice → bob | encrypted with carrier props (emulator keeps WiFi for relay) |

Calls use `org.saga.TEST_CONTACT_CALL --es contact_name <name>`.

**Harness rules:** provision may force-stop both emulators once (baseline). Mid-suite, only the **caller** is force-stopped per case; bob is not restarted in Case 6 (carrier props only).

## Test prefs (pushed via `adb`)

| File | Purpose |
|------|---------|
| `saga_dev_identity.xml` | `peer_label` for Iroh dev identity |
| `saga_iroh_relay.xml` | `relay_url` for local relay (E2E local mode) |
| `saga_test.xml` | `force_fail` — force handshake failure |
| `saga_encryption_established.xml` | Pre-seeded encryption history per contact phone key |

Prefs are read on app start; encrypted cases use `warm_encrypted_caller()` (force-stop caller, poll native relay, then dial).

## Key harness files

- [`e2e-common.sh`](e2e-common.sh) — relay poll, `contact_call`, `provision_clean_baseline`, handshake poll
- [`saga_dev_identity_bob.xml`](saga_dev_identity_bob.xml) / [`saga_dev_identity_alice.xml`](saga_dev_identity_alice.xml)
- [`saga_iroh_relay_local.xml`](saga_iroh_relay_local.xml) — `http://10.0.2.2:3340`
