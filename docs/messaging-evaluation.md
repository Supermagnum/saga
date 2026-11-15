# Text messaging evaluation (issue #29 Step 1)

**Date:** 2026-11-15  
**Status:** Evaluation only — no messaging product code in this deliverable.  
**Issue:** [Text messaging: evaluate existing open-source options before building anything](https://github.com/Supermagnum/saga/issues/29)  
**Blocked follow-up:** [Encrypted Saga-to-Saga text messaging](https://github.com/Supermagnum/saga/issues/41) starts only if this evaluation recommends **C**.

## Goal reminder

Saga needs a way to send text to the same contacts it can call securely.
Ordinary SMS/MMS is out of scope (`ROLE_SMS` is not taken). What Saga uniquely
has today: per-contact long-term keys verified through a hardware token path
(Galdralag now; Nitrokey planned), Iroh peer-to-peer transport, and no
phone-number account.

This report evaluates existing messengers against the issue #29 criteria table
with sourced facts. It ends with an A / B / C recommendation for the project
owner to decide.

## Criteria (from issue #29)

| # | Criterion | Required? |
|---|-----------|-----------|
| 1 | Open source, licence compatible with AGPL-3.0, available on F-Droid | Yes |
| 2 | End-to-end encrypted 1:1 messaging | Yes |
| 3 | Long-term identity key can live on a hardware token (OpenPGP card / CCID), or the app can be extended to support that | Yes |
| 4 | No phone-number account and no mandatory central server (peer-to-peer or self-hostable) | Strongly preferred |
| 5 | Can be linked to Saga contacts (open a chat from the Saga contact/call screen via intent or deep link) | Yes |
| 6 | Works on GrapheneOS without Google Play services | Yes |
| 7 | Offline delivery (store-and-forward) when the recipient is not online | Preferred |
| 8 | Actively maintained (releases and commits in the last 12 months) | Yes |

**Pass marks used here:** GPL-3.0 / AGPL-3.0 (and compatible) count for criterion 1.
"Available on F-Droid" means a listing in f-droid.org or an official project F-Droid
repo that GrapheneOS users routinely add. Criterion 3 is scored against **messaging
identity keys**, not merely database-at-rest keys in the TEE/Keystore.

## Summary matrix

| Candidate | 1 Licence+F-Droid | 2 E2E | 3 HW identity | 4 No phone / no mandatory central | 5 Intent/deep link | 6 GrapheneOS | 7 Offline | 8 Maintained | Fits A? |
|-----------|-------------------|-------|---------------|-----------------------------------|--------------------|--------------|-----------|--------------|---------|
| Delta Chat | Pass | Pass | Fail | Pass (email / chatmail; self-hostable) | Weak / regressing | Pass | Pass (email) | Pass | No |
| Briar | Pass | Pass | Fail | Pass (P2P + optional mailbox) | Fail (no public chat URI) | Pass | Pass (mailbox) | Weak (maintenance mode) | No |
| SimpleX | Pass | Pass | Fail | Pass (no user IDs; self-hostable SMP) | Pass (`simplex.chat` links) | Pass | Pass (queues) | Pass | No |
| Conversations / Cheogram | Pass | Pass | Partial (OpenPGP via OpenKeychain only; OMEMO default) | Pass (self-host XMPP) | Pass (`xmpp:`) | Pass | Pass (server) | Pass | No |
| Jami | Pass | Pass | Fail | Pass (P2P DHT) | Unclear / weak | Pass (docs) | Weak (both online / DHT) | Pass | No |
| Molly | Partial (own F-Droid repo; AGPL) | Pass | Fail | Fail (phone number + Signal servers) | Pass (Signal-style) | Pass (FOSS / WebSocket) | Pass | Pass* | No |
| Session | Pass (F-Droid listing; GPL) | Pass | Fail | Partial (no phone; Oxen service nodes) | Weak | Pass | Pass | Pass | No |
| Iroh-based chats | Fail (none on F-Droid as of 2026-11-15) | Varies | Fail | Often Pass | Varies | Untested | Varies | Immature | No |

\*Molly timing risk: third-party reviews in late 2026 noted releases lagging Signal; verify before any handoff plan.

---

## Candidate details

### Delta Chat

| Criterion | Result | Facts (sourced) |
|-----------|--------|-----------------|
| 1 | Pass | GPLv3+; on F-Droid as `com.b44t.messenger` ([F-Droid](https://f-droid.org/packages/com.b44t.messenger/), [GitHub license](https://github.com/deltachat/deltachat-android)). |
| 2 | Pass | Guaranteed E2E via Autocrypt / SecureJoin; ETH analysis and fixes documented ([2024-03-25 blog](https://delta.chat/en/2024-03-25-crypto-analysis-securejoin)). |
| 3 | Fail | Keys are managed inside the core; OpenKeychain integration was declined as not a priority ([issue #827](https://github.com/deltachat/deltachat-android/issues/827)). Help text: additional private keys cannot be imported; DC generates Autocrypt keys ([issue #152 comment 2025-04](https://github.com/deltachat/deltachat-android/issues/152)). No OpenPGP-card / CCID identity path. |
| 4 | Pass | No phone number; uses email or chatmail; self-hostable ([F-Droid description](https://f-droid.org/packages/com.b44t.messenger/)). |
| 5 | Weak | Historically `mailto:` / `openpgp4fpr:` ([uri-schemes.md](https://github.com/deltachat/interface/blob/main/uri-schemes.md)). Android **v2.64.0** changelog: "Don't register app for mailto links handling" ([CHANGELOG](https://github.com/deltachat/deltachat-android/blob/main/CHANGELOG.md)) — handoff from Saga became harder. |
| 6 | Pass | Pure F-Droid client; no Play Services required. |
| 7 | Pass | Store-and-forward via mail / chatmail servers. |
| 8 | Pass | F-Droid suggested version 2.69.0 added 2026-11-13. |

**Also:** Delta Chat uses Iroh for **webxdc realtime** channels, not as the primary 1:1 message transport ([HN / DC blog context](https://news.ycombinator.com/item?id=43262510)). That does not satisfy Saga's "same keys as calls" product need.

### Briar

| Criterion | Result | Facts (sourced) |
|-----------|--------|-----------------|
| 1 | Pass | GPL-3.0-or-later; F-Droid `org.briarproject.briar.android` ([F-Droid](https://f-droid.org/packages/org.briarproject.briar.android/)). |
| 2 | Pass | Peer-to-peer E2E messaging ([briarproject.org](https://briarproject.org/)). |
| 3 | Fail | No documented OpenPGP-card / CCID long-term identity; keys are app-local. |
| 4 | Pass | No central server; Bluetooth / Wi-Fi / Tor sync; optional Briar Mailbox ([F-Droid](https://f-droid.org/packages/org.briarproject.briar.android/)). |
| 5 | Fail | No documented public URI scheme to open a chat for a Saga-mapped contact from another app. |
| 6 | Pass | Recommended for GrapheneOS / NitroPhone-style setups ([Nitrokey NitroPhone apps doc](https://github.com/Nitrokey/nitrokey-documentation/blob/main/source/components/nitrophone/apps.rst)). |
| 7 | Pass | Briar Mailbox for offline receive. |
| 8 | Weak | Still releasing (1.5.24, 2026-10-30) but **maintenance mode** announced 2026-07-09 ([briarproject.org](https://briarproject.org/)). |

### SimpleX Chat

| Criterion | Result | Facts (sourced) |
|-----------|--------|-----------------|
| 1 | Pass | AGPL-3.0-or-later; F-Droid `chat.simplex.app` ([F-Droid](https://f-droid.org/packages/chat.simplex.app/), [GitHub](https://github.com/simplex-chat/simplex-chat/)). |
| 2 | Pass | Double-ratchet E2E; open SimpleX Messaging Protocol ([F-Droid / README](https://github.com/simplex-chat/simplex-chat/)). |
| 3 | Fail | Identity is not an OpenPGP-card key. Android Keystore / planned StrongBox protect the **local database key**, not a CCID messaging identity ([issue #5755](https://github.com/simplex-chat/simplex-chat/issues/5755)). A third-party fork exploring YubiKey PIV for DB unlock is not upstream identity-on-card. |
| 4 | Pass | No user IDs / phone numbers; operators run disposable SMP relays; self-hostable ([project docs](https://simplex.chat)). |
| 5 | Pass | HTTPS / app-link deep links via `simplex.chat` + `assetlinks.json` ([multiplatform README](https://github.com/simplex-chat/simplex-chat/blob/stable/apps/multiplatform/README.md)). |
| 6 | Pass | F-Droid FOSS flavor; no Play Services required. |
| 7 | Pass | Message queues on SMP servers until fetched. |
| 8 | Pass | F-Droid suggested 7.1.2 added 2026-11-13. |

### Conversations / Cheogram (XMPP + OMEMO)

| Criterion | Result | Facts (sourced) |
|-----------|--------|-----------------|
| 1 | Pass | GPL-3.0; Conversations `eu.siacs.conversations`, Cheogram `com.cheogram.android` on F-Droid ([Conversations](https://f-droid.org/packages/eu.siacs.conversations/), [Cheogram](https://f-droid.org/packages/com.cheogram.android/)). |
| 2 | Pass | OMEMO by default; optional OpenPGP ([Conversations F-Droid feature list](https://f-droid.org/packages/eu.siacs.conversations/)). |
| 3 | Partial | **OMEMO device keys are software.** Optional OpenPGP path uses OpenKeychain, which supports some OpenPGP cards / tokens ([OpenKeychain site](https://www.openkeychain.org/), [Security Tokens wiki](https://github.com/open-keychain/open-keychain/wiki/Security-Tokens)). Nitrokey 3 + Android OpenPGP remains fragile / incomplete ([Nitrokey forum](https://support.nitrokey.com/t/support-nitrokey-3-openpgp-on-android-openkeychain/4946), [Android status](https://support.nitrokey.com/t/nitrokey-start-3a-mini-and-android/5786/2)). This is **not** Saga's Brainpool / ephemeral-session identity. |
| 4 | Pass | No phone-number account required; any modern XMPP server, including self-hosted / Snikket. |
| 5 | Pass | Standard `xmpp:` URIs; Conversations / Cheogram support them ([XMPP wiki](https://wiki.xmpp.org/web/XMPP_URIs)). |
| 6 | Pass | F-Droid builds without FCM requirement (UnifiedPush optional). |
| 7 | Pass | Server store-and-forward while recipient offline. |
| 8 | Pass | Recent F-Droid builds (Conversations 2.21.x; Cheogram 2.19.x in 2026). |

### Jami

| Criterion | Result | Facts (sourced) |
|-----------|--------|-----------------|
| 1 | Pass | GPL-3.0-or-later; F-Droid `cx.ring` ([F-Droid](https://f-droid.org/packages/cx.ring/)). |
| 2 | Pass | E2E messaging / calls; account = asymmetric key / X.509 ([FAQ](https://docs.jami.net/user/faq.html)). |
| 3 | Fail | Account keys are software RSA/X.509 via GnuTLS; no OpenPGP-card identity ([introduction](https://docs.jami.net/user/introduction.html)). |
| 4 | Pass | Distributed DHT; no mandatory central account server ([docs](https://docs.jami.net/user/introduction.html)). |
| 5 | Weak | No clear, documented cross-app "open chat with this contact" URI comparable to `xmpp:` / SimpleX invite links for Saga contact mapping. |
| 6 | Pass | Official GrapheneOS install path via F-Droid ([docs](https://docs.jami.net/setup/graphene-os.html)). |
| 7 | Weak | Designed around peers / DHT connectivity; not a classic mailbox model. |
| 8 | Pass | F-Droid suggested build dated 2026-11 (version `20261106-01`). |

### Molly (Signal fork)

| Criterion | Result | Facts (sourced) |
|-----------|--------|-----------------|
| 1 | Partial | AGPL-3.0-only ([GitHub](https://github.com/mollyim/mollyim-android/)). Not on main f-droid.org; distributed via [Molly's own F-Droid repos](https://molly.im/fdroid/). Acceptable for GrapheneOS users who add the repo; weaker than criterion 1's plain reading. |
| 2 | Pass | Signal Protocol E2E. |
| 3 | Fail | Signal identity keys are software; no OpenPGP-card path. |
| 4 | Fail | Registration requires a phone number ([README](https://github.com/mollyim/mollyim-android/)); uses Signal's centralized servers. |
| 5 | Pass | Same URL / intent patterns as Signal ecosystem for opening chats (contact must already be a Signal/Molly user). |
| 6 | Pass | FOSS path without Play Services (WebSocket / UnifiedPush) ([Fosstodon](https://fosstodon.org/@mollyim/115791864290732496)). |
| 7 | Pass | Server store-and-forward. |
| 8 | Pass* | Active project; external reviews in Oct 2026 flagged version lag vs upstream Signal — treat maintenance risk as elevated until checked at decision time. |

### Session

| Criterion | Result | Facts (sourced) |
|-----------|--------|-----------------|
| 1 | Pass | GPL-3.0; F-Droid listing `network.loki.messenger.fdroid` ([F-Droid](https://f-droid.org/packages/network.loki.messenger.fdroid/), [session-android](https://github.com/session-foundation/session-android)). |
| 2 | Pass | E2E encrypted messenger ([getsession.org / Oxen docs](https://docs.oxen.io/oxen-docs/products-built-on-oxen/session)). |
| 3 | Fail | Account ID / key material is software; no OpenPGP-card identity. |
| 4 | Partial | No phone number / email ([F-Droid description](https://f-droid.org/packages/network.loki.messenger.fdroid/)). Relies on Oxen Service Node network (F-Droid AntiFeature `NonFreeNet`: depends on getsession.org servers) — not a user-run single server, but also not a classic P2P mesh. |
| 5 | Weak | No first-class Saga-style contact deep link documented for arbitrary contacts. |
| 6 | Pass | F-Droid build without Firebase "fast mode". |
| 7 | Pass | Service Nodes store messages offline. |
| 8 | Pass | F-Droid suggested 1.33.8 added 2026-11-09. |

### Iroh-based chat projects

Evaluated as a class (issue #29: "any Iroh-based chat"):

| Project | Licence | F-Droid | Maturity | HW identity | Notes |
|---------|---------|---------|----------|-------------|-------|
| [Telepathy](https://github.com/chanderlud/telepathy/) | MIT | No | Small / WIP | No | Flutter + Iroh; voice/text; GitHub releases only. |
| [jamessizeland/peer-to-peer](https://github.com/jamessizeland/peer-to-peer) | Apache-2.0 | No | PoC | No | Tauri + Iroh Gossip example. |
| [Azula](https://github.com/Azula-App/azula-app) | Unclear / early | No | Prototype | No | Compose Multiplatform + Iroh. |
| [ADA Messenger](https://github.com/vlouriet-lab/ada-messenger) | AGPL-3.0 | No | Beta | No | Iroh QUIC + Double Ratchet; APK via GitHub Releases. |

**Criterion 1 fails** for all (no F-Droid). None implement OpenPGP-card long-term identity aligned with Saga. Reusing Iroh transport is interesting for a future **C** build, but these apps are not A/B candidates today.

---

## Criterion 3 deep dive (why nothing clears the bar)

Saga's long-term identity is a hardware-held key used to **sign** ephemeral session offers
(Brainpool + ECDSA via the Galdralag / planned Nitrokey `SessionToken` path). Messaging that
is "the same contacts" in the strong sense (issue #41) needs that same trust root — not a
parallel username in another app.

| Approach in the wild | Why it fails Saga criterion 3 |
|----------------------|-------------------------------|
| OpenPGP Autocrypt (Delta Chat) | Keys generated in-app; no card; import of extra private keys rejected. |
| OpenKeychain + Conversations OpenPGP | Possible for *some* tokens on a secondary OpenPGP path; OMEMO remains default; not Saga's curve/protocol; NK3 Android support poor; OpenKeychain maintenance constrained. |
| Android Keystore / StrongBox (SimpleX DB key) | Protects local storage, not a user-portable OpenPGP-card / CCID messaging identity shared with Saga calls. |
| Signal / Molly / Session / Jami / Briar software identities | No OpenPGP-card / CCID identity key. |

Extending any of the above upstream to speak Saga's ephemeral-session wire format and
Brainpool token semantics is effectively building Saga messaging inside another product
(high friction, low likelihood of upstream acceptance). That is not a realistic **B**.

---

## Options A / B / C

### A — Integrate via intent / deep link

Would require a candidate that **passes all required criteria (1, 2, 3, 5, 6, 8)** today.
**None do**, primarily because of criterion 3, with additional gaps on 5 (Delta Chat mailto
regression; Briar/Jami/Session weak handoff) and 4/1 for Molly.

Even a best-effort handoff (e.g. SimpleX invite link or `xmpp:` URI stored beside a Saga
contact) would create a **second identity** that is not verified by the Galdralag / Nitrokey
path used for Encrypted calls. That conflicts with the product framing in issues #29 and #41.

**Prototype Message action:** not implemented. Part 3 allows a debug-only prototype only
"if A looks right"; it does not.

### B — Contribute upstream

Closest theoretical targets:

1. **Delta Chat core** — add OpenPGP-card / external key support. Upstream historically
   declined OpenKeychain integration; Autocrypt model resists manual key management.
2. **Conversations + OpenKeychain** — improve card support (esp. Nitrokey 3) and prefer
   hardware-backed OpenPGP. Still leaves OMEMO as the real messaging crypto and does not
   share Saga call keys.
3. **SimpleX** — add CCID / OpenPGP-card identity. Large protocol change; still separate
   from Saga's handshake.

None of these yield "message the contact you just called with the same verified key" without
Saga-specific protocol work. **B is rejected as the primary path.**

### C — Build in Saga

Build encrypted Saga-to-Saga text only if the owner confirms this recommendation (issue #41).
Minimum v1 shape already sketched there: separate Iroh ALPN, same ephemeral-session handshake
and trust UX as calls, no new crypto primitives, encrypted at rest, no SMS, no silent
unencrypted send.

---

## Recommendation

**Recommend C — build encrypted text messaging in Saga** (only after the owner records the
decision on issue #29 and unblocks issue #41).

**Rationale (short):**

1. **Required criterion 3 fails for every candidate** as a Saga-aligned hardware identity.
2. **A** would at best open a parallel messenger identity and still miss criterion 3.
3. **B** cannot deliver shared call/message trust without Saga protocol work that belongs in
   Saga itself.
4. Saga already owns the hard parts messaging would reuse: verified contacts, token-backed
   session establishment, Iroh transport, and the Encrypted / not-encrypted / §5a UX.

**Not recommended:** taking `ROLE_SMS`, building unencrypted SMS, or shipping a release
"Message" button that hands off to a third-party app under an A pretence.

## Open decisions for the owner

1. Confirm **A / B / C** on issue #29 (this report recommends **C**).
2. If **C**: keep issue #41 open and schedule implementation after Parts 4–5 (or as agreed);
   decide offline delivery (both-online outbox vs self-hostable mailbox) before coding it.
3. If the owner instead wants a **weak A** (convenience handoff despite criterion 3 failure):
   say so explicitly; SimpleX deep links or `xmpp:` (Conversations/Cheogram) are the least-bad
   intent targets — still debug-flag only until decided.
4. SMS/MMS remains out of scope regardless of A/B/C.

## Acceptance checklist (issue #29 Step 1)

- [x] `docs/messaging-evaluation.md` exists with sourced facts per candidate.
- [ ] Owner records A/B/C decision on issue #29 (**waiting**).
- [x] No A prototype (A not recommended).
- [ ] If C confirmed: implementation proceeds under issue #41 (not this PR).
