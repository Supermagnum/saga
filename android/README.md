# Saga Android

Fresh default-dialer app (`ROLE_DIALER` + `InCallService` + managed `ConnectionService`).
No SIP, Linphone, or `CAPABILITY_SELF_MANAGED`.

## Build

```bash
cd android
gradle wrapper --gradle-version 8.11.1   # once
./gradlew assembleDebug test
```

`preBuild` runs `cargo ndk` in [`../saga-iroh-core`](../saga-iroh-core).

- **Release (`assembleRelease` / `bundleRelease` / …):** native build ALWAYS
  required. Missing `libsaga_iroh_core.so` fails the build. Cannot be disabled
  by flags.
- **Debug + CI / `-Psaga.requireNative=true` / `SAGA_REQUIRE_NATIVE=1`:** same
  fail-hard behaviour. Use `./gradlew verifyNativeLibsInApk` to assert the
  debug APK packages the `.so`.
- **Local debug only (no flags):** if `cargo-ndk` is missing, the APK may still
  build and fall back to `StubIrohCallSession`, with a loud Gradle error-level
  warning. Never use that path for release.

## Native Iroh core

See [`../saga-iroh-core/README.md`](../saga-iroh-core/README.md). JNI class:
`org.saga.iroh.IrohNativeBridge`. Factory prefers `NativeIrohCallSession` when
`libsaga_iroh_core.so` is packaged.

## Iroh dial

1. Grant default dialer role.
2. Pick a contact or enter a dial target (phone number, contact name, or dev peer label).
3. Calls resolve via `DialTargetResolver` — cellular (`tel:`) or Iroh (`saga:`) via `SagaConnectionService`.

E2E (two emulators, local relay recommended): [`../testing/e2e/README.md`](../testing/e2e/README.md)

## Section 13 icons

`app/src/main/res/drawable/ic_saga_*.xml` — simplified from [`../icons/`](../icons/) (AGPL).

## Audio cues

`res/raw/call_secure.wav` / `call_unsecure.wav` at connect time.
Mid-call warning (section 5b): `org.saga.security.midcall.SagaMidCallSecurityAudioWarning`.
