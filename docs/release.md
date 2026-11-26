# Saga release builds (preview)

How to produce **unsigned**, zipaligned release APKs for maintainer signing.
The signing keystore **never** belongs in this repository, CI, or shared
automation.

## Version

| Field | Value |
|-------|-------|
| `applicationId` | `org.saga` (unchanged from Part 5) |
| `versionName` | `0.1.0-preview.2` |
| `versionCode` | `2` (monotonic integer; Part 5 / F-Droid scheme) |

Bump `versionCode` on every published APK. Keep
`android/app/build.gradle.kts` and `docs/fdroid/org.saga.yml` in sync.

## Toolchain pins (record what you used)

Keep these aligned with CI / F-Droid (`docs/fdroid/reproducible-builds.md`):

| Tool | Pin |
|------|-----|
| Rust | `1.94.0` (`rust-toolchain.toml`) |
| cargo-ndk | `3.5.4` |
| Android NDK | `27.2.12479018` (r27c) |
| Gradle | wrapper `8.10.2` |
| JDK | 17+ (CI uses the runner default; local preview used OpenJDK 21) |
| Android build-tools | `36.1.0` (zipalign / apksigner) |
| Release Cargo features | `iroh-transport,nitrokey` (no `mock-token`) |

## Build commands (unsigned APKs)

From a clean checkout of the release tag/commit:

```bash
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/27.2.12479018"
export ANDROID_NDK_ROOT="$ANDROID_NDK_HOME"
export SAGA_REQUIRE_NATIVE=1
export CMAKE_POLICY_VERSION_MINIMUM=3.5
# Fixed epoch for reproducibility compares (unix seconds).
export SOURCE_DATE_EPOCH=1731628800

printf 'sdk.dir=%s\n' "$ANDROID_HOME" > android/local.properties

mkdir -p out
ZIPALIGN="$ANDROID_HOME/build-tools/36.1.0/zipalign"

# cargoNdkBuild clears target/ndk-libs ABI dirs before each run so stale .so
# files are not packaged. For a double-build compare, also wipe Android cargo
# targets between runs (or use a fresh clone).

build_variant() {
  local abis="$1"
  local out_name="$2"
  rm -rf saga-iroh-core/target/ndk-libs
  cd android
  ./gradlew clean assembleRelease verifyReleaseApkHygiene --no-daemon \
    -Psaga.requireNative=true \
    -Psaga.abis="$abis"
  cd ..
  "$ZIPALIGN" -f -p 4 \
    android/app/build/outputs/apk/release/app-release-unsigned.apk \
    "out/${out_name}"
  "$ZIPALIGN" -c -v 4 "out/${out_name}"
  sha256sum "out/${out_name}"
}

# --- arm64-v8a only (real phones) ---
build_variant arm64-v8a saga-0.1.0-preview.2-arm64-v8a-unsigned.apk

# --- universal (arm64-v8a + x86_64) if size is acceptable ---
build_variant arm64-v8a,x86_64 saga-0.1.0-preview.2-universal-unsigned.apk
```

Hygiene must pass (`scripts/check-release-apk.sh` via
`verifyReleaseApkHygiene`). Artifacts under `out/` are local only (gitignored).

### Reproducibility check

Build each variant **twice** with the same `SOURCE_DATE_EPOCH` and toolchain
(fresh `ndk-libs`, Gradle `clean`). SHA-256 of the unsigned zipaligned APKs
must match. Record hashes in the PR / release checklist.

```bash
sha256sum out/saga-0.1.0-preview.2-arm64-v8a-unsigned.apk \
          out/saga-0.1.0-preview.2-universal-unsigned.apk
```

Local Part B double-build result for **0.1.0-preview.1**
(`SOURCE_DATE_EPOCH=1731628800`, same host toolchain; historical):

| APK | Size | SHA-256 |
|-----|------|---------|
| `out/saga-0.1.0-preview.1-arm64-v8a-unsigned.apk` | ~22 MiB | `fa95997609e2d03db0ad6a84f15aca1777da0941691b4a31b5916e055f80ee50` |
| `out/saga-0.1.0-preview.1-universal-unsigned.apk` | ~34 MiB | `03da7ff4cb54a62135790eac78f644a51272534e753442f5dd910d9001b43b44` |

Both variants matched across two clean `assembleRelease` + zipalign runs.

## Signing (maintainer only — local keystore)

