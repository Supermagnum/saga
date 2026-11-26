# F-Droid packaging

Saga F-Droid prep and submission status.

| Artifact | Path / URL |
|----------|------------|
| fdroiddata recipe (upstream copy) | [`org.saga.yml`](org.saga.yml) |
| AntiFeatures notes | [`anti-features.md`](anti-features.md) |
| RFP issue body (archive) | [`RFP-ISSUE.md`](RFP-ISSUE.md) |
| Fastlane metadata | [`../../fastlane/metadata/android/en-US/`](../../fastlane/metadata/android/en-US/) |
| Screenshots | `fastlane/metadata/android/en-US/images/phoneScreenshots/` |
| Release APK hygiene | [`../../scripts/check-release-apk.sh`](../../scripts/check-release-apk.sh) |
| Release `.so` hygiene | [`../../scripts/check-release-so.sh`](../../scripts/check-release-so.sh) |
| Reproducible notes | [`reproducible-builds.md`](reproducible-builds.md) |

## Submission status

| Step | URL / note |
|------|------------|
| RFP | https://gitlab.com/fdroid/rfp/-/work_items/4874 |
| fdroiddata MR | https://gitlab.com/fdroid/fdroiddata/-/merge_requests/59255 |
| Fork branch | https://gitlab.com/supermag/fdroiddata/-/tree/org.saga |

## applicationId

Still **`org.saga`**. Changing it later requires editing `applicationId` in
`android/app/build.gradle.kts`, renaming this recipe, and updating the
fdroiddata MR / Fastlane package paths before F-Droid publishes.

## versionCode / versionName (first pin)

| Field | Value |
|-------|-------|
| `versionName` | `0.1.0-preview.1` |
| `versionCode` | `1` |
| git tag / commit | `v0.1.0-preview.1` (`53dcd69bd9a914d2fd00c6a353a211e817dbad72`) |

Keep `android/app/build.gradle.kts` and `org.saga.yml` in sync when bumping.

## Developer signature (GitHub ↔ F-Droid updates)

F-Droid should use **reproducible builds** that retain the **developer
signature** (`Binaries:` pointing at the GitHub release APK, plus
`AllowedAPKSigningKeys:` with the signing certificate SHA-256 hex). That way
an install from GitHub and one from F-Droid can update each other.

[`org.saga.yml`](org.saga.yml) sets `Binaries:` to the GitHub arm64-v8a APK and
`AllowedAPKSigningKeys:` to certificate SHA-256
`7a6a218e8a497a51232ca11c9a10b362769c4969fa7dd26b359838a08cb7dd9a`
(see [`../release.md`](../release.md) and README “Verifying downloads”).
Do **not** put keystore material in this repository.

## Local checks

```bash
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
export SAGA_REQUIRE_NATIVE=1
export CMAKE_POLICY_VERSION_MINIMUM=3.5
cd android && ./gradlew assembleRelease verifyReleaseApkHygiene --no-daemon
```

Optional: build twice with the same `SOURCE_DATE_EPOCH` and compare APKs — see `reproducible-builds.md`.

Optional: `fdroid lint` / `fdroid build` inside the fdroidserver Docker image against a checkout of this recipe.
