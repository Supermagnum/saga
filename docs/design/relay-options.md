# Relay options for Saga (design note)

How Saga peers find each other and fall back when NAT blocks a direct path.
Debug/E2E defaults to a **local** `iroh-relay`; production dials use
**n0 public relays** via `Endpoint::builder(presets::N0)` (PkarrPublisher +
DnsAddressLookup). This note compares staying on n0 vs self-hosting.

## Decision — 2026-11-14

**Option A: n0 public relays (`presets::N0`) remain the default for release.**

Reasons:

1. **No infrastructure to run** — Saga does not need to operate a relay or
   pkarr/dns-server fleet for typical users to place Encrypted calls.
2. **Open-source relay software** — `iroh-relay` / related n0 stack is open
   source; dependence is on a public mesh of open software, not a proprietary
   binary in the APK.
3. **Reachability with Private DNS** — host peer↔peer public N0 soaks pass
   10/10 each direction with clean recursive DNS; emulator public path works
   10/10 when Android Private DNS (DoT) is used with the platform resolver
   path (see `TEST_RESULTS.md` / android-dns work). Captive ISP DNS without
   Private DNS remains a documented failure mode, not a reason to abandon n0.

### Risks retained next to this decision

| Risk | Detail |
|------|--------|
| **Telenor Nettvern** (and similar ISP “safe browsing” DNS) with Private DNS **off** | Captive answers for some `*.relay.n0.iroh.link` / `dns.iroh.link` → TLS `UnknownIssuer` on N0 HTTPS probes; `endpoint.online()` may never complete. Mitigation: OS Private DNS (DoT) / VPN / Orbot; **not** in-app pinned Cloudflare/Google resolvers. |
| **Metadata visible to relay and discovery operators** | n0 (or any home-relay operator) can observe EndpointId pairs, timing/volume, and client IPs when traffic uses the relay; `dns.iroh.link` sees publish/lookup. No cleartext audio when session keys stay on endpoints. See “Discovery metadata” below. |
| **Dependence on n0 service availability** | Capacity, policy, or regional outages can affect reachability when a direct path is unavailable. Local/custom relay remains the lab and privacy escape hatch. |

### Future work (not implemented)

An **advanced setting** for a user-supplied self-hosted relay URL (and later
optional custom discovery base) — productize the existing E2E
`saga_iroh_relay` override. Default remains n0; this setting is opt-in only.

## What a relay does (and does not)

| Layer | What the relay sees | What it cannot do |
|-------|---------------------|-------------------|
| QUIC / TLS to relay | That two EndpointIds are exchanging opaque datagrams; timing/volume | Read Saga voice ALPN payloads (encrypted hop) |
| Pkarr / DNS discovery | Public EndpointId → relay URL + optional direct addrs (published records) | Decrypt media or SessionToken material |
| Saga media (RoQ + SRTP / future SFrame) | Ciphertext length patterns | Decrypt audio if session keys stay on endpoints |

A relay is **not** a SIP PBX: it does not mix calls or terminate media in cleartext.
Carrier conference mixing is a different problem (§14.1).

## Discovery metadata: who can learn who calls whom

Call setup uses three distinct parties. None of them see cleartext audio when
session keys stay on the endpoints, but each can observe different **metadata**.

| Party | Role | What they can learn about “who calls whom” | What they typically cannot |
|-------|------|--------------------------------------------|----------------------------|
| **Relay operator** (n0 mesh or self-hosted `iroh-relay`) | Forwards QUIC datagrams when direct path fails | Both EndpointIds when they share a home relay or dial through the same relay; connection start/stop times; approximate byte volume / packet timing; client IP (or last-hop IP) of each endpoint | Phone numbers, contact display names, cleartext audio, SessionToken contents, which Android package dialed |
| **DNS / pkarr server** (`dns.iroh.link` or self-hosted `iroh-dns-server`) | Publishes and serves EndpointId → addressing records | Which EndpointIds **publish** (go online) and which **resolve** (look up a peer before dial); publish/lookup timestamps; querier IP for lookups; relay URL hints in the record | That a dial succeeded (lookup ≠ call); media contents; the callee’s local UI identity |
| **DNS resolver** (ISP recursive, Private DNS/DoT provider, VPN, Orbot) | Resolves hostnames used by discovery and relay TLS (`*.relay.n0.iroh.link`, `dns.iroh.link`, custom bases) | That the client resolved those hostnames (and when); with a **pinned** third-party resolver, every such lookup is visible to that provider | EndpointIds inside pkarr HTTP/DNS payloads only if it terminates TLS or sees cleartext DNS; Saga callee labels |

Implications:

- **Pinned Cloudflare/Google (or any) resolvers in-app are a privacy regression** for
  discovery: they override the user’s Private DNS / VPN / Orbot choice and send
  every relay and pkarr hostname lookup to that provider. Saga must use the
  platform / system resolver on Android unless the user explicitly configures a
  custom resolver (future setting; default off).
- A self-hosted **relay + dns-server** moves relay and pkarr metadata to an
  operator you choose; the user’s **DNS resolver** still sees hostname lookups
  unless they also route DNS through a trusted path (Private DNS, VPN, Orbot).
- EndpointId correlation across publish + relay sessions is enough for a
  sophisticated relay+dns operator to infer “A contacted B” with high
  confidence even without phone numbers.

## Option A — n0 public relays (`presets::N0`)

