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
release_branch = "belaf/release--{base}"
```

| Key | Type | Default | Notes |
|-----|------|---------|-------|
| `upstream_urls` | array of strings | required | Used to compute compare URLs in changelogs and to detect "is this the canonical clone?" for the install flow. |
| `release_branch` | string | `belaf/release--{base}` | Template for the branch `belaf prepare` pushes its release commit to. `{base}` expands to the branch you ran from, with `/` replaced by `-`. The result is validated with `git check-ref-format`. |

### Why the release branch is stable

`prepare` reuses one branch per base branch and force-pushes it, then
**updates** the open release PR instead of opening a new one. That is what
makes `prepare --ci` safe to run on every push to your main branch: N runs
produce one PR, not N PRs.

`{base}` is in the default for a reason — releasing from `main` and from a
maintenance branch like `1.x` are separate release trains, and a single
global branch name would let one clobber the other's PR.

Each run rebuilds the branch from the base commit and pushes it in one
atomic force-push, so it never passes through a state where it equals the
base — GitHub auto-closes a PR whose branch is reset to its base, and that
would orphan the release PR.

After the PR merges, nothing needs cleaning up: the next run rebuilds the
branch from the new base and opens a fresh PR. It does not matter whether
GitHub's "automatically delete head branches" setting is on.

If your branch protection forbids force-pushes on `belaf/*`, point this key
somewhere unprotected.

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
| `satellites` | Repo-relative dirs that belong to this unit but carry no manifest of their own (e.g. `crates/foo/` for a hexagonal Cargo service). Commits touching them attribute to this unit, their package-manager dependencies count as this unit's dependencies, and drift detection counts them as covered. |
| `cascade_from` | `{ source = "schema-unit", bump = "floor_minor" }` — auto-bump this unit when `source` bumps. Strategies: `mirror`, `floor_patch`, `floor_minor`, `floor_major`. |
| `visibility` | `"public"` (publishes to a registry), `"internal"`, or `"hidden"`. Surfaced on the dashboard. |
| `tag_format` | Override the ecosystem default. See "Tag-format precedence" below. |
| `baseline` | Where this unit's history starts when no release tag matches it. See "`baseline`" below. |

### `baseline`

When belaf can't find a release tag for a `kind = "deploy"` unit **and**
the repo already carries version-shaped tags, `belaf prepare` refuses to
run. That refusal is correct: analyzing from repo start would count every
commit the unit ever had and inflate the bump — usually straight to a
major.

`baseline` is how you answer it, per unit:

```toml
# Genuinely never released. Analyze from repo start; you have looked at
# it and accepted the result.
[release_unit.docs]
baseline = "first-release"

# Released before belaf existed (or under tags belaf can't reconstruct).
# Start the commit window at this commit instead.
[release_unit.desktop]
baseline = "8eb3e3cf78ac6e"
```

Which one:

- **`"first-release"`** — the unit has no release history at all. Every
  commit that ever touched it belongs in the first changelog, so "analyze
  everything" is the right answer rather than a bug.
- **a commit-ish** — the unit *was* released, you just can't point belaf
  at the tag. Anything `git rev-parse` accepts works: full sha, short sha,
  a tag, a branch. The commit itself is excluded; the window starts after
  it.

There is a third case neither value covers: the tags **do** exist and
belaf's template simply doesn't match them (`v1.2.3` vs `docs-v1.2.3`).
Fix `tag_format` for that — a `baseline` would paper over a template bug
and silently mis-bump forever.

Semantics worth knowing:

- **A real release tag always wins.** `baseline` is only consulted on a
  tag-lookup miss, so the key becomes a no-op after the unit's first
  release and can be deleted then. Leaving it costs nothing.
- **It scopes to exactly one unit.** Every other unit keeps the guard.
- On a **glob-form** block it applies to every unit the glob expands to —
  blunt by construction, so prefer a per-unit block.

#### It replaces reaching for the `belaf-baseline` tag

The older escape hatch is a repo-wide `git tag belaf-baseline <commit>`.
Prefer `baseline` over it, for two reasons:

1. The tag silences the guard for **every** unit at once — including
   units added months later, which then get no protection at all and no
   one notices.
2. It lives in a git tag. Nobody reviews git tags. `baseline` shows up in
   a `belaf/config.toml` diff with the unit's name next to it.

`belaf-baseline` still works and still applies as a fallback *after* the
per-unit key, so existing repos are unaffected. Treat it as the
bootstrap-only tool it is: fine for the very first run of a brand-new
repo, wrong as a standing answer.

#### Finding the units that need one

```bash
belaf baseline          # list the units prepare would refuse on
belaf baseline --ci     # ...as JSON
belaf baseline --fix    # write baseline = "first-release" for each of them
```

`belaf prepare` reports **all** offending units in one error, so this is
a single pass rather than one unit per run. `--fix` preserves the
formatting and comments of `belaf/config.toml`, validates the merged
result before writing, and leaves any unit that already has a `baseline`
untouched.

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

## `[cascade_inputs.<name>]`

Some files feed a release unit without being one. A shared OCI base
image, protobuf schemas compiled by a `build.rs`, a shared config tree:
no package manager can see those edges, so belaf can't infer them. Patch
a CVE in the base image and — without a declaration — nothing bumps, no
tag is cut, and the unpatched images stay in production while the fixed
definition sits in the repo.

`[cascade_inputs]` declares those edges. A change under `paths` cascades
into every unit in `affects`, exactly as if that unit had changed.

```toml
[cascade_inputs.apko-base]
paths   = ["apko/base.yaml", "apko/*.lock", "apko/overlays/**"]
affects = "all-deploy-units"        # or ["gate", "rig", "kin"]
bump    = "floor_minor"             # optional
```

| Field | Meaning |
|-------|---------|
| `paths` | Repo-relative paths. An entry containing `*`, `?` or `[` is matched as a **glob** (`apko/*.lock`, `apko/overlays/**`); anything else is a literal **path prefix** (`apko/base.yaml`, `proto`). At least one entry is required. |
| `affects` | Either the shorthand string `"all-deploy-units"` or an explicit list of unit names. An empty list is a hard error — an input that affects nothing is the exact bug this feature exists to prevent. |
| `bump` | Optional bump floor forced on the affected units: `"mirror"` (default), `"floor_patch"`, `"floor_minor"`, `"floor_major"`. Same vocabulary as `cascade_from`. |

The table key (`apko-base`) is the input's **name**, and it is
user-visible in three places:

- it becomes a graph node, so `fix(apko-base): patch zlib` is a valid
  commit scope for `belaf check`;
- affected units list it in their changelog as `via apko-base — …`;
- affected units carry it in the release manifest's `cascade_inputs`
  array, so the dashboard can show *why* twelve services suddenly
  bumped.

The name must not collide with an existing release unit; belaf refuses
to build the graph if it does.

### The `bump` floor barely matters below `floor_minor`

Any commit belaf collects for a unit is already floored to at least a
**patch** (a binary-affecting file changed, so the artifact changed).
`mirror` and `floor_patch` therefore change nothing in practice — for
the CVE-patch case the default is already correct. The setting only
starts to matter at `floor_minor` and above, where you are deliberately
saying "a change to this input is a bigger deal than the commit type
suggests".

An input can only ever *raise* a bump, never create one. A unit with no
commits in its window is skipped before the floor is applied, so a
declared input never resurrects an untouched unit. A per-unit
`max_bump` still wins: the cap is applied after the floor.

### Operational warning: the blast radius of `all-deploy-units`

`all-deploy-units` is not an edge case — the first real CVE patch to a
shared base image is the *normal* case, and it fans out to every deploy
unit at once. With 23 deploy units that is 23 tags, 23
`release:published` events, and **23 release workflow runs starting
simultaneously**, each with a full OCI build against a cold cache.

A per-tag `concurrency` group in the consumer workflow does **not**
throttle this. Each tag is its own group, so every run is alone in its
group and none of them queue. Throttle it explicitly:

The safe way to cap it is at the job level, with a matrix:

```yaml
jobs:
  release:
    strategy:
      max-parallel: 4
      matrix:
        unit: ${{ fromJson(needs.plan.outputs.units) }}
```

Pick a cap your registry and build cache can actually absorb. If you'd
rather keep the radius small, list the units explicitly instead —
`affects = ["gate", "rig"]` — at the cost of having to remember to add
new services to the list.

> **Do not throttle release runs with a shared `concurrency` group unless
> you also set `queue: max`.** A concurrency group holds exactly *one*
> pending run by default: "any existing `pending` job or workflow in the
> same concurrency group will be canceled and the new queued job or
> workflow will take its place." `cancel-in-progress: false` protects the
> run that is *already executing* — it does nothing for the queue. Point
> 23 simultaneous release runs at one group and you get one running, one
> pending, and **21 silently cancelled**: 21 services tagged as released
> whose images were never built, with nothing failing to tell you.
>
> ```yaml
> concurrency:
>   group: release-runner
>   cancel-in-progress: false
>   queue: max          # up to 100 pending; without this, all but one are dropped
> ```
>
> Even with `queue: max` the cap is 100 pending runs, beyond which runs
> are cancelled again. For a fan-out this wide, prefer `max-parallel`.

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

### JavaScript workspaces: what the npm loader reads

Nothing here needs configuring; this is what the loader does on its own, and
it is worth knowing because the four package managers disagree.

| Manager | Members declared in | Notes |
|---|---|---|
| npm, yarn, bun | `workspaces` in the root `package.json` | Array, or the `{ "packages": [...] }` form. |
| pnpm | `pnpm-workspace.yaml` (`packages:`) | The root `package.json` usually has **no** `workspaces` key at all. |

Both are read and merged, so a repo mid-migration that carries both is fine.
`!`-prefixed patterns (`"!packages/fixtures/**"`) exclude a directory from the
workspace; a `package.json` there is still discovered as a package in its own
right, it just is not a member and gets no internal dependency edges.

Two dependency specs are **never** rewritten, whatever the release does to the
version:

- `workspace:*`, `workspace:^`, `workspace:~`, `workspace:1.2.3` — bun, pnpm
  and yarn. These say *resolve from this monorepo*; the manager substitutes a
  real version at publish time. Writing a number over one sends the install to
  the public registry, which for a `"private": true` package has nothing to
  give it.
- `catalog:` and `catalog:<name>` — bun and pnpm. The version lives once in
  the root catalog. Inlining it here opts the package out of the catalog,
  which is the one thing a catalog exists to prevent.

`link:`, `file:` and `portal:` are left alone for the same reason: they are
paths, not versions. Ordinary ranges (`^1.2.3`, `>=0.1.0`) stay belaf's to
manage.

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

```bash
# Which units have no release tag and no `baseline` — i.e. what would
# stop the next `belaf prepare`. Exits 4 while any are unanswered.
belaf baseline
```

### The schema of this file, from the binary

```bash
belaf schema config
```

Prints a JSON Schema for `belaf/config.toml`, generated from the same serde
types that parse it — so it cannot drift from what belaf actually accepts, and
`deny_unknown_fields` carries through as `additionalProperties: false`. Every
doc comment in this document's source lands there as a `description`.

This exists for the reader who has the installed binary and not this
repository: an agent working inside someone else's project can ask the tool
for the exact spelling of `[ignore_paths]` instead of guessing.
`belaf schema manifest` prints the other direction — the release manifest
belaf writes for the GitHub App to consume.

## Reference

- [`docs/getting-started.md`](getting-started.md) — fresh-install walk-through.
- [`docs/architecture.md`](architecture.md) — how it fits together.
- [`docs/adr/`](adr/) — why it's shaped this way.
