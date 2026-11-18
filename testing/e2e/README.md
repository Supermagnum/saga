# Emulator E2E — Iroh dial path (section 15.4)

No SIP registrar or Flexisip. Requires one or two running AVDs, `adb`, and (by default) a **local Iroh relay** on the host.

**CI note:** the two-emulator suite is **local-only** — not a push/PR gate.
GitHub-hosted runners cannot reliably run two concurrent AVDs (RAM/KVM), so
Cases 1–6 are not claimed green by Actions. Gating CI is
[`.github/workflows/ci.yml`](../../.github/workflows/ci.yml) (host Rust +
`assembleDebug` with packaged `libsaga_iroh_core.so`).

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
| thor | +15550100012 | (none) | Cellular-only; cannot secure; never an Iroh identity |
| laptop | +15550100020 | `15550100020` | Linux saga-peer (encrypted) |
| charlene | +15550100030 | `15550100030` | Third call peer (emulator-5558, encrypted) |

Dev identity prefs (`saga_dev_identity.xml`) use the same phone-digit labels (`15550100010` for bob, `15550100011` for alice). Legacy labels such as `bobpeer12` are **not** used in the dial path.

Seeding: `seed-test-contacts.sh` (in-app `TestContactSeeder`) + `verify-contact-keys.sh`.

### Relay infrastructure

| Mode | Configuration | When to use |
|------|---------------|-------------|
| **local** (default) | `testing/iroh-relay/start-local-relay.sh` → bind `0.0.0.0:3340`, advertise `http://<LAN-IP>:3340` for host + emulators + saga-peer (via `lan-relay-url.sh` / `push_iroh_relay_pref`) | Normal E2E and CI — stable ~3s `endpoint.online()`, no load on public N0 relays |
| **public** | `SAGA_IROH_RELAY_MODE=public` — Iroh `presets::N0` (`*.relay.n0.iroh.link`) | Comparison / soak only; intermittent 120s registration stalls observed under rapid restarts |

Forensics on public relay showed **genuine** `endpoint.online()` delays (endpoint bound, relay log never emitted), not harness false negatives. Restart-count sweep: 1/8 cycles failed at 120s, 7/8 passed immediately — intermittent, not cumulative “fatigue.”

### Relay gating (harness)

The harness **does not** use logcat markers for relay synchronization.

1. Rust sets `RELAY_POLL` when `endpoint.online().await` completes (`nativePollRelayReady()`).
2. Harness sends `org.saga.TEST_RELAY_QUERY` → app writes `files/relay_status.txt`.
3. Harness reads via `adb shell run-as org.saga cat files/relay_status.txt`.

Poll interval: **3s**, ceiling: **120s** (`RELAY_POLL_MAX_SECS`). Handshake completion uses similar polling (`wait_for_handshake_log`), not fixed sleeps.

