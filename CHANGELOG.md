# Changelog

All notable changes to belaf are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## 4.0.0 — 2026-08-07

Configured release units were not in the dependency graph.

A `[release_unit.<name>]` block — explicit or glob-form — was registered as a
graph node and nothing else. The only two places that draw dependency edges ran
for `[cascade_inputs]` and for **auto-discovered** units, so in a repo where
both sides of a dependency are configured there was no edge between them at
all. A fix in a shared library reached no service, and `prepare` said
`nothing_to_do` — no error, because there was no unresolved dependency to
complain about. Nothing was missing; nothing had ever been drawn.

Everything below follows from that one hole, and from the three other things
that were quietly compensating for it.

### Fixed

- **Configured units get their dependency edges.** Discovery now receives a
  map of the paths each `[release_unit.X]` covers, and routes the edges of any
  package inside them onto the owning unit. A hexagonal service spanning a bin
  crate plus a tree of satellites collects the dependencies of all of them;
  edges between its own crates collapse to nothing. Edges bind to a concrete
  unit id rather than a name, so a cargo crate can no longer bind to a
  same-named npm package, and an ambiguous name is an error instead of a
  first-match guess.
- **`satellites` attribute their commits.** The key is documented as "paths
  whose commits attribute to this unit" and reached the ownership map and the
  drift detector, but never the unit's path matcher. For a hexagonal service —
  where the released `crates/bin` is a thin wrapper and the work lands in
  `crates/api`, `crates/core`, `crates/infrastructure` — that means essentially
  every real change attributed to no unit at all.
- **A configured unit is no longer also auto-discovered.** Workspace protocols
  enumerate every member from one call on the workspace root, which the
  path-level skip-list could not filter. Both nodes landed on the same
  directory, and when the unit name equalled the package name the two were
  indistinguishable: `multiple projects with same name`.
- **`Cargo.lock` is refreshed and committed.** Configured units write their
  versions through a different rewriter than auto-discovered ones, and that
  one never went near the lockfile — in a repo where every cargo unit is
  configured, the lock was never updated at all. The sync now runs once per
  affected workspace after the rewrites, from what actually changed, and the
  lockfile is added to the release commit. A release commit carrying bumped
  manifests against a stale lock is not untidy, it is terminal: the next
  checkout regenerates the lock, and `prepare` refuses to run in a dirty tree.
- **A failed lockfile update is an error.** It used to be a `warn!` nobody
  reads, which is how the above went unnoticed for days.
- **The Scoop bucket updates itself.** `update-scoop.yml` listened on
  `release: published`, which never fires for a release created with the
  workflow's own `GITHUB_TOKEN` — GitHub's recursion guard. Two releases in a
  row shipped a stale manifest and were fixed by hand. It is now a job in
  `release.yml`, on the same tag push that already works.

### Changed

- **A cargo workspace counts as one project only if every member actually
  inherits its version.** The test was the mere presence of
  `[workspace.package].version`, which is version *inheritance being offered*,
  not taken — practically every modern workspace sets it. That collapsed
  ordinary multi-crate repos into a single unit owning no `[package]`, named
  after the repo directory, which nothing releases and which trips the
  untagged-unit guard.
- **A gitignored `Cargo.lock` is left alone** — not refreshed, not committed.

### Migration

Both changes below surface config that was only ever describing a bug.

- If you added a `[release_unit.<name>]` block to suppress a unit named after
  your repo directory, **delete it**. That unit no longer exists, and a
  partial-override block matching nothing is a hard error.
- Expect **more units to bump**. Cascades that silently did nothing now work,
  including `[cascade_inputs]` reaching units transitively through a configured
  library. Run `belaf status --ci` before the first `prepare` to see the new
  set. If a unit bumps that you never intended to release, declare it:
  `kind = "ignore"` keeps it out of the release set entirely, `kind =
  "internal"` keeps it as a graph node that cascades but is never tagged.

