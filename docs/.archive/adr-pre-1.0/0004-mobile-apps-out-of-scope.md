# ADR 0004 — mobile apps stay out of scope

- **Status**: Accepted (belaf 3.0)
- **Date**: 2026-05-01

## Context

belaf's wizard auto-detects iOS (`*.xcodeproj/project.pbxproj`) and
Android (`build.gradle.kts` with `versionName`/`versionCode`) apps
during init. The questions that come up in user interviews:

- Should belaf manage the version-bump/tag-create/PR-create flow for
  mobile apps too?
- Should the manifest carry mobile-specific release metadata (build
  number, platform, signing info)?

## Decision

**No.** Mobile apps stay out of scope as managed release units. The
detector recognises them and the wizard auto-adds them to
`[allow_uncovered]` so the drift detector stays silent. The user
keeps managing mobile releases through their existing tooling
(fastlane, Bitrise, Xcode Cloud, Play Store releases).

The `single_mobile_repo` shape (a repo whose only project is a mobile
app) gets a wrong-tool warning from `SingleMobileStep` in the wizard —
"belaf is for managing semantic releases of code packages; mobile
apps are managed differently. Recommend: skip belaf for this repo."

## Consequences

- No `mobile_app` ecosystem loader. No `MobileAppRewriter`. No
  `MobileVersionFieldSpec`.
- Wizard's `UnifiedSelectionStep` renders mobile entries in a
  read-only "Externally-managed" category with a `—` indicator.
- `MobileApp { platform }` stays as a `DetectorKind` variant for the
  warning + auto-allow-uncovered path; never a release-emit path.
- Re-evaluating this is a 3.x discussion; reopening doesn't change
  3.0's contract.

## Why not?

- Mobile release flows are platform-locked and rarely look like
  "tag a commit + cargo-dist publishes". App Store / Play Store
  upload is a serialised, signed, store-side review — orthogonal to
  the manifest model.
- The signing material + secrets needed are not wire-format-shaped
  things; they live in CI runners or platform-specific keychains.
- A "mobile app" in this context is really 3-5 deliverables (iOS app,
  Android app, fastlane lanes, store listings, the mobile SDK that
  wraps shared business logic). The mobile *SDK* is in scope (it's a
  Kotlin/Swift library, picked up by `JvmLibrary` / `Swift` loaders);
  the app shell isn't.

## Alternatives considered

- "Mobile-aware" tag-format support: emit a tag like `ios-v3.4.5`
  alongside the regular release tag. Rejected: that's a tag-format
  override question, not a full ecosystem; users who want this can
  add a `[[release_unit]]` with `external_versioner` and write their
  own tag format.
- Optional `mobile` plugin: ships a separate crate that adds the
  ecosystem if installed. Rejected: unclear demand, plugin system
  itself is out of scope (per the 3.0 plan §16).
