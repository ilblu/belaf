## [1.3.6](https://github.com/ilblu/belaf/compare/v1.3.5...v1.3.6) (2026-04-27)


### Bug Fixes

* **pypa:** detect PEP 621 `[project]` table in `pyproject.toml`. Previously belaf only looked at `[tool.belaf]` and the legacy `setup.cfg`/`setup.py` markers, which forced users on modern Python layouts (uv, hatch, poetry, plain setuptools >=61) to duplicate `name` and `version` in `[tool.belaf]` even though they were already in `[project]`. Now `[project] name` and `[project] version` are picked up natively; `[tool.belaf]` remains as an explicit override. ([commit](https://github.com/ilblu/belaf/commit/HEAD))
* **pypa:** add `PyProjectVersionRewriter` to write back `[project] version = "..."` in `pyproject.toml` for PEP 621-only projects (no setup.py, no setup.cfg). Fixes `failed to open file '...setup.py' for reading` during `belaf prepare` on modern Python projects. ([commit](https://github.com/ilblu/belaf/commit/HEAD))


### Tests

* **pypa:** integration tests for two new scenarios — PEP 621-only projects and `[tool.belaf] name` overriding `[project] name`. ([commit](https://github.com/ilblu/belaf/commit/HEAD))


### Build System

* **release:** bump cargo-dist 0.30.3 → 0.31.0 and harden the host job's `gh release upload` step. The previous releases (1.3.4, 1.3.5) needed a manual `gh release create` + `--failed` rerun because GitHub's release backend wasn't always queryable immediately after `dist host` reported success ([cli/cli#6599](https://github.com/cli/cli/issues/6599)). The step now (a) ensures the release exists with the dist-manifest body if missing and (b) retries `gh release upload` 5× with backoff. ([commit](https://github.com/ilblu/belaf/commit/HEAD))


## [1.3.5](https://github.com/ilblu/belaf/compare/v1.3.4...v1.3.5) (2026-04-27)


### Bug Fixes

* **errors:** error output now shows the full caused-by chain and surfaces actionable hints instead of dropping the real reason. Previously, running `belaf init` in a dirty repo printed only `Error: could not initialize app and project graph` while the actual cause (`refusing to proceed (use --force to override)`) was discarded. Migrated to [`annotate-snippets`](https://github.com/rust-lang/annotate-snippets-rs) (the same renderer rustc uses), so errors now look like:

  ```
  error: refusing to proceed
    |
  help: pass `--force` to override, or commit/stash your changes first
  ```

  Plus context-aware hints for typed errors (`ApiError::RateLimited` shows the retry duration; `ApiError::Unauthorized` suggests `belaf install`; `DirtyRepositoryError` suggests `--force`; `BareRepositoryError` explains the working-tree requirement). ([5d766db](https://github.com/ilblu/belaf/commit/5d766db))


### Code Refactoring

* **errors:** consolidate the user-facing error renderer in `core::errors::display_diagnostic`. Removed the parallel dead `errors::report` path and 9 redundant `atry!` wraps (`could not initialize app and project graph` etc.) where the inner error was already specific. Inner errors now speak directly. ([5d766db](https://github.com/ilblu/belaf/commit/5d766db))
* **deps:** add `annotate-snippets = "0.12"` for diagnostic rendering. Honors `NO_COLOR` env, `--no-color` flag, and stderr-is-a-TTY for color decisions. ([5d766db](https://github.com/ilblu/belaf/commit/5d766db))


### Tests

* **diagnostic:** add `tests/test_diagnostic.rs` with insta snapshot tests pinning the rendered output for 6 representative error cases (dirty repo, bare repo, rate-limit, unauthorized, annotated notes, plain error). Format changes are now reviewed via committed `.snap` files in PRs. ([5d766db](https://github.com/ilblu/belaf/commit/5d766db))


## [1.3.4](https://github.com/ilblu/belaf/compare/v1.3.3...v1.3.4) (2026-04-27)


### Bug Fixes

* **install:** `belaf install` no longer fails with "API did not provide installation URL" on uninstalled repos. Root cause: the API was emitting `installUrl` (camelCase) while the CLI deserialised `install_url` (snake_case); the missing field defaulted to `None`. The `/api/cli/*` contract is now schema-first with snake_case enforced at the type level on both sides. ([2ac07cc](https://github.com/ilblu/belaf/commit/2ac07cc))
* **api:** `belaf install` now receives `username` correctly (was emitted as `githubUsername` — a second silent drift in the same shape). ([2ac07cc](https://github.com/ilblu/belaf/commit/2ac07cc))


### Code Refactoring

* **api:** Replace hand-written wire types with `progenitor`-generated Rust types from a committed OpenAPI spec (`api-spec/openapi.cli.json`). Future drift between the belaf API and CLI surfaces as a Rust compile error instead of a silent `serde(default)` miss. ([2ac07cc](https://github.com/ilblu/belaf/commit/2ac07cc))
* **deps:** Drop unused `octocrab` (0% used in `src/`) and `core/root.rs` (duplicated `version_check.rs`). All GitHub data flows exclusively through the belaf API now. ([2ac07cc](https://github.com/ilblu/belaf/commit/2ac07cc))


### Security

* **deps:** Resolve all open `cargo audit` advisories. Bump `git2 0.20.2 → 0.20.4` (RUSTSEC-2026-0008), `ratatui 0.29 → 0.30` (drops indirect `lru 0.12.5`, RUSTSEC-2026-0002), `indicatif 0.17 → 0.18` (drops `number_prefix`, RUSTSEC-2025-0119). Transitive bumps via `cargo update`: `lru → 0.16.4`, `rand → 0.9.4 / 0.8.6` (RUSTSEC-2026-0097), `time → 0.3.47+` (RUSTSEC-2026-0009), `rustls-webpki → 0.103.13` (RUSTSEC-2026-0049/-0098/-0099/-0104). ([98f3eee](https://github.com/ilblu/belaf/commit/98f3eee))
* **deps:** Drop unused `http-cache-reqwest`, `reqwest-middleware`, and `cacache` (never imported in `src/`) to close the last unmaintained `bincode 1.3.3` warning (RUSTSEC-2025-0141). ([98f3eee](https://github.com/ilblu/belaf/commit/98f3eee))


## [1.3.3](https://github.com/ilblu/belaf/compare/v1.3.2...v1.3.3) (2025-12-31)


### Bug Fixes

* **api:** Improve error messages for API failures. Display actual messages from the API instead of generic "failed to create pull request" or "failed to get git credentials" — e.g. so users see "PR already exists" or "authentication expired" directly. ([e4c5960](https://github.com/ilblu/belaf/commit/e4c5960))


## [1.3.2](https://github.com/ilblu/belaf/compare/v1.3.1...v1.3.2) (2025-12-29)


### Bug Fixes

* **graph:** show dynamic project name in web view and update docs ([3a8888e](https://github.com/ilblu/belaf/commit/3a8888ec9a54079b362e138dcfee2a7cbf3ad586))

## [1.3.1](https://github.com/ilblu/belaf/compare/v1.3.0...v1.3.1) (2025-12-27)

# [1.3.0](https://github.com/ilblu/belaf/compare/v1.2.0...v1.3.0) (2025-12-24)


### Features

* **cli:** add belaf install command with API authentication ([#13](https://github.com/ilblu/belaf/issues/13)) ([146be3e](https://github.com/ilblu/belaf/commit/146be3ecfbf26dfcb2f23496db0a938208cce1ff))

# [1.2.0](https://github.com/ilblu/belaf/compare/v1.1.4...v1.2.0) (2025-12-22)


### Features

* **cli:** add LazyVim-style dashboard when running without arguments ([ca159a8](https://github.com/ilblu/belaf/commit/ca159a8e14ff70e516649385367bdc253ab58fc3))

## [1.1.4](https://github.com/ilblu/belaf/compare/v1.1.3...v1.1.4) (2025-12-22)


### Bug Fixes

* **cli:** unify version output for 'belaf version' and 'belaf --version' ([2c2bf7e](https://github.com/ilblu/belaf/commit/2c2bf7edae86db451eb2aa93f4e0d584554955ec))
* **manifest:** validate previous_tag exists before setting compare_url ([770fb39](https://github.com/ilblu/belaf/commit/770fb39e2860b21283da1d3d0600c3be6a503481))

## [1.1.3](https://github.com/ilblu/belaf/compare/v1.1.2...v1.1.3) (2025-12-22)

## [1.1.2](https://github.com/ilblu/belaf/compare/v1.1.1...v1.1.2) (2025-12-19)


### Bug Fixes

* correct command references and fix release workflow ([bd8f046](https://github.com/ilblu/belaf/commit/bd8f0465dd769786ba76e2e4862a0c5b868983ca))

## [1.1.1](https://github.com/ilblu/belaf/compare/v1.1.0...v1.1.1) (2025-12-19)


### Bug Fixes

* **auth:** update GitHub OAuth client ID to belaf app ([ae78426](https://github.com/ilblu/belaf/commit/ae7842633defcc2de0b1bc30eb593775d637bc6c))

# [1.1.0](https://github.com/ilblu/belaf/compare/v1.0.0...v1.1.0) (2025-12-19)


### Features

* **cli:** improve version output and add install success message ([8612d08](https://github.com/ilblu/belaf/commit/8612d08aec774c2f6ecd72a0827efbd8f45e6b74))

# 1.0.0 (2025-12-19)


### Features

* initial release of belaf ([a2dc442](https://github.com/ilblu/belaf/commit/a2dc4427d41d5dc64fb28d4bbf0f822c41e2420f))

# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2025-01-15

### Added

- **Multi-language monorepo support**: Detect and manage releases for Rust, Node.js, Python, Go, Elixir, and Swift projects
- **Dependency graph analysis**: Automatically detect internal dependencies between packages with `belaf graph`
- **Topological release ordering**: Release packages in the correct dependency order
- **Conventional commits parsing**: Parse commit history to determine version bumps
- **AI-powered changelogs**: Generate meaningful changelogs using Claude AI (optional)
- **Interactive TUI**: Rich terminal UI for release preparation with `belaf prepare`
- **GitHub OAuth authentication**: Secure device flow authentication with `belaf auth login`
- **Anthropic OAuth authentication**: PKCE flow for Claude API access
- **Release status overview**: View pending releases across all packages with `belaf status`
- **Multiple output formats**: JSON, YAML, and human-readable output
- **Shell completions**: Generate completions for Bash, Zsh, Fish, and PowerShell
- **Cross-platform support**: Linux (x86_64, aarch64), macOS (Intel, Apple Silicon), Windows

### Ecosystem Support

| Language | Version Detection | Dependency Detection | Version Update |
|----------|------------------|---------------------|----------------|
| Rust     | Cargo.toml       | Cargo workspace     | Cargo.toml     |
| Node.js  | package.json     | npm/pnpm workspaces | package.json   |
| Python   | pyproject.toml   | Poetry/Hatch        | pyproject.toml |
| Go       | go.mod           | Go modules          | Git tags       |
| Elixir   | mix.exs          | Mix umbrella        | mix.exs        |
| Swift    | Package.swift    | Swift Package       | Git tags       |

### Commands

- `belaf init` - Initialize release management for a repository
- `belaf status` - Show release status for all packages
- `belaf prepare` - Prepare releases with interactive TUI
- `belaf graph` - Visualize package dependency graph
- `belaf auth login` - Authenticate with GitHub and/or Anthropic
- `belaf auth logout` - Remove stored credentials
- `belaf auth status` - Check authentication status
- `belaf completions` - Generate shell completions
- `belaf version` - Show version information

[Unreleased]: https://github.com/ilblu/belaf/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/ilblu/belaf/releases/tag/v0.1.0
