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
cargo ndk -o target/ndk-libs -t arm64-v8a -t x86_64 build --release --features iroh-transport
```

Output: `target/ndk-libs/` with per-ABI `libsaga_iroh_core.so` files consumed by
`android/app/build.gradle.kts` (`preBuild` runs this automatically when `cargo-ndk` is installed).

## Features

| Feature | Description |
|---------|-------------|
| *(default)* | Native thread stub (short sleep, no QUIC) |
| `iroh-transport` | Real `iroh::Endpoint::connect` with ALPN `saga/voice/1`, accept loop, session map |

## Dev peer labels (E2E)

Dial targets may be a real hex/base32 `EndpointId`, or a dev label (8+ alphanumeric) such as
`bobpeer12`. Dev labels map to a deterministic `SecretKey` via BLAKE3 (`saga-dev-v1:{label}`).

Seed the callee emulator with `shared_prefs/saga_dev_identity.xml` (`peer_label`) before first
launch so `SagaApplication` calls `nativeSetDevIdentity` and binds that identity. See
`testing/e2e/saga_dev_identity_bobpeer12.xml` and `provision-emulator.sh` (`DEV_IDENTITY=`).

Two-emulator E2E:

```bash
SERIAL_A=emulator-5554 SERIAL_B=emulator-5556 ./testing/e2e/run-e2e-iroh.sh
```

## JNI surface

Kotlin class `org.saga.iroh.IrohNativeBridge`:

- `nativeSetDevIdentity(peerLabel)` — must run before first connect/bind (E2E callee)
- `nativeConnect(sessionId, peerId)` — async connect; callbacks `notifyConnected` / `notifyFailed`
- `nativeDisconnect(sessionId)`
- `nativeIsAvailable()`
- `nativeLocalEndpointId()` — local endpoint id after bind
