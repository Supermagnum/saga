# F-Droid packaging prep (Part 5)

Prepare Saga for F-Droid; **do not** open the fdroiddata merge request from here.

| Artifact | Path |
|----------|------|
| Draft recipe | [`org.saga.yml`](org.saga.yml) |
| AntiFeatures notes | [`anti-features.md`](anti-features.md) |
| Fastlane metadata | [`../../fastlane/metadata/android/en-US/`](../../fastlane/metadata/android/en-US/) |
| Screenshots | `fastlane/metadata/android/en-US/images/phoneScreenshots/` |
| Release APK hygiene | [`../../scripts/check-release-apk.sh`](../../scripts/check-release-apk.sh) |
| Release `.so` hygiene | [`../../scripts/check-release-so.sh`](../../scripts/check-release-so.sh) |
| Reproducible notes | [`reproducible-builds.md`](reproducible-builds.md) |

## applicationId

Still **`org.saga`**. Part 5 does not change it — choose the final ID before submission; then edit `applicationId` in `android/app/build.gradle.kts` and rename this recipe.

## versionCode / versionName

`versionCode` is a **monotonic integer** (Part 5 scheme), kept in sync between
`android/app/build.gradle.kts` and this recipe's `CurrentVersionCode` /
`Builds[].versionCode`. `versionName` is a human string (SemVer + optional
pre-release label, e.g. `0.1.0-preview.1`).

## Developer signature (GitHub ↔ F-Droid updates)

F-Droid should use **reproducible builds** that retain the **developer
signature** (`Binaries:` pointing at the GitHub release APK, plus
`AllowedAPKSigningKeys:` with the signing certificate SHA-256 hex). That way
an install from GitHub and one from F-Droid can update each other.

Placeholders live as comments at the top of [`org.saga.yml`](org.saga.yml).
Fill them after the maintainer signs (see [`../release.md`](../release.md)).
Do **not** put keystore material in this repository.

## Local checks

```bash
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
export SAGA_REQUIRE_NATIVE=1
export CMAKE_POLICY_VERSION_MINIMUM=3.5
cd android && ./gradlew assembleRelease verifyReleaseApkHygiene --no-daemon
```

Optional: build twice with the same `SOURCE_DATE_EPOCH` and compare APKs — see `reproducible-builds.md`.

Optional: `fdroid lint` / `fdroid build` inside the fdroidserver Docker image against a checkout of this recipe (report results; do not publish).