## 3.1.0 — 2026-08-07

Answering "this unit has no release tag" per unit instead of repo-wide.

The guard that refuses to analyze a deploy unit with no matching tag is right —
analyzing from repo start inflates the bump, usually straight to a major. But
the only way to answer it was a `belaf-baseline` git tag, which silences the
guard for **every** unit at once, present and future, and lives somewhere no
review ever looks. And because the run aborted on the first offender, finding
out which units were actually affected meant re-running once per unit.

### Added

- **`baseline` on `[release_unit.<name>]`** — `"first-release"` to analyze from
  repo start (deliberately, for a unit that genuinely never shipped), or a
  commit-ish to start the window there. A real release tag always wins, so the
  key becomes a no-op after the first release. It scopes to one unit; every
  other unit keeps the guard.
- **`belaf baseline`** — lists every deploy unit still missing a tag or a
  `baseline` (JSON under `--ci`, exit 4 while anything is unanswered so it works
  as a CI gate). `--fix` writes `baseline = "first-release"` for each,
  format- and comment-preserving, validating the result before touching the
  file, and is idempotent.

### Changed

- **All untagged units are reported in one error**, with the unit name and the
  tag template it tried, instead of aborting on the first.
- **`affects` accepts a glob-form `[release_unit.<key>]`** instead of restating
  every unit the glob covers — a list that drifts the moment a service is added.
  Resolution is unit name, then glob key. A name that is neither is now a hard
  error rather than a warning: a typo used to silently release nothing.
  Ambiguity (a unit and a glob key of the same name) is also an error.
- **`prepare`'s pre-flight tag fetch authenticates**, using the same short-lived
  installation token the release push uses. It ran unauthenticated, so on a
  private repo over HTTPS the run died before doing anything. It is also
  fail-soft now: if the refresh fails while the clone already has version tags,
  it warns and continues on those. Only a clone with no tags at all still errors.

### Fixed

- **A prerelease unit with no stable tag no longer aborts the whole run.** A
  package kept permanently in beta never gets a stable tag, so one such unit
  took every other unit's release down with it. Its last prerelease tag now
  bounds the commit window, and the version follows release-please's rule: the
  base holds while the components below the bump level are zero
  (`0.6.0-beta.3` + fix → `0.6.0-beta.4`), and a breaking change still breaks
  out (→ `1.0.0-beta.1`). Previously the base crept forward every run and the
  counter reset each time.
- **An explicitly declared path is no longer claimed — or warned about — by a
  glob that also matches it.** `paths = [...]` units, the only shape
  `kind = "ignore"` can take, contributed nothing to the covered set, so they
  shadowed nothing at all; the warning also fired before the shadow check ran.
- **Release commits no longer need a configured git identity.** `create_commit`
  bypassed the fallback that already existed, so `prepare` failed at the commit
  on a fresh CI runner.

### Documentation

- The `all-deploy-units` throttling advice recommended a shared `concurrency`
  group. That was wrong and dangerous: a group holds exactly one pending run,
  so 23 simultaneous release runs would have left one running, one pending and
  **21 silently cancelled**. Replaced with `max-parallel`, plus an explicit
  warning about `queue: max`.
- `examples/github-actions/belaf-prepare.yml` sets a git identity and pins the
  installer to a version instead of `latest`.

## 3.0.0 — 2026-08-06

Two things this release exists for: `prepare` can finally run on every push
to main, and files that feed a release unit without being one can finally
trigger it.

### Idempotent `prepare`

Re-running it now refreshes the open release pull request instead of opening
another one, which is what makes `belaf prepare --ci` safe to wire into a
push-to-main workflow: N pushes produce one release PR, not N. This is the
pattern release-please and changesets both use, and until now belaf could
not be used that way at all.

### Declared path inputs

`[cascade_inputs]`. Some files feed a release unit
without being one: a shared OCI base image, protobuf schemas compiled by a
`build.rs`, a shared config tree. No package manager can see those edges.
Patch a CVE in a shared base image and, until now, nothing bumped — no tag,
no rebuild — while the fixed definition sat in the repo and the unpatched
images stayed in production.

