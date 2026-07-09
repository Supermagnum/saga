# Emulator E2E — Iroh dial path (section 15.4)

No SIP registrar or Flexisip. Requires one or two running AVDs and `adb`.

## Prerequisites

- Android SDK / `adb` on PATH
- Two emulators (default `emulator-5554` = alice, `emulator-5556` = bob)
- Optional: `cargo-ndk` for native `libsaga_iroh_core.so` (see `saga-iroh-core/README.md`)

## Contacts vs peer labels

Tests dial **contacts by display name** (`bob`, `alice`, `thor`), not raw Iroh labels.

| Contact | Phone | Saga key (peer label) | Role |
|---------|-------|----------------------|------|
| bob | +15550100010 | bobpeer12 | callee on SERIAL_B |
| alice | +15550100011 | alicepeer1 | caller on SERIAL_A |
| thor | +15550100012 | (none) | cellular-only, cannot secure |

`bobpeer12` and `alicepeer1` are **dev identity labels** pushed via `saga_dev_identity.xml` so each emulator's Iroh endpoint matches the contact key. They are not contact names.

## Run

```bash
chmod +x testing/e2e/*.sh
SERIAL_A=emulator-5554 SERIAL_B=emulator-5556 bash testing/e2e/run-e2e-full.sh
```

## What it tests

| Case | Call | Asserts |
|------|------|---------|
| 1 | alice -> thor | cellular `call_unsecure` (no Saga key) |
| 2 | alice -> bob | encrypted + media round-trip on bob |
| 2b | bob -> alice | encrypted + media round-trip on alice |
| 2c | bob -> thor | cellular only (no key) |
| 3-6 | alice -> bob | trust, downgrade, mid-call, WiFi-off variants |

Calls use `org.saga.TEST_CONTACT_CALL --es contact_name <name>`.

## Test prefs

- `saga_test.xml` — `force_fail` boolean
- `saga_encryption_established.xml` — per-contact phone key (e.g. `+15550100010` for bob)

Restart app (`force-stop`) between cases so SharedPreferences reload from disk.
