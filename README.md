# Saga

Encrypted voice calling for Android — default dialer + Iroh transport.

**Canonical spec:** [saga-spec.md](saga-spec.md)

**Call-state icons:** [icons/](icons/) (AGPL — see [icons/readme.md](icons/readme.md))

| Component | Path |
|-----------|------|
| Android app | [`android/`](android/) |
| Native Iroh core (JNI) | [`saga-iroh-core/`](saga-iroh-core/) |
| Emulator E2E harness | [`testing/e2e/`](testing/e2e/) |
| Local Iroh relay (E2E/CI) | [`testing/iroh-relay/`](testing/iroh-relay/) |
| Test results log | [TEST_RESULTS.md](TEST_RESULTS.md) |
| Gating CI | [`.github/workflows/ci.yml`](.github/workflows/ci.yml) |

## Status

Implementation runs on two Android emulators with contact-based dialing and encrypted Iroh calls. E2E Cases 1–6 pass with a **local dev relay** and **native relay-ready polling**; the suite is not production-ready until release hardening and on-device validation beyond emulators. Push/PR CI covers host Rust + Android debug with packaged `libsaga_iroh_core.so`; two-emulator E2E stays local-only (see [testing/e2e/README.md](testing/e2e/README.md)).

See [TEST_RESULTS.md](TEST_RESULTS.md) for the latest run matrix and blockers.

## Recommended OS

[GrapheneOS](https://grapheneos.org/) is the preferred Android OS for running Saga.
It provides a hardened security model relative to typical Android builds, and it
includes a duress PIN/password feature. Saga does not require GrapheneOS, but it
is the recommended environment when choosing a device OS.

## Testing approach (summary)

E2E uses **two AVDs** (`emulator-5554` = alice, `emulator-5556` = bob), `adb`, and the scripts under `testing/e2e/`. Calls are placed by **contact display name** (`bob`, `alice`, `thor`), not raw peer labels.

| What | Why |
|------|-----|
| **Local `iroh-relay --dev`** on host port 3340 (`http://10.0.2.2:3340` from emulators) | Public N0 relays (`*.relay.n0.iroh.link`) caused intermittent `endpoint.online()` stalls (up to 120s) under rapid automated restarts; local relay gives stable ~3s registration and avoids hammering shared public infrastructure during CI-style runs. |
| **Native `pollRelayReady()` (JNI)** | Relay gating reads `endpoint.online()` completion in Rust, not logcat markers. Logcat-as-IPC caused false pass/fail races (stale markers, clear-then-hope). |
| **Phone-digit contact keys** (`15550100010` / `15550100011`) | Replaces legacy `bobpeer12` labels; aligns contact keys, dev identity, and InCall registry keys. |
| **Standalone case runs** (`run-case-standalone.sh`) | Cases 3–6 verified individually from clean boot before relying on full-suite ordering. |

Quick start:

```bash
# Start local relay (once per host session)
bash testing/iroh-relay/start-local-relay.sh

# Full suite (builds APK, seeds contacts, Cases 1–7)
SERIAL_A=emulator-5554 SERIAL_B=emulator-5556 bash testing/e2e/run-e2e-full.sh
```

Use `SAGA_IROH_RELAY_MODE=public` to exercise public N0 relays for comparison (expect more variance).

Details: [testing/e2e/README.md](testing/e2e/README.md).

## Troubleshooting

### Public N0 relays fail with TLS `UnknownIssuer` or never go online

Some ISP “safe browsing” resolvers (e.g. Telenor Nettvern) hijack lookups for
`*.relay.n0.iroh.link` / `dns.iroh.link` and return a captive portal certificate.
Iroh then fails TLS probes (`UnknownIssuer`) and `endpoint.online()` never
completes.

Saga installs `ndk_context` (`IrohNativeBridge.installAndroidContext`) and
routes iroh DNS through `android.net.DnsResolver` (`AndroidDnsBridge`) so
A/AAAA/TXT honour **Private DNS (DoT), VPN, and Orbot**. It does **not** pin
Cloudflare/Google (or any) resolvers in-app: that would leak every discovery
hostname lookup and override the user’s DNS settings.

**What helps:**

| Approach | Effect |
|----------|--------|
| Android **Private DNS** (DoT), e.g. `one.one.one.one` | Platform `DnsResolver` uses DoT; bypasses ISP recursive hijacks (e.g. Telenor Nettvern → captive `UnknownIssuer`) |
| Clean recursive DNS / VPN / Orbot | Same path — Saga uses the active network’s platform resolver |
| Local / custom relay (`saga_iroh_relay`) | Avoids public N0 hostnames for the call path |
| Host `saga-peer` soak `bwrap` + clean `resolv.conf` | Applies only to the Linux peer binary, not the Android app |

If public N0 never becomes reachable and the call fails with **Not reachable**
within the connect budget (~18s), check Private DNS / VPN. Saga must not hang
silently on hijacked DNS.

See also [docs/design/relay-options.md](docs/design/relay-options.md) (discovery
metadata: what relay, pkarr/DNS, and the recursive resolver each observe).
