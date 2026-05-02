# ADR 0001 — ReleaseUnit as the sole primitive

- **Status**: Accepted (belaf 3.0)
- **Date**: 2026-05-01

## Context

Belaf 1.x and 2.x carried two parallel primitives for "the thing that
gets a version":

- `Project` — resolved-state struct in `core/project.rs` carrying the
  bumped version, a list of rewriters, the prefix in the repo, and
  `internal_deps`.
- `ReleaseUnit` — declarative config in `core/release_unit.rs`
  introduced in 2.0 to model multi-manifest bundles, satellites,
  external versioning, and cascade rules.

Both were "the project". The resolver pipeline turned each
`ReleaseUnit` into a `Project` and the rest of the codebase consumed
`Project`. Wizard code branched on whether to show the manual project
list (`ProjectSelectionStep`) or the auto-detected bundle list
(`DetectorReviewStep`). Two screens, two state vectors, two halves of
the same answer to the same question.

The symptom that triggered the cleanup: 2.1's `DetectorReviewStep`
shipped per-item exclusion, then we needed the same for the manual
`ProjectSelectionStep`, and the gap surfaced as user confusion in the
2.1.1 wizard.

## Decision

`ReleaseUnit` is the sole declarative primitive. The struct that
carries resolved state is renamed `ResolvedReleaseUnit` and treated as
resolver-internal (the public API exposes `ReleaseUnit`, the resolver
pipeline produces `ResolvedReleaseUnit` for the graph).

- `Project` → `ResolvedReleaseUnit`
- `ProjectId` → `ReleaseUnitId`
- `ProjectGraph` → `ReleaseUnitGraph`
- `ProjectGraphBuilder` → `ReleaseUnitGraphBuilder`
- `Group.members: Vec<ProjectId>` → `Vec<ReleaseUnitId>`
- `ProjectSelectionStep` + `DetectorReviewStep` → `UnifiedSelectionStep`

The wire-format manifest also gets typed promotions of fields that
used to live entirely on the `[[release_unit]]` config side
(`bundle_manifests`, `external_versioner`, `version_field_spec`,
`cascade_from`, `visibility`, `satellites`) — see ADR 0003.

## Consequences

- Single mental model: every versioned thing is a ReleaseUnit. Bundles,
  externally-versioned units, satellites — same primitive, different
  shape.
- Wizard collapses to one screen: `UnifiedSelectionStep` with three
  categories (Bundles / Standalone / Externally-managed).
- The resolved-state vs declarative-config distinction stays internal
  to the resolver pipeline — callers don't need to know.
- The github-app's wire format treats Schema 3.0 as the contract.
  Producers populate the new typed fields when the underlying unit
  has them; consumers (dashboard) render with the typed payload.

## Alternatives considered

- Keep the two-primitive split, just add a `unified_selection` UI on
  top. Rejected: the underlying split would still leak into config
  surfaces and into the resolver, and any new feature would have to
  decide which side it lives on.
- Drop `ReleaseUnit` and use `Project` everywhere. Rejected: 2.0's
  ReleaseUnit-only surfaces (multi-manifest, external versioning,
  cascade) are exactly what we want to keep; rolling back to a single
  Project struct would remove model power.

## Status of related items

- The maven.rs (1521 LOC) and detector.rs (1192 LOC) physical splits
  are deferred follow-ups documented inline at file head.
- Bootstrap.toml retirement is deferred — see ADR 0002.
