# Nitrokey SessionToken (Part 4)

Galdralag hardware remains blocked on [xous-core#937](https://github.com/betrusted-io/xous-core/issues/937).
Nitrokey is the first **production** `SessionToken` so release builds can make
Encrypted calls when a supported card is present.

## Security model (weaker than Galdralag)

| Step | Galdralag | Nitrokey (this crate) |
|------|-----------|------------------------|
| Ephemeral keygen | On-token TRNG | Phone OS CSPRNG (`OsTrng`) |
| Ephemeral private key | Token SRAM only | Phone RAM until ECDH, then zeroised |
| Long-term sign | On-token | OpenPGP **AUT** via INTERNAL AUTHENTICATE |
| Wire format | `ephemeral-session` Init/Response | Same bytes |

Forward secrecy **per call** still holds after the ephemeral private key is
zeroised. A compromised phone process during the handshake can learn that
call's ephemeral sk; Galdralag does not have that window.

## Wire / crypto details (verified against pinned firmware)

- **Preimage (Init):** `version || curve_wire_id || ephemeral_pubkey_sec1`
- **Preimage (Response):** `version || curve_wire_id || resp_epk || init_epk`
- **Hash:** SHA-256(preimage) — same as Galdralag `sign_handshake_sha256_prehash`
- **Card:** ECDSA over that digest (OpenPGP ECC INTERNAL AUTH; digest sent as-is)
- **Wire signature:** DER ECDSA (same as Galdralag verify)
- **Fingerprint:** SHA-256(AUT SEC1 uncompressed pubkey), hex as 64 lowercase
  ASCII — **not** the OpenPGP v4 fingerprint shown by GnuPG

## Supported models / firmware (sourced)

| Device | Brainpool AUT | Notes / source |
|--------|---------------|----------------|
| **Nitrokey 3** with Secure Element | P-256 / P-384 / P-512 | [NK3 FAQ algorithms](https://docs.nitrokey.com/nitrokeys/nitrokey3/faq); SE default since firmware v1.7.0 |
| Nitrokey 3 **without** SE | Brainpool **not** available | Same FAQ / [SE05x doc](https://docs.nitrokey.com/nitrokeys/nitrokey3/secure-element) |
| **Nitrokey Pro 2** | Brainpool listed for OpenPGP ECC | [Nitrokey ECC feature doc](https://docs.nitrokey.com/nitrokeys/features/misc/ecc) / product OpenPGP Card 3.x |

Saga **reads** AUT algorithm attributes from the card and refuses anything other
than ECDSA on brainpoolP256r1 or brainpoolP384r1 with a clear
"card not supported" mapping (`TokenError::CardNotSupported`). No silent
fallback to software keys.

Host probe on this machine (2026-11-15): Nitrokey 3 `20a0:42b2`, firmware
v1.8.3, OpenPGP 3.4, AUT = **ECDSA BrainpoolP256r1** with public key present.

## NFC

**Not implemented.** Nitrokey 3 over NFC on Android is reported as FIDO2-only
for OpenPGP (OpenPGP-over-NFC unsupported). Saga uses USB Host CCID only
(`android.hardware.usb` + device filter).

## USB IDs (Android filter + host)

| Device | VID:PID |
|--------|---------|
| Nitrokey 3 | `20a0:42b2` |
| Nitrokey Pro 2 | `20a0:4108` |
| Galdralag Dabao (future, same CCID path) | `1d50:6197` |

## PIN UX

User PIN (OpenPGP PW1, P2=82 for INTERNAL AUTH) is collected in-app, never
logged. Retries / locked card / unplugged / timeout map to existing security
states (`PinRequired`, `CardLocked`, `Unavailable`, `Timeout`) → Not encrypted /
§5a as appropriate.

## Testing

| Layer | How |
|-------|-----|
| Host CI | `saga-token-usb` feature `test-card`: software AUT ↔ `MockSessionToken` interop |
| Host live | feature `pcsc` + plugged Nitrokey; `SAGA_NITROKEY_PIN=… cargo test -p saga-token-usb --features pcsc,test-card -- --ignored` |
| Emulator USB passthrough | Optional `-qemu -device usb-host,…`; report in TEST_RESULTS if it works |
| Android on phone | Needs real USB OTG + card (listed under needs-hardware) |

## Crate layout

- `saga-token-usb` — production (not under `testing/`)
- `NitrokeySessionToken` implements `saga_session::SessionToken`
- Shared `CardFactory` / `ApduTransport` for PC/SC (host) and Android USB CCID (JNI)