`[codegen_edges]` was the partial answer to this and is replaced. It only
ever worked for directory globs: every entry was turned into a path prefix,
so `apko/*.lock` became the prefix `apko/*.lock/`, which matches nothing —
a silent no-op with no warning. It also had no bump control and left no
trace in the manifest, so nothing explained *why* twelve services bumped.

### Changed

- **BREAKING — the release branch is stable.** `prepare` pushed to a fresh
  `release/<timestamp>-<uuid>` branch on every run; it now reuses one branch
  per base branch, `belaf/release--{base}` by default, and force-updates it.
  Override with `[repo] release_branch`. `{base}` expands to the branch the
  run started from, with `/` replaced by `-`, and the result is validated
  with `git check-ref-format`.

  The base branch is part of the name on purpose: releasing from `main` and
  from a maintenance branch are separate release trains, and one shared
  branch name would let each clobber the other's PR.

  **Migration:** anything matching on `release/*` — branch protection rules,
  CI path filters, automation — needs to match `belaf/release--*` instead.
  If branch protection forbids force-pushes on that prefix, point
  `[repo] release_branch` somewhere unprotected.
- **`prepare` updates an existing release PR.** It looks for the open PR
  whose head is the release branch and refreshes its title and body; only
  when there is none does it open one. If GitHub reports that a PR already
  exists (a concurrent run opened one between the lookup and the create),
  the run finds and updates it rather than failing after it has already
  pushed. The interactive wizard asks whether to update the open PR or open
  a separate one on a throwaway branch; `--ci` always updates.
- **`prepare --ci` refuses to start from a release branch.** A run that
  fails after creating its branch leaves you on it; starting again from
  there used to name the next branch after the release branch and base the
  PR on it. It now stops with an explanation instead.
- **BREAKING — `[codegen_edges]` is removed, replaced by
  `[cascade_inputs.<name>]`.** A config that still has the old section fails
  to load with a message that renders the replacement block ready to paste.
  There is no silent migration: the semantics changed enough to be worth one
  look.

  ```toml
  [cascade_inputs.apko-base]
  paths   = ["apko/base.yaml", "apko/*.lock", "apko/overlays/**"]
  affects = "all-deploy-units"        # or ["gate", "rig", "kin"]
  bump    = "floor_minor"             # optional
  ```

- **BREAKING — the input's name is the table key.** It used to be derived
  from the glob's last path segment. The name is user-visible in three
  places — it is a valid commit scope for `belaf check`, it appears in
  affected units' changelogs as `via <name>`, and it is what the manifest
  records — so the accepted scope set shifts with it. The rendered migration
  block keeps the old names, so existing scopes stay valid if you paste it
  as-is.
- **Path entries are classified, not blanket-prefixed.** An entry containing
  `*`, `?` or `[` is matched as a glob; anything else is a literal path
  prefix. That is what makes `apko/*.lock` work.
- **An input that matches nothing is a hard error.** Empty `paths`, empty
  `affects`, an unknown `affects` shorthand, or an unknown `bump` strategy
  all fail at config load. A path that `[binary_affecting]` would filter out
  entirely is warned about.

### Added

- **`[repo] release_branch`** — the release branch template. See
  `docs/configuration.md`.
- **`pr_action` and `release_branch` in the `--ci` JSON status.**
  `pr_action` is `created` or `updated`, so a workflow can tell a new
  release PR from a refreshed one.
- **Push-to-main in the workflow template.**
  `examples/github-actions/belaf-prepare.yml` now triggers on pushes to
  `main` with a concurrency group, which is the setup this release exists
  to enable.
