# Changelog

All notable changes to belaf are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
