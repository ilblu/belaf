# Changelog

All notable changes to belaf are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