Do **not** commit the keystore or share it with CI. Example with Android
build-tools `apksigner` (enable **v2 and v3**):

```bash
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
APKSIGNER="$ANDROID_HOME/build-tools/36.1.0/apksigner"

# Sign arm64 (repeat for universal with matching paths)
"$APKSIGNER" sign \
  --ks /path/to/your-release.keystore \
  --ks-key-alias YOUR_ALIAS \
  --v1-signing-enabled true \
  --v2-signing-enabled true \
  --v3-signing-enabled true \
  --out out/saga-0.1.0-preview.2-arm64-v8a.apk \
  out/saga-0.1.0-preview.2-arm64-v8a-unsigned.apk

# Verify schemes + print certificate digests
"$APKSIGNER" verify --verbose --print-certs \
  out/saga-0.1.0-preview.2-arm64-v8a.apk
```

Certificate SHA-256 (for README “Verifying downloads”, AppVerifier, and
`AllowedAPKSigningKeys` in the F-Droid recipe) — from `apksigner verify
--print-certs`, or:

```bash
keytool -list -v -keystore /path/to/your-release.keystore \
  -alias YOUR_ALIAS
# Use the SHA-256 line under the certificate fingerprint block.
# For F-Droid AllowedAPKSigningKeys: hex only, no colons, lowercase.
```

### Preview.1 signing certificate (recorded)

| Field | Value |
|-------|-------|
| Certificate SHA-256 | `7a6a218e8a497a51232ca11c9a10b362769c4969fa7dd26b359838a08cb7dd9a` |
| Schemes observed | APK Signature Scheme **v3** only (`v1`/`v2` false). minSdk 29 is fine with v3; re-sign with v2+v3 later if desired. |

Do **not** commit the keystore. Signed APKs are release assets only (not git).

## F-Droid note

Use reproducible builds with the developer signature (`Binaries:` +
`AllowedAPKSigningKeys:`) so GitHub and F-Droid installs can update each
other. Placeholders are in `docs/fdroid/org.saga.yml` (filled after signing).

---

## Draft release notes (`v0.1.0-preview.1`)

Plain-language text for the GitHub pre-release (mark as **pre-release**, not
Latest). Source: tag `v0.1.0-preview.1` —
https://github.com/Supermagnum/saga/tree/v0.1.0-preview.1

### Preview status

This is an early preview. It has been tested on **Android emulators only**
(API 35 and API 29), **not on real phones**. It has **not** been security
audited. Expect bugs.

**Iroh group calls are included** (up to four participants with pairwise /
mesh links). Carrier (cellular) conference behaviour is unchanged where the
device supports it.

### Encrypted calls and Nitrokey

Encrypted Saga-to-Saga calls need a **supported Nitrokey** with a **Brainpool
AUT** key (USB). Galdralag hardware support is still pending upstream work.
Without a supported token, Saga-to-Saga calls still work but show
**“Not encrypted”**.

### Network and privacy

Default transport uses **n0 public relays**. Read the Privacy section in the
README before installing. A system-wide privacy-focused **VPN** and Android
**Private DNS** (DoT) are recommended. **GrapheneOS** is the recommended OS.

### Default phone app

Installing Saga and granting the default-dialer role makes Saga your phone
app. To switch back to the stock dialer:

1. Open **Settings → Apps → Default apps → Phone app** (wording varies by OEM).
2. Choose your previous Phone / Dialer app.
3. You can uninstall Saga afterward if you no longer want it.

### Known limitations (needs real hardware)

From `TEST_RESULTS.md` (emulator-only / not verified on physical devices):

- Physical-phone install, audio path, and store-style screenshots
- USB OTG Nitrokey on a phone (permission, CCID, PIN, Encrypted call)
- Carrier merge / VoLTE conference (`getChildren()`); emulator C1/C2 are
  NEEDS_HARDWARE
- Dual-SIM multi-call edge cases
- WireGuard (or other) VPN verified on a real call path
- Reliable public-relay soak on device (emulator public discovery is flaky)
- API 29 first-call relay warm-up flake tracked as issue #49

### How to report problems

Use the **Preview feedback** GitHub issue template (device, Android version,
ROM, what happened, redacted logs). Do **not** post phone numbers or other
personal data.

### Source

AGPL-3.0-or-later. Source for this build:
https://github.com/Supermagnum/saga/tree/v0.1.0-preview.1
