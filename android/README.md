# Saga Android

Fresh default-dialer app (`ROLE_DIALER` + `InCallService` + managed `ConnectionService`).
No SIP, Linphone, or `CAPABILITY_SELF_MANAGED`.

## Build

```bash
cd android
gradle wrapper --gradle-version 8.11.1   # once
./gradlew assembleDebug test
```

`preBuild` runs `cargo ndk` in [`../saga-iroh-core`](../saga-iroh-core). If `cargo-ndk` is
missing, the APK still builds and falls back to `StubIrohCallSession`.

## Native Iroh core

See [`../saga-iroh-core/README.md`](../saga-iroh-core/README.md). JNI class:
`org.saga.iroh.IrohNativeBridge`. Factory prefers `NativeIrohCallSession` when
`libsaga_iroh_core.so` is packaged.

## Iroh dial

1. Grant default dialer role.
2. Enter peer node ID (8+ alphanumeric; `nokey` suffix = no Galdra key; `fail` = transport fail).
3. Tap **Call via Iroh** — `saga:<nodeId>` via managed `SagaConnectionService`.

E2E: [`../testing/e2e/run-e2e-iroh.sh`](../testing/e2e/run-e2e-iroh.sh)

## Section 13 icons

`app/src/main/res/drawable/ic_saga_*.xml` — simplified from [`../icons/`](../icons/) (AGPL).

## Audio cues

`res/raw/call_secure.wav` / `call_unsecure.wav` at connect time.
Mid-call warning (section 5b): `org.saga.security.midcall.SagaMidCallSecurityAudioWarning`.
