# belaf configuration reference

belaf reads `belaf/config.toml` from the repo root. The wizard
(`belaf init`) writes a fully-formed config — you rarely write one
from scratch. This page is the reference for every section.

The schema is **strict**: unknown keys fail the parse. That's by
design: a typo'd `tag_formats =` (plural) silently doing nothing was
the worst class of 2.x bug.

## `[repo]`

```toml
[repo]
upstream_urls = ["https://github.com/your-org/your-repo.git"]
```

| Key | Type | Default | Notes |
|-----|------|---------|-------|
| `upstream_urls` | array of strings | required | Used to compute compare URLs in changelogs and to detect "is this the canonical clone?" for the install flow. |

### `[repo.analysis]`

| Key | Type | Default |
|-----|------|---------|
| `commit_cache_size` | int | `512` |
| `tree_cache_size` | int | `3` |

Tuning knobs for the libgit2 walker. Defaults are fine for repos up
to a few hundred thousand commits.

## `[changelog]`

```toml
[changelog]
conventional_commits = true
include_breaking_section = true
include_contributors = true
emoji_groups = true
output = "CHANGELOG.md"
```

| Key | Type | Default | Notes |
|-----|------|---------|-------|
| `conventional_commits` | bool | `true` | Parse `feat:` / `fix:` / etc. |
| `include_breaking_section` | bool | `true` | Emit a `### BREAKING CHANGES` block. |
| `include_contributors` | bool | `true` | List unique authors per release. |
| `emoji_groups` | bool | `true` | Prefix sections with emoji (`✨ Features`, …). |
| `output` | string | `CHANGELOG.md` | Path relative to the unit's prefix; written by the rewriter pass. |

## `[bump]`

```toml
[bump]
features_always_bump_minor = true
breaking_always_bump_major = true
initial_tag = "0.1.0"
```

The defaults match conventional-commits semantics. You can override
per Release Unit (see below).

## `[commit_attribution]`

How a commit gets routed to a Release Unit when no explicit scope
matches.

| Key | Values | Default |
|-----|--------|---------|
| `strategy` | `path_first` \| `scope_first` \| `path_only` \| `scope_only` | `scope_first` |
| `scope_matching` | `exact` \| `smart` | `smart` |

`smart` lowercases, strips ecosystem suffixes, and matches `feat(api)`
against units named `api`, `my-api`, `@org/api`, etc.

## `[release_unit.<name>]`

The core declarative primitive. Each unit is one releasable thing
with one version. The TOML key is the unit name; setting `glob = "..."`
switches the entry into glob-form, expanding into N units per
matching directory.

```toml
[release_unit.schema]
ecosystem = "npm"
manifests = [{ path = "packages/schema/package.json", version_field = "npm_package_json" }]
tag_format = "schema-v{version}"
```

### `manifests` vs `external`

```toml
# Single manifest (most common)
[release_unit.foo]
ecosystem = "cargo"
manifests = [{ path = "packages/foo/Cargo.toml", version_field = "cargo_toml" }]

# Bundle: one unit, several manifests kept lock-step
[release_unit.desktop]
ecosystem = "tauri"
manifests = [
  { path = "apps/desktop/package.json", version_field = "npm_package_json" },
  { path = "apps/desktop/src-tauri/Cargo.toml", version_field = "cargo_toml" },
  { path = "apps/desktop/src-tauri/tauri.conf.json", version_field = "tauri_conf_json" },
]

# Externally-managed (e.g. plugin-driven Gradle). `manifests` and
# `external` are mutually exclusive — exactly one must be set.
[release_unit.kotlin-sdk]
ecosystem = "external"
external = { tool = "gradle", read_command = "./gradlew -q :sdk:printVersion", write_command = "./gradlew :sdk:setVersion -PnewVersion={version}", cwd = "sdks/kotlin", timeout_sec = 60 }
```

### Optional fields

| Key | Notes |
|-----|-------|
| `satellites` | Repo-relative dirs that belong to this unit but carry no manifest of their own (e.g. `crates/foo/` for a hexagonal Cargo service). Drift detection counts these as covered. |
| `cascade_from` | `{ source = "schema-unit", bump = "floor_minor" }` — auto-bump this unit when `source` bumps. Strategies: `mirror`, `floor_patch`, `floor_minor`, `floor_major`. |
| `visibility` | `"public"` (publishes to a registry), `"internal"`, or `"hidden"`. Surfaced on the dashboard. |
| `tag_format` | Override the ecosystem default. See "Tag-format precedence" below. |

### Glob form

Convenience for "every package under `apps/services/*`":

```toml
[release_unit.services]
ecosystem = "cargo"
glob = "apps/services/*"
name = "{basename}"
manifests = ["{path}/crates/bin/Cargo.toml"]
fallback_manifests = ["{path}/crates/workers/Cargo.toml"]
satellites = ["{path}/crates"]
```

The resolver expands the glob into one Release Unit per matching
directory. Templates `{path}`, `{basename}`, `{parent}` substitute in
`name`, `manifests`, `fallback_manifests`, and `satellites`.

### Tag-format precedence

Highest wins:
1. `tag_format = "..."` on the `[release_unit.<name>]` block
2. `tag_format = "..."` on the unit's `[group.<id>]` block
3. ecosystem default (`{name}@v{version}` for npm, `{name}-v{version}`
   for cargo, `{groupId}/{artifactId}@v{version}` for maven, …)

## `[group.<id>]`

```toml
[group.schema]
members = ["schema-npm", "schema-jvm"]
tag_format = "schema-v{version}"
```

Bundles units that release together as a single atomic group. The
GitHub App tags every member at the same version on PR merge; if any
member's tag-write fails the whole group is rolled back.

## `[ignore_paths]` and `[allow_uncovered]`

```toml
[ignore_paths]
paths = ["vendor/", "third_party/"]

[allow_uncovered]
paths = ["apps/clients/ios/", "apps/clients/android/"]
```

| Section | Effect |
|---------|--------|
| `[ignore_paths]` | The resolver skips these paths (no Release Unit) **and** the drift detector stays silent. Use for vendored code or archives. |
| `[allow_uncovered]` | The resolver skips these paths but they're still acknowledged — the drift detector won't fire. Use for things released by another tool (mobile apps via Bitrise, etc.). |

The wizard auto-adds detected mobile apps to `[allow_uncovered]`.

## `[ecosystems]`

```toml
[ecosystems]
disable = ["go"]  # opt out of the Go loader entirely
```

Rarely needed — the loaders are cheap and idempotent.

## `[[bump_source]]`

Inject bump decisions from an external tool (e.g. release-please for a
specific package).

```toml
[[bump_source]]
name = "release-please-mirror"
command = "./scripts/release-please-decisions.sh"
```

The command must emit JSON of the form
`{ "decisions": [{ "release_unit": "<name>", "bump": "minor" }] }`.

## Inspecting the resolved config

```bash
# Human-readable
belaf config explain

# JSON for tooling (consumed by the dashboard's Explain tab)
belaf config explain --format json
```

`config explain` prints the full resolved view: every Release Unit,
its source, tag format, group membership, cascade edges, and the
ecosystem default that applied.

## Reference

- [`docs/getting-started.md`](getting-started.md) — fresh-install walk-through.
- [`docs/architecture.md`](architecture.md) — how it fits together.
- [`docs/adr/`](adr/) — why it's shaped this way.