**What it is:** n0’s published relay mesh (`*.relay.n0.iroh.link`) plus pkarr over
`dns.iroh.link`. Release Android already builds with this preset when no
`saga_iroh_relay` override is set. **Chosen as the release default** (see
Decision above).

| Dimension | Assessment |
|-----------|------------|
| Reliability | Generally good on clean DNS; host soak with unhijacked DNS: home relay ~3s, voice connect ~70ms. **Telenor Nettvern** (ISP DNS filter / “safe browsing”) rewrites some `*.relay.n0.iroh.link` / `dns.iroh.link` answers → TLS `UnknownIssuer` on N0 HTTPS probes when the app uses the ISP recursive path (Private DNS off). With Android Private DNS (DoT) and Saga’s `AndroidDnsBridge`, probes resolve to real n0 hosts. Do **not** ship in-app pinned Cloudflare/Google resolvers as a workaround (privacy). Emulator NAT + public discovery remains flakier than local relay. |
| Privacy | See table above: n0 sees EndpointId pairs and envelopes; `dns.iroh.link` sees publish/lookup; user’s recursive DNS sees hostnames. Trust n0’s ops and jurisdiction. No Saga cleartext. |
| Cost / effort | Free for typical volumes; zero ops. Subject to n0 policy/capacity changes. |
| F-Droid | Usually fine (no proprietary binary). If n0 were ever considered a “non-free network service” anti-feature, document dependency clearly; self-host is the escape hatch. |
| User config | None today. Future (not implemented): advanced self-hosted relay URL + optional custom DNS resolver (default off). |

## Option B — Self-hosted `iroh-relay` + `iroh-dns-server`

**What it is:** Run your own relay (as in `testing/iroh-relay/`) and optionally an
iroh-dns / pkarr publisher so dial-by-public-key does not depend on `dns.iroh.link`.
**Not the release default**; remains the lab path and a future advanced setting.

| Dimension | Assessment |
|-----------|------------|
| Reliability | Full control; LAN/dev is already how E2E avoids public flakiness. WAN needs a stable public IP/DNS, TLS certs, and monitoring. |
| Privacy | Relay + pkarr metadata stay with the operator you choose (self, co-op, NGO). Still metadata-visible to that operator; user’s DNS resolver still sees hostnames unless DNS is also trusted. |
| Cost / effort | VPS (~few €/mo) + TLS + upgrades. dns-server adds another moving piece (pkarr records, rate limits). |
| F-Droid | Strong story: default stays n0; shipping a default that *requires* a proprietary cloud is avoided. |
| User config | Future settings field: relay URL (and later dns/pkarr base). App already pushes `saga_iroh_relay.xml` in E2E; productize that path later. |

## How a user could configure their own relay (future product sketch)

Not implemented. Sketch only:

1. Settings → Network → **Relay URL** (e.g. `https://relay.example.org`).
2. Optional: **Discovery** = n0 pkarr (default) | custom dns/pkarr base | discovery-off + manual EndpointAddr (power users).
3. Optional: **Custom DNS resolver** (default **off**) — only if the user opts in; never ship a forced third-party resolver.
4. On save: clear endpoint, re-bind with `RelayMode::Custom` + appropriate TLS (public CA or pinned).
5. Diagnostics: show home-relay URL, last `online()` age, last pkarr publish error (same signals as E2E `relay_status.txt`).

## Recommendation frame (for decision, not a commit)

- **Ship production dials on n0** for reachability out of the box; keep local/custom override for labs and privacy-conscious users.
- **Document DNS requirements** (real resolution of `*.relay.n0.iroh.link` / `dns.iroh.link`) — captive ISP resolvers (e.g. Telenor Nettvern) break public mode with `UnknownIssuer`; prefer OS Private DNS over in-app pins.
- **Invest in self-host docs + settings** before treating self-host as the default; ops cost is the main barrier, not code.
- Emulator public-relay soaks are a weak production proxy; prefer host↔host and real devices for release gates.

## Future options (IP / metadata protection)

Watch for a **Rust crate** that offers low-latency IP/metadata protection suitable
for live voice. Saga should not invent a custom tunnel while nothing mature exists;
the interim mitigation is a system-wide privacy-focused VPN (see the README
[Privacy](../../README.md#privacy-what-saga-does-and-doesnt-protect) section).

- **`nym-smoldvpn`** (pure-Rust, userspace WireGuard over Nym, 1–2 hops) is a
  candidate for a **later spike**: it can hide the client IP behind Nym hops but
  gives no timing protection, needs paid zk-nym credentials, and iroh's UDP would
  have to be routed through it. Not scheduled.
- **Nym mixnet mode** is unsuitable for live audio (only for small messages).

## Evidence pointers

- Host public soak harness: `testing/e2e/run-public-relay-peer-soak.sh` (optional host-side `bwrap` + clean `resolv.conf` for the **peer binary only**, not the Android app).
- Emulator public soak: `testing/e2e/run-public-relay-emu-peer-soak.sh` and
  `testing/e2e/run-public-relay-peer-emu-soak.sh` (enable Android Private DNS).
- Android iroh DNS: `AndroidDnsBridge` → `android.net.DnsResolver` (Private DNS /
  VPN / Orbot). No in-app pinned third-party resolvers.
- Android bind path: `saga-iroh-core` `Endpoint::builder(presets::N0)` unless `relay_url` override; `installAndroidContext` publishes `ndk_context` so iroh can read system DNS.
- Local insecure TLS is **mock-token / debug only** (`CaTlsConfig::insecure_skip_verify`); release must not ship that path.