- **`bump` floor per input** — `mirror` (default), `floor_patch`,
  `floor_minor`, `floor_major`, the same vocabulary as `cascade_from`. Note
  that a collected commit is already floored to at least a patch, so the
  setting only changes anything from `floor_minor` up. An input can only
  raise a bump, never create one: a unit with no commits is skipped before
  the floor applies, and a per-unit `max_bump` still caps the result.
- **`affects = "all-deploy-units"`** — every deploy unit, without listing
  them. See the blast-radius warning in `docs/configuration.md`: the first
  real CVE patch fans out to every deploy unit at once, and a per-tag
  `concurrency` group does not throttle that.
- **`cascade_inputs` in the release manifest.** Each affected unit's release
  entry records the inputs that pulled it in and the bump each declared, so
  the dashboard can show why a unit bumped. Additive — `SCHEMA_VERSION`
  stays `"1"`.
- **`[cascade_inputs]` documentation.** `docs/configuration.md` has a
  section for it (`[codegen_edges]` never had one), and `belaf init
  --auto-detect` emits a stub for it.

### Fixed

- **Pull request creation in OIDC-only CI runs.** `prepare` loaded the API
  token without the GitHub Actions OIDC fallback that the push path uses,
  so on a runner with an empty keyring the release branch pushed and then
  the pull request failed to authenticate. Both paths now exchange the OIDC
  token the same way.

## 2.1.0 — 2026-07-18

Config-aware auto-detection. `belaf init` now respects every decision
already recorded in `belaf/config.toml`, which makes re-running
auto-detect safe and turns the drift-error remediation
(`belaf init --ci --auto-detect --force`) into a real, per-path
idempotent re-detect.

### Added

- **Real `--force` re-detect.** On an already-initialized config,
  `belaf init --ci --auto-detect --force` now appends exactly the
  detector hits the config does not cover yet — nothing more. Coverage
  mirrors the `prepare`-time drift check precisely (same shared
  helpers, including glob-form unit expansion and partial-override
  resolution): whatever drift is silent about, re-detect will not
  re-emit. Without `--force`, an initialized config stays untouched and
  the run reports how many uncovered candidates are waiting.
- **`[allow_uncovered]` merging.** Newly detected externally-managed
  paths (e.g. a mobile app added after the first init) are merged into
  an existing `[allow_uncovered]` table via `toml_edit` — format- and
  comment-preserving — instead of appending a duplicate table header,
  which would have been a TOML parse error.
- **Validated config writes.** Every auto-detect write (CI and wizard,
  including the wizard's tag-format override) parses the merged result
  before writing. Residual collisions — say, a re-emitted unit name
  that already exists as a table — abort with a diagnostic and leave
  `config.toml` untouched.
- **`--ci` JSON status fields.** `already_covered` (detector hits
  suppressed because the config already handles them) and `advice`
  (config-free detector hints, see below).

### Changed

- **`[ignore_paths]` suppresses detection at scan level.** Its
  documented contract ("belaf does not scan inside at all") now also
  holds for `init` auto-detect, not just the `prepare` drift check:
  hits under ignored paths are dropped before any emission, counting,
  or wizard display.
- **`[allow_uncovered]` and configured `[release_unit]` blocks suppress
  emission.** Paths a human already classified — externally managed, or
  claimed by an existing unit (explicit, glob-expanded, or partial
  override) — are never re-emitted; they surface in the summary as
  `already_covered` instead.
- **Detector hints are advice, not config.** The sdk-cascade
  suggestion, npm-workspace, nested-submodule, and single-project hints
  are rendered as config comments exactly once (first init). Re-runs
  surface them via the log and the `--ci` JSON instead of accumulating
  duplicate comment blocks in `config.toml`.
- **Wizard re-runs respect prior decisions.** Paths that are ignored,
  allow-uncovered, or covered by a configured unit no longer reappear
  as selectable detection rows.

### Fixed

- **Wizard re-run no longer silently drops confirmed choices.** The
  auto-detect marker gate used to discard the entire wizard output
  (including freshly confirmed `cascade_from` rules) when the config
  had been initialized before. Writes are now coverage-filtered and
  validated instead of marker-gated.
- **Tag-format override can no longer corrupt the config.** Re-running
  the wizard and picking a tag format for a project that already has a
  `[projects.<name>]` block used to append a duplicate table (a TOML
  parse error on the next load); it now aborts cleanly without
  writing.
- **Drift-error remediation text was wrong.** The `prepare` drift error
  recommended `belaf init --ci --auto-detect --force`, which silently
  did nothing on an initialized config. The command now works as
  advertised, and the message describes the per-path idempotency
  correctly.

## 2.0.0 — 2026-06-23

Dependency-closure-aware release model (F1–F11).

### Added

- F1 unit kinds (`deploy`/`internal`/`ignore`) + manifest-less
  `paths = [...]` units.
- F2/F5 dependency-closure cascade + patch-floor on binary-affecting
  changes.
- F3 configurable `[binary_affecting]` path filter.
- F4 `[codegen_edges]` synthetic cascade nodes + Tier-3 glob ownership.
- F6 path-based WHETHER — scope decoupled from the bump decision.
- F7 "via `<crate>`" changelog provenance.
- F8 `belaf check` commit-label validation.
- F9 glob soft-skip; F10 wired `[commit_attribution]` scope matcher.
- F11 per-unit `[release_unit.<name>.bump]` overrides + prerelease
  generation.

### Changed

- **BREAKING:** bump decisions are now path-based — a binary-affecting
  change floors to patch even for `chore`/`refactor` commits, and a
  commit's scope no longer decides whether a unit bumps (path
  attribution only).

