# Reproducible build notes (Saga)

Goal: two `assembleRelease` runs with the same inputs produce bit-identical APKs (or a documented residual diff).

## Pins (keep in sync)

| Input | Pin |
|-------|-----|
| Rust | `rust-toolchain.toml` → `1.94.0` (CI + F-Droid recipe) |
| NDK | `27.2.12479018` / r27c (`android/app/build.gradle.kts`, CI, recipe) |
| Gradle | wrapper `8.10.2` |
| cargo-ndk | `3.5.4` |
| Cargo features (release) | `iroh-transport,nitrokey` (no `mock-token`) |

## Suggested compare

```bash
export SOURCE_DATE_EPOCH=1731628800   # example: fixed unix time
export SAGA_REQUIRE_NATIVE=1
export CMAKE_POLICY_VERSION_MINIMUM=3.5
export ANDROID_NDK_HOME=… ANDROID_HOME=…

cd android
./gradlew clean assembleRelease --no-daemon
cp app/build/outputs/apk/release/app-release-unsigned.apk /tmp/saga-a.apk

./gradlew clean assembleRelease --no-daemon
cp app/build/outputs/apk/release/app-release-unsigned.apk /tmp/saga-b.apk

sha256sum /tmp/saga-a.apk /tmp/saga-b.apk
# Optional deeper: unpack and diff classes.dex / lib/**/libsaga_iroh_core.so
```

## Known residuals

- Native Rust/LTO and cmake Opus builds may still embed non-deterministic
  metadata unless the full F-Droid environment sets `SOURCE_DATE_EPOCH`,
  `CFLAGS`/`LDFLAGS` `-ffile-prefix-map`, and stable path layouts.
- No signing keys are stored in this repository; F-Droid signs binaries.
- Treat bit-identity as a goal for the fdroiddata build VM; local developer
  paths often differ. Document any remaining diff when first building in
  fdroidserver Docker.
