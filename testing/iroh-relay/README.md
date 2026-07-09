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
bash testing/iroh-relay/start-local-relay.sh

# Stop
bash testing/iroh-relay/stop-local-relay.sh
```

| Endpoint | URL |
|----------|-----|
| Host | `http://127.0.0.1:3340` |
| Android emulator | `http://10.0.2.2:3340` |

The app reads the emulator URL from `shared_prefs/saga_iroh_relay.xml` (pushed by E2E as `saga_iroh_relay_local.xml`).

Logs: `/tmp/saga-iroh-relay.log`  
PID file: `/tmp/saga-iroh-relay.pid`

## Public relay comparison

```bash
SAGA_IROH_RELAY_MODE=public bash testing/e2e/diagnose-relay-timeout.sh
```

## Production note

`--dev` serves plain HTTP and is for tests only. Production builds must use proper TLS and public or dedicated relays — see Iroh deployment docs. The Android app only applies `insecure_skip_verify` when a custom test relay URL is configured.
