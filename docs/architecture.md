# belaf architecture

belaf is two cooperating processes:

| Process | Lives at | Job |
|---------|----------|-----|
| **CLI** (this repo) | the developer's laptop / CI runner | analyse, generate manifest, open PR |
| **GitHub App** ([github-app](https://github.com/ilblu/belaf-github-app)) | hosted (api.belaf.dev) | parse manifest on PR merge, tag + release |

The contract between them is a JSON manifest written to
`belaf/releases/<uuid>.json` in the user's PR. Schema 3.0
([ADR 0003](adr/0003-schema-3-0-promotion.md)) is the wire format.

## The two primitives

belaf 3.0 has exactly two declarative primitives:

| Primitive | Section in `config.toml` | What it represents |
|-----------|--------------------------|--------------------|
| **`ReleaseUnit`** | `[[release_unit]]` | One thing with one version. Carries one or more manifests, optional satellites, optional cascade. |
| **`Group`** | `[[group]]` | Two or more Release Units that release together as a single atomic group (one tag, one GitHub Release). |

That's it. Everything else (`bundle_manifests`, `external_versioner`,
`cascade_from`, `visibility`, `satellites`) lives **on** a
`ReleaseUnit`. There is no "project tier" above units —
[ADR 0001](adr/0001-release-unit-as-sole-primitive.md) records the
decision to retire the old `Project` distinction.

## Pipeline (`belaf prepare`)

```
                    ┌───────────────────┐
 Working tree ───▶  │  Ecosystem loaders │  ── one per language
                    └─────────┬─────────┘
                              ▼
                    ┌───────────────────┐
                    │  ReleaseUnitGraph │  ── petgraph DAG + GroupSet
                    └─────────┬─────────┘
                              ▼
                    ┌───────────────────┐
                    │   Resolver        │  ── ResolvedReleaseUnit
                    └─────────┬─────────┘
                              ▼
                    ┌───────────────────┐
                    │  Drift detector   │  ── covers every detected path?
                    └─────────┬─────────┘
                              ▼
                    ┌───────────────────┐
                    │  Bump inference   │  ── conventional-commits
                    └─────────┬─────────┘
                              ▼
                    ┌───────────────────┐
                    │  Cascade pass     │  ── cascade_from edges
                    └─────────┬─────────┘
                              ▼
                    ┌───────────────────┐
                    │  Changelog gen    │  ── Tera templates
                    └─────────┬─────────┘
                              ▼
                    ┌───────────────────┐
                    │  Rewriters        │  ── one per ecosystem
                    └─────────┬─────────┘
                              ▼
                    ┌───────────────────┐
                    │  Manifest emit    │  ── belaf/releases/*.json
                    └─────────┬─────────┘
                              ▼
                              PR
```

Source files (in this repo):

| Stage | File |
|-------|------|
| Ecosystem loaders | `src/core/ecosystem/{cargo,npm,pypa,go,maven,swift,csproj,elixir}.rs` |
| Graph | `src/core/graph.rs` |
| Resolver | `src/core/release_unit/resolver.rs` |
| Detectors + drift | `src/core/release_unit/detector.rs` + `detector/{scanners,walk}.rs` |
| Bump inference | `src/core/bump.rs` |
| Cascade | `src/core/release_unit/cascade.rs` |
| Changelog | `src/core/changelog/` |
| Pipeline orchestrator | `src/core/workflow.rs` |
| Wire types | `src/core/wire/{codegen,domain,known}.rs` |

## Where Schema 3.0 fields live

The six promoted fields are typed both on the config side
(`[[release_unit]]`) and on the wire side (`releases[]` in the
manifest). Wire-side definitions:

| Field | Source | Where it shows on the dashboard |
|-------|--------|---------------------------------|
| `bundle_manifests: string[]` | unit's `source.manifests` when ≥2 | `<BundleBadge>` + `<ManifestFileList>` |
| `external_versioner` | unit's `source.external_versioner` | `<ExternalVersionerBadge>` |
| `version_field_spec` | rewriter pick — `cargo_toml`, `npm_package_json`, … | inline label on `<ManifestFileList>` |
| `cascade_from` | unit's `cascade_from` | `<CascadeArrow>` + Cascade Graph tab |
| `visibility` | unit's `visibility` (`public` / `internal`) | inline badge on `<ReleaseUnitCard>` |
| `satellites` | unit's `satellites` | `<SatelliteList>` |

## Drift

The drift detector runs on every `belaf prepare` (and on every wizard
launch, to seed the unified-selection list). It walks the same
heuristics as the init detectors and asks "is every hit covered by a
ReleaseUnit, an `[ignore_paths]` entry, or an `[allow_uncovered]`
entry?". Uncovered hits are a hard error.

`SingleProject` and `NestedMonorepo` matches are wizard-only — they
describe the repo shape rather than a missed bundle, so they never
escalate to drift errors.

See `src/core/release_unit/detector.rs::is_drift_signal` for the
discriminator.

## Why split CLI vs. App?

- **The CLI never needs registry credentials.** Publishing happens on
  the App side under the workspace owner's GitHub identity, so a leaked
  developer machine doesn't leak npm tokens.
- **Atomic groups need a single coordinator.** Two CI jobs both
  pushing tags at the same time race; the App serialises them.
- **Permissions are simpler.** The App's GitHub App identity has
  precisely the permissions it needs; the CLI runs as the developer
  and only opens a PR.

## Cross-repo development

When you change the manifest schema:

1. Edit `belaf/schemas/manifest.v1.schema.json`.
2. `cargo build` here regenerates the Rust wire types via `typify`
   (see `belaf/build.rs`).
3. Vendor the schema into github-app:
   `cp belaf/schemas/manifest.v1.schema.json github-app/api-spec/manifest.v1.schema.json`.
4. In github-app:
   `bun run packages/shared/scripts/generate-manifest-zod.ts` →
   regenerates `manifest-v1-schema.gen.ts`.
5. Commit both sides together.

Drift between producer and consumer surfaces as a Rust compile error
(typify) or a Zod parse error (github-app webhook handler). The
schema is intentionally strict (`additionalProperties: false`) so
unknown keys don't get silently dropped — there is an explicit `x`
field for forward-compatible vendor extensions.

## ADRs

The architectural decisions live in [`adr/`](adr/):

- [`0001`](adr/0001-release-unit-as-sole-primitive.md) — `ReleaseUnit` as the only primitive.
- [`0002`](adr/0002-bootstrap-toml-retirement.md) — `bootstrap.toml` retirement.
- [`0003`](adr/0003-schema-3-0-promotion.md) — Schema 3.0 promotion.
- [`0004`](adr/0004-mobile-apps-out-of-scope.md) — Mobile apps out of scope.
- [`0005`](adr/0005-drop-project-tenancy-tier.md) — Dropping the projects tenancy tier in github-app.