## 1.3.2 — 2026-05-12

### Fixed

- **`belaf prepare` fetches upstream tags before reading its
  baseline.** Tag lookup only read local refs; when a prior release was
  tagged server-side (by the belaf GitHub App), a developer's stale
  local clone picked an old baseline and inflated the bump suggestion.
  Both prepare paths (CI and wizard) now fetch tags from the resolved
  upstream first; opt out with `BELAF_NO_FETCH=1`.

## 1.3.1 — 2026-05-12

Hotfix release. Two correctness bugs that inflated semver bumps for
multi-ecosystem repos.

### Fixed

- **Tag-format lookup is now ecosystem-aware.** `find_latest_tag_for_project`
  used to hardcode `{name}-v{version}` (cargo) plus a bare-`v{version}`
  fallback for single-project repos. Every other ecosystem's default tag
  template — npm `{name}@v{version}`, maven `{groupId}/{artifactId}@v{version}`,
  pypa `{name}-{version}` (no `v`), go `{module}/v{version}` — silently
  failed to match, so the lookup fell back to "analyze every commit since
  repo start". That swept in old `feat:`s from previous releases and
  inflated the recommended bump (the bug that took `@clikd/landing` from
  v0.7.0 → v0.8.0 on a single `fix:` commit; expected v0.7.1).
  The new `core::tag_format::TagMatcher` compiles the project's effective
  `tag_format` (per-`[release_unit]` > per-`[group]` > ecosystem default)
  into a regex with a `version` capture group — symmetric to how belaf
  *writes* tags. All ecosystems with non-cargo default templates now find
  their previous tags correctly.
- **Hard-fail when tag-lookup misses but the repo has version tags.**
  `analyze_histories` previously logged a `warn!` and walked the full
  history when no tag matched. That was the bug amplifier. New behaviour:
  if the repo has any version-shaped tags (`\d+\.\d+\.\d+` anywhere in
  the tag name) but none matched the project's template, bail with a
  diagnostic pointing at the likely `tag_format` mismatch. Truly-new
  repos with zero version tags still fall through with a warning.
- **`revert:` and `Revert "..."` now trigger a patch bump.**
  `analyze_commits` ignored Conventional-Commit `revert:` (treated as
  `other` → no bump) and git's auto-generated `Revert "<subject>"` (failed
  to parse → no bump). Both shapes now drive a patch bump; `revert!:`
  and `BREAKING CHANGE:` footers still lift to major. The TUI commit
  summary and `default.toml` changelog template gain a "Reverts" group
  (⏪).

