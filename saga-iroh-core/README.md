# saga-iroh-core

JNI native library for Saga Iroh call transport on Android.

## Build for Android

Requires [cargo-ndk](https://github.com/bbqsrc/cargo-ndk):

```bash
cargo install cargo-ndk
rustup target add aarch64-linux-android x86_64-linux-android
```

From this directory:

```bash
# Debug / CI (no fat LTO) — default from Gradle assembleDebug
cargo ndk -o target/ndk-libs -t arm64-v8a -t x86_64 build --profile ci --features iroh-transport,mock-token

# Release / F-Droid (LTO + codegen-units=1)
cargo ndk -o target/ndk-libs -t arm64-v8a -t x86_64 build --release --features iroh-transport,mock-token
```

Output: `target/ndk-libs/` with per-ABI `libsaga_iroh_core.so` files consumed by
`android/app/build.gradle.kts` (`preBuild` runs this automatically when `cargo-ndk` is installed).
CI sets `CI=true` so Gradle builds **x86_64 only** with `[profile.ci]`; release packaging keeps both ABIs and `[profile.release]`.

## Features

| Feature | Description |
|---------|-------------|
| *(default)* | Native thread stub (short sleep, no QUIC) |
| `iroh-transport` | Real `iroh::Endpoint` with ALPN `saga/voice/1`, accept loop, session map |
| `mock-token` | Thin wrappers around `saga-session` + Galdr FakeTrng (E2E / debug). Production session logic lives in `saga-session` (always linked). See root `NOTICE`. |

### Session crypto

- **`saga-session`**: production Brainpool handshake, confirmation MAC (§12), channel binding, media AEAD. Always compiled into the `.so`.
- **`testing/mock-token`**: debug-only Galdr FakeTrng + identity/media helpers. Linked only with `--features mock-token`.
- **`iroh-roq`**: RoQ RTP-over-QUIC (datagrams); git-rev pin of the Saga fork ported to iroh 1.x (`docs/upstream/iroh-roq-port.md`). Wired with `iroh-transport`; media pipeline is Phase 2 Step 2+.

`testing/mock-token` / `saga-session` pin Galdralag-firmware via git **rev**
`b781bf6fa5ccf8700a5fb061583c31cfa00241a1` (**2026-10-22**), never a branch tip.
Licence: **GPL-3.0** (compatible with Saga AGPL-3.0).

The `iroh` dependency enables `test-utils` only under `mock-token` so E2E can use a **local HTTP dev relay** with `CaTlsConfig::insecure_skip_verify()`. Release builds (`iroh-transport` only) must not ship the test-relay path.

Toolchain: root [`rust-toolchain.toml`](../rust-toolchain.toml). Android NDK is pinned via `ndkVersion` in `android/app/build.gradle.kts`. CI sets `SAGA_REQUIRE_NATIVE=1` so a failed `cargo ndk` build fails the job (no silent stub).

## Dev peer labels (E2E)

Dial targets may be a hex/base32 `EndpointId`, or a dev label (8+ alphanumeric) such as
`15550100010`. Dev labels map to a deterministic **Iroh** `SecretKey` via BLAKE3
(`saga-dev-v1:{label}`). Token long-term Brainpool keys are separate
(`saga-mock-token-v2-brainpool:{label}:{curve_wire_id}`). Contacts store the
token fingerprint (64 hex), not the Iroh key.

Current E2E uses **phone-digit labels** for Iroh identity:

| Emulator | `peer_label` in `saga_dev_identity.xml` |
|----------|----------------------------------------|
| bob (5556) | `15550100010` |
| alice (5554) | `15550100011` |

`SagaApplication` calls `nativeSetDevIdentity` on start, which binds the shared endpoint and spawns `endpoint.online()` for relay registration.

### Custom relay URL (E2E)

`nativeSetRelayUrl(url)` must be called **before** the first bind (app does this from `saga_iroh_relay.xml` when present). When set, the endpoint uses `RelayMode::Custom` instead of public N0 relays.

E2E local default: `http://10.0.2.2:3340` (host `iroh-relay --dev`, port 3340).

## Relay readiness

`nativePollRelayReady()` returns:

| Value | Meaning |
|-------|---------|
| `0` | Pending — endpoint not bound or `online()` not complete |
| `1` | Ready — `endpoint.online().await` finished |
| `2` | Failed (reserved) |

The E2E harness polls this via `TEST_RELAY_QUERY` (writes `files/relay_status.txt`), not logcat markers.

## JNI surface

Kotlin class `org.saga.iroh.IrohNativeBridge`:

| Method | Purpose |
|--------|---------|
| `nativeSetRelayUrl(relayUrl)` | Optional custom relay (before bind); empty clears override |
| `nativeSetDevIdentity(peerLabel)` | Dev identity + callee listen bind |
| `nativePollRelayReady()` | `endpoint.online()` completion state |
| `nativeConnect(sessionId, peerId)` | Async connect; `notifyConnected` / `notifyFailed` |
| `nativeDisconnect(sessionId)` | Tear down session |
| `nativePollHandshake(sessionId)` | Handshake outcome (mock-token E2E) |
| `nativeSetForceHandshakeFail(force)` | Test hook for downgrade / failure paths |
| `nativeIsAvailable()` | Library loaded |
| `nativeLocalEndpointId()` | Local endpoint id after bind |

## E2E

Two-emulator suite (local relay recommended):

```bash
bash testing/iroh-relay/start-local-relay.sh
SERIAL_A=emulator-5554 SERIAL_B=emulator-5556 bash testing/e2e/run-e2e-full.sh
```

See [`testing/e2e/README.md`](../testing/e2e/README.md) for harness details, forensic scripts, and public-vs-local relay comparison.
