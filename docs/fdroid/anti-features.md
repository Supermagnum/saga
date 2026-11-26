# F-Droid AntiFeatures assessment (Saga)

Honest assessment for a future fdroiddata submission. Not a submission itself.

| AntiFeature | Apply? | Reasoning |
|-------------|--------|-----------|
| **NonFreeNet** | **Yes (recommended)** | Release defaults to n0 public relays (`presets::N0`, `*.relay.n0.iroh.link`) and n0 discovery. Relay *software* is open source, but the default service is operated by n0 and is not user-replaceable in this build (no in-app self-hosted relay URL). Same class of concern as apps that default to a third-party network they do not operate. Prefer declaring `NonFreeNet` with an en-US explanation that the relay code is FOSS and defaults to operators Saga does not run. |
| **TetheredNet** | Optional / overlap | If maintainers prefer `TetheredNet` over `NonFreeNet` for “talks to specific hosts not trivial to replace”, that also fits. Do not double-list without guidance from F-Droid review; **pick one** — this draft recommends **NonFreeNet**. |
| **NonFreeDep** | No | Native stack is built from source (Rust, NDK, Opus via cmake). No proprietary SDKs. |
| **NonFreeAdd** | No | No proprietary add-ons. |
| **Tracking** | No | No analytics / crash reporters shipped. |
| **Ads** | No | No ads. |
| **UpstreamNonFree** | No | Upstream Saga is AGPL; dependencies are FOSS. |
| **KnownVuln** | No | None known at packaging time; revisit each release. |
| **DisabledAlgorithm** | No | Not applicable. |
| **NoSourceSince** | No | Source on GitHub. |

## Application ID

Current `applicationId` / recipe basename: **`org.saga`**.

Part 5 deliberately does **not** change the ID. Changing it is a one-line edit in `android/app/build.gradle.kts` (`applicationId = "…"`) plus renaming `docs/fdroid/org.saga.yml` and Fastlane package paths. **Decide the final ID before F-Droid submission.**
