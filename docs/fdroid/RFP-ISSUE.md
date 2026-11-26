# F-Droid RFP draft (Saga)

Paste into a new issue at https://gitlab.com/fdroid/rfp/issues
(title suggestion: `New App: Saga (org.saga)`).

Preferred path for upstream authors is an fdroiddata merge request; this RFP
is prepared so issuebot can scan the app while the metadata MR is reviewed.
Do not invent proprietary dependencies — see NOTICE and anti-features.md.

---

* [x] The app complies with the [inclusion criteria](https://f-droid.org/docs/Inclusion_Policy/).
* [x] The app is not already [listed](https://gitlab.com/search?scope=issues&group_id=28397) in the repo or issue tracker.
* [x] The app has not already [been requested](https://gitlab.com/search?scope=issues&project_id=2167965)
* [x] The upstream app source code repo contains the app metadata _(summary/description/images/changelog/etc)_ in a [Fastlane](https://gitlab.com/snippets/1895688) or [Triple-T](https://gitlab.com/snippets/1901490) folder structure
* [x] The original app author has been notified, and does not oppose the inclusion.
* [ ] Optionally [donated](https://f-droid.org/donate/) to support the maintenance of this app in F-Droid.

---------------------

This issue tracker is meant for anyone to get an automated review to start the
process of getting an app included. Opening issues here does not guarantee that the
app will be reviewed or packaged. If you are looking to submit an app to
F-Droid, please [open an merge request](https://gitlab.com/fdroid/fdroiddata/-/blob/master/CONTRIBUTING.md)
instead.

#### APPLICATION ID: org.saga

```yaml
AntiFeatures:
  NonFreeNet:
    en-US: Default encrypted calls use n0 public relays and discovery
      (*.relay.n0.iroh.link). Relay software is open source, but the default
      operators are not run by Saga. A self-hosted relay URL setting is
      planned; until then Private DNS (DoT) is recommended.

Categories:
  - Phone & SMS
  - Security

License: AGPL-3.0-or-later

AuthorName: Saga contributors

WebSite: https://github.com/Supermagnum/saga
SourceCode: https://github.com/Supermagnum/saga
IssueTracker: https://github.com/Supermagnum/saga/issues
Changelog: https://github.com/Supermagnum/saga/releases

AutoName: Saga

RepoType: git
Repo: https://github.com/Supermagnum/saga.git
```

Why do you want this app added to F-Droid:

> Upstream author submission. Saga is AGPL encrypted voice calling for Android
> (default dialer + Iroh transport). We want reproducible F-Droid builds that
> keep the developer signing key so GitHub and F-Droid installs can update each
> other. Draft recipe, AntiFeatures notes, and Fastlane metadata live in the
> upstream repo under `docs/fdroid/` and `fastlane/metadata/`. Full build recipe
> for fdroiddata: see `docs/fdroid/org.saga.yml` (also attached to the
> fdroiddata MR when opened).

Summary:

> Encrypted voice calling: default dialer with Iroh transport and hardware tokens

Description:

> Saga is a free/libre Android dialer that places ordinary cellular calls and
> encrypted peer-to-peer voice calls over the Iroh network.
>
> Encrypted Saga calls use a hardware SessionToken for the long-term signature.
> Supported today: Nitrokey OpenPGP cards with a Brainpool AUT key (USB).
> Galdralag hardware remains blocked on upstream firmware work. Without a
> supported token, Saga still works as a dialer; Encrypted state requires the
> token path.
>
> Transport defaults to n0 public relays (open-source relay software).
> Discovery and call metadata are visible to those operators. Encrypted call
> content is protected, but IP address, who you call, and timing/volume
> metadata remain visible unless you use a system-wide privacy-focused VPN.
>
> Licence: AGPL-3.0-or-later. Source: https://github.com/Supermagnum/saga
>
> First published build pin: versionName 0.1.0-preview.1, versionCode 1,
> git tag v0.1.0-preview.1 (commit 53dcd69bd9a914d2fd00c6a353a211e817dbad72).

### Packager notes

* No Google Play Services, Firebase, Crashlytics, ads, or analytics.
* Native `libsaga_iroh_core.so` is built from source (Rust + NDK + Opus via cmake).
* Release Cargo features: `iroh-transport,nitrokey` (no `mock-token`).
* Declared AntiFeature: **NonFreeNet** (default n0 public relays/discovery).
* Reproducible-build intent: `Binaries:` GitHub arm64-v8a APK +
  `AllowedAPKSigningKeys:` `7a6a218e8a497a51232ca11c9a10b362769c4969fa7dd26b359838a08cb7dd9a`.
* Upstream Fastlane: `fastlane/metadata/android/en-US/`.
* Draft fdroiddata recipe: https://github.com/Supermagnum/saga/blob/main/docs/fdroid/org.saga.yml
* Inclusion / FOSS notes: NOTICE, LICENSE (AGPL-3.0-or-later).
