# Changelog

All notable changes to belaf are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
