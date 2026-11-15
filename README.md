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
| F-Droid prep (not submitted) | [`docs/fdroid/`](docs/fdroid/) |
| Store listing metadata | [`fastlane/metadata/`](fastlane/metadata/) |

## Status

Implementation runs on Android emulators with contact-based dialing and encrypted Iroh calls. E2E Cases 1–10 (+ conference / onboarding) pass with a **local dev relay** on API 35; API 29 is exercised each part. Push/PR CI covers host Rust + Android debug with packaged `libsaga_iroh_core.so`; two-emulator E2E stays local-only (see [testing/e2e/README.md](testing/e2e/README.md)). F-Droid packaging is prepared under `docs/fdroid/` — **not** submitted yet. `applicationId` remains `org.saga` until a final ID is chosen.

See [TEST_RESULTS.md](TEST_RESULTS.md) for the latest run matrix and blockers.

### Screenshots (emulator)

| | |
|--|--|
| Encrypted call | ![Encrypted](fastlane/metadata/android/en-US/images/phoneScreenshots/01-encrypted-call.png) |
| Group call (debug builds only; not in release) | ![Group](fastlane/metadata/android/en-US/images/phoneScreenshots/07-group-call.png) |
| Voice disguise | ![Disguise](fastlane/metadata/android/en-US/images/phoneScreenshots/05-voice-disguise-chip.png) |

## Recommended OS

[GrapheneOS](https://grapheneos.org/) is the preferred Android OS for running Saga.
It provides a hardened security model relative to typical Android builds, and it
includes a duress PIN/password feature. Saga does not require GrapheneOS, but it
is the recommended environment when choosing a device OS.

## Privacy: what Saga does and doesn't protect

Saga encrypts **call content** end to end on Encrypted Iroh calls (SRTP after a
token-authenticated handshake). It does **not** hide who you talk to, your IP
address, or call timing/volume. Do not treat Saga as a way to stay untraceable
on the network.

Opus audio uses **constant bitrate (CBR, 32 kbps)** with DTX off, so encrypted
packet sizes do not follow speech content. That removes one traffic-analysis
leak; it does not hide that a call is happening.

### What is exposed today

Iroh prefers a **direct** peer path when NAT allows it, and falls back to a
**relay** when it does not. Discovery uses n0 pkarr/DNS (`dns.iroh.link`) unless
you override the relay for labs. DNS on Android goes through the platform
`DnsResolver` (Private DNS, VPN, and Orbot are honoured; Saga does not pin a
third-party resolver).

| Who | What they can see | What they cannot see |
|---|---|---|
| **The person you call** | Your public IP when the call connects **directly** (preferred when possible) | Call content on Encrypted calls |
| **Relay operator (n0, when a call is relayed)** | Both parties' IP addresses, EndpointIds, call timing, duration, and data volume | Call content (end-to-end encrypted) |
| **Discovery service (n0 DNS/pkarr)** | Lookups/publishes for a Saga EndpointId (that someone is trying to reach or announce a given identity), and from which IP | Call content; whether a dial actually connected |
| **Your DNS resolver** | Which discovery/relay hostnames are looked up (Private DNS hides this from your ISP, not from the resolver operator) | Call content |
| **Your ISP / network operator** | That you connect to relays or to a peer's IP, plus timing and volume | Call content |
| **Anyone correlating calls** | Saga EndpointIds are long-lived, so lookups and calls can be linked over time | — |
| **Carrier (normal phone calls)** | Everything on the phone-network path: those calls are never encrypted by Saga | — |

### Hardware token and forward secrecy

With a **Nitrokey**, Saga has per-call forward secrecy: a fresh ephemeral key is
generated on the phone for each call and discarded afterwards; the long-term key
stays on the card and only signs the ephemeral offer. The ephemeral private key
exists briefly in **phone memory** during the handshake — that is a weaker window
than on-token ECDH.

**Galdralag** (and a possible future Nitrokey on **Baochip-1x**, the same chip
family as Galdralag's Dabao target) does the ephemeral key exchange inside the
token, so the ephemeral key never leaves the hardware. Track Baochip-1x Nitrokey
support only when such a device exists; no work until then.

### Recommended mitigation: a privacy-focused VPN

Until a suitable low-latency Rust crate for IP/metadata protection exists, use a
**system-wide VPN** from a privacy-focused provider (**not yet verified with a
VPN**). Turn on Android **Always-on VPN** and **Block connections without VPN**
so Saga never dials outside the tunnel if the VPN drops. Saga respects the
phone's VPN and Private DNS settings.

**What a VPN changes:** the person you call, the relay operator, and your ISP see
the VPN exit address instead of yours.

**What it does not change:** the VPN provider can see your real IP and which
relays/peers you connect to; timing and volume metadata remain. Choose a
provider you trust. Tor is not suitable (latency too high for live voice; no
UDP/QUIC path for this stack).

#### Suitable providers (run by privacy advocates)

| Provider | Based in | Sign-up | Anonymous payment | No-logs verification | Notes |
|---|---|---|---|---|---|
| **Mullvad VPN** | Sweden | Random account number, no email | Cash by post, Monero, Bitcoin | Public audits (Cure53, KPMG) | WireGuard; open-source apps |
| **IVPN** | Gibraltar | Random account ID, no email | Cash, Monero, Bitcoin | Cure53 audit | WireGuard; open-source apps |
| **Proton VPN** | Switzerland | Email required (a disposable one works) | Bitcoin (no cash/Monero) | Annual Securitum audits | Open-source apps; free tier available |
| **RiseupVPN** | Riseup collective (since 1999) | No account | Free, donation-funded | No external audit | Bitmask client (OpenVPN); on F-Droid |
| **CalyxVPN** | Calyx Institute (non-profit) | No account | Free | No external audit | Bitmask client (OpenVPN); client and server open source |
| **NymVPN** | Decentralised network | See provider | See provider | — | Use only the fast **two-hop** mode for calls; mixnet mode adds too much delay for live voice |

This table is a starting point, not an endorsement. Re-check each provider's
current policies and audits before relying on them; review the list about once a
year.

**Guidance:**

- Prefer **WireGuard** (UDP) for calls; it usually adds less delay. OpenVPN-based
  services (Riseup, Calyx) work but may add latency, and free capacity can be
  slower when busy.
- Pay without linking a bank card where possible (cash, Monero): card payment
  ties your identity to the account.
- Use Always-on VPN + block connections without VPN.
- Also recommended: GrapheneOS, and Private DNS on (see [Troubleshooting](#troubleshooting)).
- See [docs/design/relay-options.md](docs/design/relay-options.md) for relay/
  discovery metadata detail and a short note on future Rust IP-protection options.

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
