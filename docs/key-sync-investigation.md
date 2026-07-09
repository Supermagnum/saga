# Galdralag-firmware key sync — investigation (Step 5)

**Date:** 2026-07-09  
**Source repo:** `https://github.com/Supermagnum/Galdralag-firmware` (shallow clone at investigation time)  
**Status:** Investigation only — no Saga implementation until reviewed.

---

## 1. Multi-identifier keyring support

**On the token (firmware):** The device exposes OpenPGP card slots (**SIG / DEC / AUT**) and vault-backed long-term keys. There is no on-token contact directory with multiple lookup identifiers per peer key. Handshake traffic uses the `ephemeral-session` crate (init/response/transcript over a bidirectional stream), not a name-indexed keyring read from flash.

**On the host (Galdra / `galdra-core-host`):** Contact public keys and rich metadata live in a **local SQLite `identities` table**, not in a GnuPG keyring file. Each row supports multiple optional identifiers used for lookup:

| Field | Lookup in `resolve_contact_identifier` |
|-------|----------------------------------------|
| Row UUID | Yes |
| Callsign | Yes (unique column) |
| Email | Yes |
| OpenPGP fingerprint (40 hex) | Yes |
| Fluxer id | Yes |
| Discord id | Yes |
| IRC id | Yes |
| DMR subscriber id (1..16777215) | Yes |
| Phone number | Stored; searchable via `contact_search`, not in the primary resolver chain |
| Display name | Search only |

Reference: `galdra-core-host/src/contacts.rs` (`Identity` struct, `resolve_contact_identifier`).

**Saga implication:** Multi-identifier lookup for Android Contacts / dial resolution is a **Saga-side design** (mapping phone, name, callsign, etc. to stored pubkey rows). It is not something to bulk-sync verbatim from token firmware — the firmware does not maintain that directory.

Saga spec §13’s “name-tag / keyring model” aligns with **Galdra host contacts**, not an on-device keyring.

---

## 2. “Fluxer ID”

The term **does appear** in Galdralag-firmware, but only in **host-side contact metadata** and planned WoT server docs — not in token APDU or vault layout.

From `docs/server.md`:

> **Fluxer ID** — UTF-8, max 128 Unicode scalars — submitter-declared handle or opaque id on **Fluxer** (not verified against a remote service in v1).

From `docs/GALDRA-TOOL.md` / `README.md`: optional `--fluxer-id` on `galdra contact add`, stored in SQLite `fluxer_id`, resolvable like callsign or email.

**Not defined:** What Fluxer the platform/service is beyond “submitter-declared handle.” No Fluxer API integration in firmware v1. **Clarification still useful** for Saga product intent (is this a specific chat platform the user expects to match?).

---

## 3. Available sync mechanism

| Mechanism | What it syncs | Protocol |
|-----------|---------------|----------|
| **`galdra sync export` / `sync import`** | Host SQLite contacts + groups (public keys, metadata) | Offline SQLite package (`galdra-core-host/src/sync.rs`); merge or replace modes |
| **`galdra contact fetch`** | Single contact pubkey from HKP / WKD / LDAP | Network fetch into SQLite |
| **`galdra contact refresh`** | Re-fetch stored contacts from configured sources | Per-row |
| **`galdrad` HTTP API** | CRUD on `/contacts`, groups | Local daemon over host DB |
| **Token APDU (`key_export_public`, ephemeral-session)** | One slot’s public key or one handshake | PC/SC; no “export full keyring” bulk API found |

**Conclusion:** “Sync from token” in practice means either (a) export/import the **Galdra host database** between machines, or (b) iterate known contacts and refresh/fetch keys — not a single token command that dumps all trusted keys. Saga’s ContactsContract MIME rows would likely be populated from Galdra export/import or per-contact fetch, not a dedicated token keyring dump.

---

## 4. Amateur-radio identity conventions

The firmware repo and Galdra tooling already encode amateur-radio fields:

- **`callsign`** — unique optional column; ICAO-style token (e.g. `LA1BC`) in server docs
- **`dmr_id`** — integer 1..16777215 (DMR subscriber id)
- **`radio_affiliation`** — free text (e.g. `NRRL`, `ARRL`, club names)
- GitHub topics include **radioamateur**; `docs/GALDRA-TOOL.md` has an “Amateur radio operators” section (keys on HKP under callsign, emergency comms use cases)

These are **host SQLite metadata**, same trust model as Fluxer/IRC (not cryptographically verified unless correlated out-of-band).

**Saga implication:** When linking keys to callsigns, prefer mapping to existing Galdra field semantics rather than inventing a parallel scheme.

---

## Open questions (block implementation)

1. **Fluxer** — confirm with product owner which service/handle format Saga should match.
2. **Sync path for Saga Android** — import Galdra `sync export` SQLite on device, USB/NDK bridge to `galdrad`, or manual per-contact key paste?
3. **Phone number in resolver** — Galdra stores `phone_number` but `resolve_contact_identifier` does not try it; should Saga treat phone as first-class for keyed lookup (already does via ContactsContract)?

---

## Recommended next step (after review)

Design a one-way **Galdra export → Saga ContactsContract** importer (public keys + optional identifier columns), without assuming token-resident multi-key storage.
