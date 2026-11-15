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

## Local checks

```bash
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
export SAGA_REQUIRE_NATIVE=1
export CMAKE_POLICY_VERSION_MINIMUM=3.5
cd android && ./gradlew assembleRelease verifyReleaseApkHygiene --no-daemon
```

Optional: build twice with the same `SOURCE_DATE_EPOCH` and compare APKs — see `reproducible-builds.md`.

Optional: `fdroid lint` / `fdroid build` inside the fdroidserver Docker image against a checkout of this recipe (report results; do not publish).