Default: `SAGA_RELAY_RETRY_ON_FAIL=1` — one warm restart after native timeout
(helps API 29 suite-start stalls, issue #49). Set `=0` to disable.

Shared helpers: [`e2e-common.sh`](e2e-common.sh) (sourced by full suite, standalone, probe, diagnostics).

## Scripts

| Script | Purpose |
|--------|---------|
| [`run-e2e-full.sh`](run-e2e-full.sh) | Build, install, seed, Cases 1–7 + Phase 3 (Case 7 = speech PCM metrics) |
| [`run-case7-speech.sh`](run-case7-speech.sh) | Speech round-trip thresholds (corr/SI-SNR/STOI) |
| [`run-public-relay-emu-peer-soak.sh`](run-public-relay-emu-peer-soak.sh) | Emulator→peer public N0 ×N (enables Private DNS) |
| [`run-public-relay-peer-emu-soak.sh`](run-public-relay-peer-emu-soak.sh) | Peer→emulator public N0 ×N |
| [`run-network-change-recovery.sh`](run-network-change-recovery.sh) | Airplane / WiFi toggle → relay-ready + call (×N; ≤10s target) |
| [`run-e2e-phone-account-onboarding.sh`](run-e2e-phone-account-onboarding.sh) | Issue #2: disabled PhoneAccount blocks Iroh (no SIM), Settings UI enable, inbound reject |
| [`run-case-standalone.sh`](run-case-standalone.sh) | Single case (3–6) from clean boot |
| [`run-e2e-conference.sh`](run-e2e-conference.sh) | Phase 4 C1–C5 multi-call / conference (emulator limits honest) |
| [`run-e2e-group.sh`](run-e2e-group.sh) | Multi-peer group calls G1-G11 (3 emulators + saga-peer; **local only**) |
| [`run-alice-bob-probe.sh`](run-alice-bob-probe.sh) | Focused alice→bob encrypted probe |
| [`diagnose-relay-timeout.sh`](diagnose-relay-timeout.sh) | Case-2-position wait + 8-cycle restart sweep with native/logcat forensics |
| [`seed-test-contacts.sh`](seed-test-contacts.sh) | Seed bob/alice/thor on one emulator |
| [`verify-contact-keys.sh`](verify-contact-keys.sh) | Assert contact pubkey rows |

Relay host:

| Script | Purpose |
|--------|---------|
| [`../iroh-relay/start-local-relay.sh`](../iroh-relay/start-local-relay.sh) | Start local dev relay (0.0.0.0 + LAN advertise URL) |
| [`../iroh-relay/lan-relay-url.sh`](../iroh-relay/lan-relay-url.sh) | Print `http://<LAN-IP>:3340` |
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
| `SAGA_IROH_RELAY_URL` | `http://<LAN-IP>:3340` (from `lan-relay-url.sh`) | Shared relay URL for emulators + saga-peer (local mode) |
| `SAGA_IROH_RELAY_LAN_IP` | (auto) | Override LAN IP used by `lan-relay-url.sh` / `saga-peer relay-host` |
| `RELAY_POLL_MAX_SECS` | `120` | Native relay wait ceiling |
| `RELAY_POLL_INTERVAL_SECS` | `3` | Poll interval |
| `SAGA_RELAY_RETRY_ON_FAIL` | `1` | One warm restart on relay timeout (issue #49); set `0` to disable |

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

## Multi-peer group calls (G1-G11, local only)

Needs **three** emulators plus the host `saga-peer`; never a CI gate (same RAM/KVM
limit as the two-emulator suite, one more AVD).

| Role | Serial | Contact label | Identity prefs |
|------|--------|---------------|----------------|
| alice | `emulator-5554` (`SERIAL_A`) | `15550100011` | `saga_dev_identity_alice.xml` |
| bob | `emulator-5556` (`SERIAL_B`) | `15550100010` | `saga_dev_identity_bob.xml` |
| charlene | `emulator-5558` (`SERIAL_C`) | `15550100030` | `saga_dev_identity_charlene.xml` |
| laptop (4th) | host `saga-peer listen` | `15550100020` | built-in |

Charlene's contact (+15550100030, key `e4f3e33e...`) is seeded by `seed-test-contacts.sh` /
`TestContactSeeder` on all three emulators. Start the AVDs with matching ports
(`emulator -avd <name> -port 5558`) and install the **current** debug APK on each (the
suite probes `TEST_ADD_PARTICIPANT` first and exits 2 if the APK is older).

```bash
bash testing/e2e/run-e2e-group.sh                      # G1-G11
G_CASES="G1 G2 G3" bash testing/e2e/run-e2e-group.sh   # subset
SAGA_IROH_RELAY_MODE=public bash testing/e2e/run-e2e-group.sh   # public N0 relays (Private DNS)
SAGA_RUN_GROUP=1 bash testing/e2e/run-e2e-step5.sh     # optional, appended to Step 5 (default off)
```

Output: `/tmp/saga-group-e2e/` (logcat per case, `*.score.txt`, host-mesh log, UI dumps),
screenshots in `/tmp/saga-group-e2e/screencaps/` (`OUT_DIR`, `SCREENCAP_DIR` override).

Debug intents used (MainActivity, `FLAG_DEBUGGABLE` only):

| Intent | Extras | Use |
|--------|--------|-----|
| `org.saga.TEST_CONTACT_CALL` | `contact_name` | 1:1 baseline dial |
| `org.saga.TEST_ANSWER_INCOMING` | `lookup_key` | answer a ringing leg |
| `org.saga.TEST_ADD_PARTICIPANT` | `contact_name`, `confirm_unencrypted`, `introduced_by`, `roster_fingerprint`, `conference` | add to the live call; outcome logged as `E2E add participant name=[..] outcome=[..]` |
| `org.saga.TEST_GROUP_HANGUP_PARTICIPANT` | `contact_name` or `lookup_key` | hang up one participant |
| `org.saga.TEST_MEDIA_MUTE` (broadcast) | `--ez muted` | mute all engines |

Manual equivalents (from a running Iroh call on the adder):

```bash
adb -s emulator-5554 shell am start -n org.saga/.ui.MainActivity \
  -a org.saga.TEST_ADD_PARTICIPANT --es contact_name charlene
adb -s emulator-5556 shell am start -n org.saga/.ui.MainActivity \
  -a org.saga.TEST_ADD_PARTICIPANT --es contact_name 15550100030 \
  --ez confirm_unencrypted true --es introduced_by alice --es roster_fingerprint <64hex>
```

Shared helpers added to `e2e-common.sh`: `push_identity` (works for `charlene`),
`provision_clean_baseline_group`, `push_group_peer_prefs`, `add_participant`,
`add_participant_outcome`, `hangup_group_participant`, `settled_lookups`/`settled_count`,
`delete_contact_by_name`, `set_contact_key`, `tap_ui_id`.

The debug seeder re-inserts contacts on every app launch, so G4/G5 edit contacts
(`delete_contact_by_name`, `set_contact_key`) only after the call is established and never
restart that device afterwards.

Result tags: `PASS`, `FAIL` (exit 1), `NEEDS_HARDWARE` (emulator cannot exercise it) and
`PENDING_MESH` (needs the native mesh backend for Bob-Charlene links, or a mesh-capable
`saga-peer`; the pairwise-session scaffold only provides the adder's links).

| Case | What it checks | Expected on pairwise scaffold |
|------|----------------|-------------------------------|
| G1 | alice -> bob 1:1 Encrypted, no group markers | PASS |
| G2 | alice adds charlene; alice has 2 Encrypted links; screenshots; PCM metrics | PASS (3 of 3 links with mesh-join B↔C) |
| G3 | bob <-> charlene direct link, distinct media sessions | PASS (roster-driven mesh-join) |
| G4 | charlene deleted from bob -> "introduced by alice"; wrong roster fingerprint -> KEY_MISMATCH | PASS (Part 2a) |
| G5 | wrong key -> not Encrypted on alice; key-less peer needs confirm to admit | PASS (Part 2a) |
| G6 | add laptop; alice has 3 Encrypted links; 6-link mesh and hear-three | Alice 3 Encrypted PASS; 6-link hear-three PENDING_MESH (saga-peer 1:1) |
| G7 | 5th participant refused | PASS |
| G8 | bob leaves (alice + charlene continue); alice leaves (bob + charlene continue) | PASS both (Part 2c remote-close + mesh leave) |
| G9 | mute and hold reach every link | PASS mute + hold/resume (Part 2c setHeldAll) |
| G10 | incoming cellular during group: hold, no Merge | PASS or NEEDS_HARDWARE (emulator GSM) |
| G11 | charlene adds laptop | charlene→laptop Encrypted PASS; A/B auto-link PENDING_MESH |

Media metrics: `group_media_probe` injects the speech clip on alice, dumps playout PCM on
bob/charlene and scores corr / SI-SNR / STOI with `pcm_score.py` (recorded as `METRIC`
lines; INFO if no dump). The host mesh model (`cargo test -p saga-peer --test host_mesh`,
disable with `SAGA_GROUP_HOST_MESH=0`) is recorded as `HOSTMESH:` lines and corroborates 3 and
4 peer mixing, leave and 5th refusal without emulators; it does not replace PENDING_MESH.

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
- [`saga_iroh_relay_local.xml`](saga_iroh_relay_local.xml) — fallback stub; E2E generates LAN URL prefs at runtime
