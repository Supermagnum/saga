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
cargo ndk -o target/ndk-libs -t arm64-v8a -t x86_64 build --release --features iroh-transport,mock-token
```

Output: `target/ndk-libs/` with per-ABI `libsaga_iroh_core.so` files consumed by
`android/app/build.gradle.kts` (`preBuild` runs this automatically when `cargo-ndk` is installed).

## Features

| Feature | Description |
|---------|-------------|
| *(default)* | Native thread stub (short sleep, no QUIC) |
| `iroh-transport` | Real `iroh::Endpoint` with ALPN `saga/voice/1`, accept loop, session map |
| `mock-token` | Dev handshake + media round-trip probe (E2E) |

The `iroh` dependency enables `test-utils` so E2E can use a **local HTTP dev relay** with `CaTlsConfig::insecure_skip_verify()`. Release builds must use public relays with normal TLS — do not ship the test relay path to production.

## Dev peer labels (E2E)

Dial targets may be a hex/base32 `EndpointId`, or a dev label (8+ alphanumeric) such as
`15550100010`. Dev labels map to a deterministic `SecretKey` via BLAKE3 (`saga-dev-v1:{label}`).

Current E2E uses **phone-digit labels** matching contact pubkey endpoints:

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