### Internal

- `core::git::repository::find_latest_tag_for_project` signature changed
  from `(project_name: &str, is_single_project: bool)` to
  `(matcher: &TagMatcher)` and now returns the parsed version alongside
  the OID/tag name. `Repository::analyze_histories` and
  `find_earliest_release_containing` take the matcher slice / one matcher.
  No effect on the binary surface or the manifest wire format.

## 1.3.0 — 2026-05-08

Two themes: agent-friendly CLI surface (so AI assistants can drive
belaf without parsing 12 `--help` outputs) and config UX (partial
`[release_unit]` overrides + first-class pyproject support). No wire
format changes — `manifest.v1.schema.json` and `/api/cli/*` are
untouched, so this is drop-in for existing repos.

### Added

- **`belaf describe --json`** — single command that dumps the full CLI
  surface (every command + arg, env vars, exit codes, embedded
  schemas, example workflows). Walks the live `clap::Command` tree so
  it can never drift from what the binary accepts. `--text` produces a
  human-readable summary; `--json` is the default. Designed for AI
  agents that landed in a repo with `belaf` on `$PATH` and have no
  other context.
- **`belaf schema <name>`** — print an embedded JSON Schema by name
  (currently `manifest`). Lets agents validate manifests they parse
  without round-tripping to the dashboard or vendoring the schema.
- **`belaf doctor`** / **`belaf doctor --json`** — environment
  diagnostic. Checks auth (keyring + API verify), config validity,
  repository state, ecosystem auto-detect, and runs a real HTTP probe
  against `<api_url>/health` (3s timeout, reports `latency_ms`). Each
  check has a `status` field (`ok` / `warn` / `error` / `skipped`)
  plus an overall `ok` boolean and a `precondition` exit code (4)
  when not ready.
- **Stable exit-code contract** in `core::exit_code::ExitCode` (8
  codes: `0` ok, `1` generic, `2` usage, `3` nothing-to-do, `4`
  precondition, `5` conflict, `6` network, `7` config-invalid).
  Documented in `belaf describe --json` so agents can branch on them.
- **Partial-override `[release_unit.<name>]` blocks.** Omit
  `ecosystem` to inherit it (and `manifests`/`source`) from the
  auto-detected unit with the same name. Only override fields are
  permitted in this form (`tag_format`, `visibility`, `satellites`,
  `cascade_from`); structural fields raise
  `partial_override_structural_field`. Empty blocks raise
  `partial_override_empty`. Names that don't match an auto-detected
  unit raise `partial_override_no_match`. Replacive merge for lists
  (matches Kubernetes / Vite / Tauri / Biome convention).
- **`version_field = "pep_621"`** — first-class reader/writer for
  `pyproject.toml` `[project].version` using `toml_edit`. Drops the
  `generic_regex` workaround and preserves comments + ordering on
  write. Auto-selected for explicit `pypa` blocks (and reachable as
  the new ecosystem default).
- **`prepare --ci` final JSON status** on stdout: `{ status:
  "released" | "nothing_to_do" | "no_actionable_bumps", pr_url,
  release_units: [{name, bump}] }`. Decorative messages routed to
  stderr.
- **`init --ci` final JSON status** on stdout: `{ status:
  "initialized", config_path, release_units_detected, ecosystems }`.
- **`changelog --ci` final JSON status**: `{ status, mode:
  "disk"|"preview"|"stdout", projects, files_written }`. Routed to
  stderr when `--stdout` is also set so the changelog content stays
  uncontaminated on stdout.
- **Top-level `--help` agent hint** points first-time agents at
  `belaf describe --json` and notes the `--ci` / `--format=json`
  conventions.

### Changed

