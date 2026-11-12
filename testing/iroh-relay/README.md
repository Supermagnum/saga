# Local Iroh relay for E2E / CI

Runs [`iroh-relay`](https://crates.io/crates/iroh-relay) in `--dev` mode for automated Android emulator tests.

## Why not public N0 relays?

E2E hammers the same two dev identities across many force-stops in a short suite. Against `presets::N0` (`*.relay.n0.iroh.link`):

- `endpoint.online()` sometimes never completed within 120s (genuine stall, not logcat detection gap).
- Failures were intermittent (one restart failed, the next passed immediately), not cumulative restart “fatigue.”
- Repeated automated CI runs should not depend on shared public relay infrastructure.

A local relay removes that variance and is the **default** for `run-e2e-full.sh`.

## Usage

```bash
# Start (installs iroh-relay 1.0.2 via cargo if missing)
# Binds 0.0.0.0:3340 and advertises http://<LAN-IP>:3340
bash testing/iroh-relay/start-local-relay.sh

# Print the advertise URL (also written to /tmp/saga-iroh-relay-url.txt)
bash testing/iroh-relay/lan-relay-url.sh
# or: cargo run --manifest-path tools/saga-peer/Cargo.toml -- relay-host

# Stop
bash testing/iroh-relay/stop-local-relay.sh
```

| Endpoint | URL |
|----------|-----|
| Host + emulator + saga-peer | `http://<LAN-IP>:3340` (shared; from `lan-relay-url.sh`) |

Do **not** put `127.0.0.1` or `10.0.2.2` in EndpointAddr advertisements — the peer
publishes its home relay URL into the address, and every side must resolve it.

E2E generates `shared_prefs/saga_iroh_relay.xml` with the LAN URL via
`push_iroh_relay_pref` (see `testing/e2e/e2e-common.sh`).

Logs: `/tmp/saga-iroh-relay.log`  
PID file: `/tmp/saga-iroh-relay.pid`  
Advertise URL: `/tmp/saga-iroh-relay-url.txt`

## Public relay comparison

```bash
SAGA_IROH_RELAY_MODE=public bash testing/e2e/diagnose-relay-timeout.sh
```

## Production note

`--dev` serves plain HTTP and is for tests only. Production builds must use proper TLS and public or dedicated relays — see Iroh deployment docs. The Android app only applies `insecure_skip_verify` when a custom test relay URL is configured.
