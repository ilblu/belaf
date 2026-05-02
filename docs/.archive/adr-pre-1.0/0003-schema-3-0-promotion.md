# ADR 0003 — Manifest Schema 3.0 promotion

- **Status**: Accepted (belaf 3.0)
- **Date**: 2026-05-01

## Context

Schema 2.0 (introduced in belaf 2.0) carried `releases[]` entries with
a fixed shape: name, ecosystem, group_id, previous/new version,
bump_type, tag_name, changelog, contributors, statistics, plus an
explicit `x: object` vendor-extension namespace.

Six pieces of metadata sat on the CLI config side (`[[release_unit]]`
in TOML) but never reached the wire — meaning the github-app dashboard
could not display them without re-fetching the user's config:

- `bundle_manifests` — list of manifest paths in a multi-manifest
  ReleaseUnit (the Tauri triplet, hexagonal cargo services).
- `external_versioner` — config for plugin-managed Gradle / scripts
  that compute version from git tags.
- `version_field_spec` — which file format encodes the version
  (cargo_toml, npm_package_json, tauri_conf_json, gradle_properties,
  generic_regex).
- `cascade_from` — when this unit was bumped because a schema source
  was bumped.
- `visibility` — public-publish vs internal-only.
- `satellites` — repo-relative directories that are part of the unit
  but contain no version-bearing manifest.

## Decision

Promote all six fields to first-class typed fields on each Release
entry in Schema 3.0. All optional; absent on the wire = unit doesn't
have that piece of metadata.

```jsonc
{
  "schema_version": "3.0",
  // ... unchanged top-level
  "releases": [{
    "name": "...", "ecosystem": "...", /* ...existing... */
    "bundle_manifests": ["packages/foo/package.json", "..."],
    "external_versioner": {
      "tool": "gradle",
      "read_command": "./gradlew currentVersion -q",
      "write_command": null,
      "cwd": null,
      "timeout_sec": null,
      "env": null
    },
    "version_field_spec": "cargo_toml",
    "cascade_from": { "source": "@org/schema", "bump": "minor" },
    "visibility": "public",
    "satellites": ["crates/foo/lib", "crates/foo/workers"]
  }]
}
```

`schema_version` is now the literal `"3.0"`. The github-app dispatcher
in `parseManifest` accepts only `"3.0"`; v2.0 manifests hard-error
with a clear message ("upgrade the producer CLI to 3.0+").

## Consequences

- belaf CLI emits 3.0 manifests; github-app v3.0 consumes 3.0 only.
  Big-bang cutover (no production data, per the user's audit).
- Dashboard can render `<BundleBadge>`, `<ExternalVersionerBadge>`,
  `<CascadeArrow>`, `<SatelliteList>`, `<TagFormatPreview>`, etc. from
  the wire payload alone. Wave 4 follow-up adds these visualizers.
- New typed fields are populated by the producer (CLI) when the
  underlying ReleaseUnit has them; absent fields = empty arrays /
  null on the wire.
- Forward-compatible: future fields land additively in 3.x without
  schema bumps. New ecosystems / bump types / visibility values stay
  free-form strings classified by hand-maintained `Known | Unknown`
  whitelists on the consumer side.

## Status of producer wiring

Wave 2 lands the wire shape (schema file + typify codegen + Zod regen
+ dispatcher swap). The `From<Release> for WireRelease` impl emits
`None`/`Vec::new()` for the new typed fields today — they're
populated from `ResolvedReleaseUnit` metadata as a focused follow-up.
This is documented inline at the impl site; the cleanup PR is bounded
because the data flow is `ResolvedReleaseUnit → wire::Release` only,
no schema change required.

## Alternatives considered

- Stay on 2.0 and use `x.bundle_manifests` etc. Rejected: the user's
  3.0 plan explicitly chose first-class promotion to make the wire
  format self-describing.
- Stripe-style coexistence (consumer accepts both 2.0 and 3.0).
  Rejected: no production data + the user explicitly chose big-bang
  per ADR 0005's audit.