- `[release_unit.X].ecosystem` is now `Option<String>` in the TOML
  schema. Existing configs with `ecosystem = "..."` are unchanged;
  blocks without it become partial overrides.
- `belaf install` re-auth hint (emitted on `ApiError::Unauthorized`)
  now also points to `belaf doctor --json` for full-environment
  diagnosis.
- `pypa` ecosystem's default `version_field` is now `pep_621` (was
  `cargo_toml` fallback). Auto-detect path still uses the existing
  `PyProjectVersionRewriter`, so behavior is unchanged for
  auto-detected pypa projects.
- `belaf explain --format=json` gained a new `kind: "partial_override"`
  origin variant. Backward-additive — no consumer in github-app
  reads this surface.

### Wire format

No changes. `belaf/schemas/manifest.v1.schema.json` is unchanged,
`SCHEMA_VERSION` is unchanged, `api-spec/openapi.cli.json` is
unchanged. github-app does not need a coordinated update.

## 1.2.0 — 2026-05-08

Companion release to github-app `api@1.2.0`. Surfaces tier-limit
responses from the action edge as a structured CLI diagnostic instead
of a raw HTTP error string.

### Added

- `ApiError::LimitExceeded { tier, current, limit, upgrade_url }`
  variant. The HTTP client now recognises `402 Payment Required` with
  the `repository_limit_exceeded` code and parses the structured
  payload from `/api/cli/repos/.../pulls` and
  `/api/cli/repos/.../git/credentials`.
- Diagnostic renderer emits a `help: upgrade your plan: <url>` line
  when `LimitExceeded` is encountered, so a `belaf prepare` run that
  hits the limit displays an actionable message and direct upgrade
  link instead of the raw response body.

### Wire format

- `api-spec/openapi.cli.json` mirrors github-app `api@1.2.0`. The
  `ErrorResponse` envelope gained optional `code`, `tier`, `current`,
  `limit`, `upgrade_url` fields; old binaries ignore them via serde
  defaults.

## 1.0.0 — 2026-05-03

Initial stable release.

belaf is a Rust CLI that manages semantic releases across multi-language
monorepos. The release workflow is PR-based: the CLI never publishes
packages directly — it produces a release manifest in a PR, and a
separate GitHub App (`api.belaf.dev`) finalises the release on merge.

### Features

- **`belaf init`** — interactive TUI wizard that bootstraps
  `belaf/config.toml`. Auto-detects bundles (Tauri, hexagonal-cargo,
  JVM-library) and standalone units (Cargo, npm, PyPA, Go, Maven,
  Swift, Elixir, .NET via the `csharp` feature). Hint annotations
  decorate matching standalones; mobile apps land in
  `[allow_uncovered]`.
- **`belaf prepare`** — TUI + `--ci` mode to draft a release manifest:
  scans commits since each unit's last tag, infers conventional-commit
  bumps, generates per-unit changelogs via Tera templates, and writes
  `belaf/releases/<uuid>.json`.
- **`belaf graph`** — visualise the dependency DAG of release units.
- **`belaf explain`** — print the resolved release-unit topology
  (`--ci` emits JSON).
- **`belaf dashboard`** — no-arg entry TUI that dispatches to the
  other subcommands.

### Wire format

- Release manifest schema `v1` (`schemas/manifest.v1.schema.json`).
  Single-integer versioning (Kubernetes/Terraform-style); additive
  changes ship without a bump.
- Config syntax: named-entry tables only — `[release_unit.<name>]`
  (with optional `glob` field) and `[group.<id>]`. The legacy
  array-of-tables `[[release_unit]]` / `[[release_unit_glob]]` /
  `[[group]]` forms are not accepted.

### Distribution

- Pre-built binaries for `aarch64-apple-darwin`, `x86_64-apple-darwin`,
  `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`,
  `x86_64-pc-windows-msvc` via cargo-dist.
- Homebrew tap (`ilblu/homebrew-tap`) and Scoop bucket
  (`ilblu/scoop-bucket`).
