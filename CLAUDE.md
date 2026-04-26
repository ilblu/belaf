# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

The repo uses [`just`](https://github.com/casey/just) as a task runner — `justfile` is the source of truth.

```bash
just              # list all recipes
just check        # cargo check --all-features
just test         # BELAF_NO_KEYRING=1 cargo test --all-features
just lint         # cargo clippy --all-targets --all-features -- -D warnings
just format       # cargo fmt
just format-check # cargo fmt -- --check
just audit        # cargo audit
just ci           # check + test + lint + format-check + audit
just fix          # cargo fmt + cargo clippy --fix
just all          # check + test + lint + format
```

`BELAF_NO_KEYRING=1` is required when running tests (the keyring crate hangs in headless test environments). Always invoke the test suite via `just test` rather than `cargo test`, or set the env var manually.

Run a single test:

```bash
BELAF_NO_KEYRING=1 cargo test --all-features test_name
BELAF_NO_KEYRING=1 cargo test --test test_dependency_graph   # one integration file
BELAF_NO_KEYRING=1 cargo test --all-features -- --nocapture  # show stdout
```

The Rust toolchain is pinned to **1.91** via `rust-toolchain.toml`. The `csharp` ecosystem is gated behind a feature flag (`--features csharp`); `--all-features` enables it.

## Architecture

belaf is a Rust CLI that manages semantic releases across multi-language monorepos. The big-picture flow is **PR-based releases**: the CLI never publishes packages directly — it produces a release manifest in a PR, and a separate GitHub App finalizes the release on merge.

### Entry points and command dispatch

- `src/main.rs` — minimal: parses CLI, sets up tracing, runs `pre_execute()` (checks for updates against `https://api.belaf.dev`), then dispatches. With **no subcommand**, falls through to `cmd::dashboard::run()` (a Ratatui menu) instead of printing help.
- `src/lib.rs` — declares the entire module tree explicitly (no `mod.rs` files; clippy denies `mod_module_files`). The `execute()` function is the central match over `Commands`.
- `src/cli.rs` — clap derive definitions. Each command has rich `long_about` text — when adding a command, follow the same pattern (short `about`, multi-paragraph `long_about` with bullet sections).

### Module layout

```
src/
├── cli.rs              clap definitions (Cli, Commands, *Args)
├── cmd/                one file per subcommand; subdirs hold TUI wizards
│   ├── prepare.rs + prepare/wizard.rs
│   ├── init.rs + init/wizard.rs
│   ├── graph.rs + graph/{wizard.rs, browser.rs, templates/}
│   └── dashboard.rs    no-arg entry TUI
├── core/
│   ├── workflow.rs     ReleasePipeline — the orchestrator (see below)
│   ├── session.rs      AppBuilder/AppSession — wires repo + project graph + config
│   ├── root.rs         pre_execute hook (update check)
│   ├── config.rs       belaf/config.toml schema (syntax::* types)
│   ├── manifest.rs     ReleaseManifest schema (belaf/releases/*.json)
│   ├── ecosystem/      one file per language (cargo, npm, pypa, go, elixir, swift, csproj)
│   ├── changelog/      git-cliff-style template engine (Tera-based)
│   ├── git/            libgit2 wrapper around the working repo
│   ├── github/         octocrab client + PR creation
│   ├── api/            client for api.belaf.dev (auth, releases)
│   ├── auth/token.rs   keyring-backed token storage
│   ├── graph.rs        petgraph DAG of inter-project dependencies
│   ├── bump.rs         conventional-commit → semver bump inference
│   └── ui/             shared Ratatui components
└── utils/              theme, file_io, version_check
```

### Release pipeline (the core flow)

`core::workflow::ReleasePipeline` (in `src/core/workflow.rs`) is the heart of `belaf prepare`:

1. **Discover** projects via `Ecosystem` impls (one per language in `core/ecosystem/`).
2. **Build dependency graph** (`core::graph`, petgraph) — used to topologically order releases.
3. **Analyze commits** since each project's last tag (`core::bump`, `git-conventional`).
4. **Infer bumps** (auto/major/minor/patch) using `BumpConfig` from `belaf/config.toml`.
5. **Generate changelogs** via Tera templates in `core::changelog` (compatible with git-cliff conventions; see `cliffy.toml` for the full TOML option reference).
6. **Write manifests** to `belaf/releases/<uuid>.json` — schema versioned, see `MANIFEST_DIR` and `SCHEMA_VERSION` in `core/manifest.rs`.
7. **Create branch + commit + push + open PR** via `core::github::pr`.

The manifest is the contract: a downstream GitHub App consumes it to publish tags/releases. Don't make the CLI publish directly.

### Ecosystem abstraction

Adding language support means implementing the `Ecosystem` interface in `core/ecosystem/<lang>.rs` and registering its `EcosystemType` variant in `core/ecosystem/types.rs`. Each ecosystem owns: manifest detection, version reading, version writing, and dependency extraction. Tests live alongside in the same file (unit tests) plus broader scenarios in `tests/test_ecosystem_edge_cases.rs`.

### TUI and CI modes

Every user-facing command supports both an interactive Ratatui TUI and a `--ci` mode. The `--ci` flag should always: skip prompts, emit JSON when applicable, and fail loudly rather than asking for confirmation. The dashboard (no-arg) is TUI-only and dispatches into the same `cmd::*::run()` functions used by direct commands.

### Error handling convention

- `anyhow::Result` at the application layer (`main.rs`, `cmd/*`, top of `lib.rs::execute`).
- `thiserror` for typed errors in `core::errors`, `core::api::error`, `core::changelog::error`.
- Don't mix the two in one function. Use `.context()` to add information when bubbling up.

### Lint rules that catch real bugs

`Cargo.toml` enforces (deny-level): `mod_module_files` (never create `mod.rs`), `wildcard_imports`, `enum_glob_use`. Warnings: `todo`, `dbg_macro`, `allow_attributes`, `unsafe_code`. `clippy.toml` allows `unwrap`/`expect` in tests only.

### Environment variables

- `BELAF_NO_KEYRING=1` — disable OS keyring (required for tests, useful in CI).
- `BELAF_API_URL` — override the belaf API endpoint (default `https://api.belaf.dev`); used by the update checker and `cmd::install`.
- `BELAF_WEB_URL` — override the dashboard URL opened from the TUI.
- `RUST_LOG` — standard tracing filter; CLI verbosity flags (`-v`, `-vv`, `-vvv`) override the level.
- `CI` / `GITHUB_ACTIONS` / `GITLAB_CI` etc. — auto-detected by `session::detect_ci_environment` to switch off interactive prompts.

### Tests

Integration tests in `tests/` are organized by feature area (`test_release_prepare.rs`, `test_dependency_graph.rs`, `test_changelog.rs`, etc.) and share helpers in `tests/common.rs` (notably `TestRepo`, which spins up a temp git repo per test). The project uses `insta` for snapshot tests, `assert_cmd`/`assert_fs` for CLI integration, `wiremock` for HTTP mocking, and `trycmd` is wired up but not yet broadly used.

### Distribution

Releases are built via `cargo-dist` (config in `dist-workspace.toml`) and published to GitHub Releases, Homebrew tap (`ilblu/homebrew-tap`), and a Scoop bucket. The release profile is size-optimized (`opt-level = "z"`, LTO, `panic = "abort"`). `build.rs` injects `TARGET` and `RUSTC_VERSION` at compile time for the `version` command, and additionally runs `progenitor` to code-generate the API client (see below).

Release trigger: pushing a SemVer tag (`v1.2.3`, etc.) to `main` triggers `.github/workflows/release.yml`. Plain pushes/PRs to `main` only run `ci.yml`. There is no auto-tag bot — tags are pushed by hand (or by belaf itself).

### API client is code-generated

`src/core/api/types.rs` has **no hand-written wire structs** for `/api/cli/*` endpoints. They are all re-exported from `core::api::generated::types::*`, which is `progenitor`-generated at build time from `api-spec/openapi.cli.json`. The spec itself is sourced from the github-app repo (`apps/api/openapi.cli.json`).

- **Do not edit `types.rs` to add a wire struct.** Update the Zod schema in the github-app repo, regenerate the spec, and copy it into `api-spec/openapi.cli.json`. The compiler will then surface every drift in `client.rs` and downstream call sites as a type error — that is the entire point.
- **Hand-written exceptions** that *stay* in `types.rs`: `StoredToken` (local on-disk format, not wire), `DeviceCodeRequest/Response` and `TokenPollRequest/Response` (Better-Auth device-flow endpoints under `/api/auth/*`, not part of `/api/cli/*`), and `CreatePullRequestParams<'a>` (a borrowed builder used only inside `client.rs`).
- **`build.rs`** opens the spec, runs `progenitor::Generator::default().generate_tokens(&spec)`, pretty-prints with `prettyplease`, and writes the result to `$OUT_DIR/belaf_api_codegen.rs`. `src/core/api/generated.rs` is a one-liner that `include!`s that file plus a wide `#![allow(...)]` for clippy lints we don't want imposed on generated code.
- **`octocrab` was removed.** All GitHub data flows through the belaf API; the CLI never contacts `api.github.com` except for the update check in `utils/version_check.rs`. If you find yourself reaching for a direct GitHub call, add the endpoint to `/api/cli/*` instead.

### Schema-update workflow (cross-repo)

1. In `github-app`: edit `apps/api/src/routes/cli/schemas.ts` (or `routes.ts` for new endpoints), then `bun run apps/api/scripts/generate-openapi.ts`.
2. The github-app's snapshot test (`bun test src/routes/cli/openapi-snapshot.test.ts`) verifies the regenerated spec matches what's committed.
3. `cp ../github-app/apps/api/openapi.cli.json api-spec/openapi.cli.json`.
4. `cargo build` here — every drift between the new spec and existing call sites lands as a compile error.
5. Fix the call sites (often just `client.rs`); commit `api-spec/openapi.cli.json` together with the call-site fixes.
