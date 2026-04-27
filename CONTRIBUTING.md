# Contributing to belaf

Thank you for your interest in contributing to belaf! This document provides guidelines and instructions for contributing.

## Code of Conduct

Please be respectful and constructive in all interactions. We welcome contributors of all experience levels.

## Getting Started

### Prerequisites

- Rust 1.91 (pinned by `rust-toolchain.toml`)
- [`just`](https://github.com/casey/just) for the task runner
- `git` ≥ 2.40 (the Maven tag-format validator shells out to
  `git check-ref-format`)

### Setup

```bash
# Clone the repository
git clone https://github.com/ilblu/belaf.git
cd belaf

# Build
cargo build

# Run tests (BELAF_NO_KEYRING=1 is required — the keyring crate hangs
# in headless test environments)
just test

# Run with debug output
RUST_LOG=debug cargo run -- --help
```

## Development Workflow

### `just` recipes

The `justfile` is the source of truth for development commands:

```bash
just              # list all recipes
just check        # cargo check --all-features
just test         # BELAF_NO_KEYRING=1 cargo test --all-features
just lint         # cargo clippy --all-targets --all-features -- -D warnings
just format       # cargo fmt
just format-check # cargo fmt -- --check
just audit        # cargo audit
just ci           # check + test + lint + format-check + audit
```

### Single test file or test name

```bash
BELAF_NO_KEYRING=1 cargo test --all-features test_name
BELAF_NO_KEYRING=1 cargo test --test test_groups   # one integration file
```

### Code style enforced as errors

Lint config (in `Cargo.toml`):

- **deny**: `mod_module_files` (no `mod.rs` files), `wildcard_imports`,
  `enum_glob_use`, `allow_attributes` (use `#[expect]` with a reason).
- **warn**: `todo`, `dbg_macro`, `unsafe_code`.

`#[allow]` attributes won't compile through `just lint` — replace with
`#[expect(lint, reason = "…")]` or fix the underlying issue.

### Commit Messages

We follow [Conventional Commits](https://www.conventionalcommits.org/):

```
feat: add new feature
fix: resolve bug in parser
docs: update README
refactor: simplify dependency resolution
test: add tests for changelog generator
chore: update dependencies
```

**Types:**
- `feat` — New feature (minor version bump)
- `fix` — Bug fix (patch version bump)
- `docs` — Documentation only
- `refactor` — Code change that neither fixes a bug nor adds a feature
- `test` — Adding or updating tests
- `chore` — Maintenance tasks

**Breaking Changes:**

Add `!` after the type or include `BREAKING CHANGE:` in the footer:

```
feat!: change API signature

BREAKING CHANGE: The `prepare` command now requires explicit confirmation.
```

## Pull Requests

1. Fork the repository
2. Create a feature branch (`git checkout -b feat/amazing-feature`)
3. Make your changes
4. Run tests and linting (`cargo test && cargo clippy`)
5. Commit with a descriptive message
6. Push to your fork
7. Open a Pull Request

### PR Guidelines

- Keep PRs focused — one feature or fix per PR
- Update tests for new functionality
- Update documentation if needed
- Ensure CI passes

## Project Structure

See `CLAUDE.md` for the canonical module map. High-level:

- `src/cli.rs` — clap definitions
- `src/cmd/` — one file per subcommand
- `src/core/` — domain logic
  - `wire/` — typify-generated v2 manifest types + ergonomic domain wrappers
  - `ecosystem/` — one file per language plus the `Ecosystem` trait + registry
  - `group.rs`, `tag_format.rs`, `bump_source.rs` — v2 release-shape primitives
  - `workflow.rs` — orchestrator (`ReleasePipeline`)
- `schemas/manifest.v2.0.schema.json` — canonical wire format

## Adding Language Support

belaf 2.0 makes this **two lines of editing** plus the loader file
itself. There is no central enum to widen.

1. Create `src/core/ecosystem/<lang>.rs` with a `FooLoader` struct.
2. Implement two inherent helpers — `record_path(&mut self, dirname, basename)`
   for the index scan and `into_projects(self, app, pconfig)` for graph
   registration — so unit tests can drive the loader without standing up
   a real `Repository`/`ProjectGraphBuilder`.
3. Add an `impl Ecosystem for FooLoader` block (in
   `src/core/ecosystem/registry.rs`) supplying `name`, `display_name`,
   `version_file`, `tag_format_default`, `tag_template_vars`, and trait
   shims that delegate to the inherent helpers.
4. Add `r.register(Box::new(FooLoader::default()))` to
   `EcosystemRegistry::with_defaults()`.
5. Add the wire string to `KNOWN_ECOSYSTEMS` in
   `src/core/wire/known.rs` (one line) so manifests classify it as
   `Known(...)` rather than `Unknown(...)`.
6. Add the per-ecosystem display info on `KnownEcosystem::display_name`
   and `version_file`.
7. Tests: unit tests live alongside in the same file; integration
   scenarios go in `tests/test_<lang>.rs` if the loader has interesting
   behaviour worth covering through `belaf init` + `prepare --ci`.

No schema bump, no DB migration, no central match-arm to update.

## Modifying the API client

The Rust types under `src/core/api/types.rs` for `/api/cli/*` endpoints are **code-generated** at build time from `api-spec/openapi.cli.json` via `progenitor` (see `build.rs`). Do **not** hand-edit a wire struct in `types.rs` — the next build will overwrite the change with whatever the spec says.

To change a wire field:

1. In the github-app repo, edit the Zod schema in `apps/api/src/routes/cli/schemas.ts` and regenerate via `bun run apps/api/scripts/generate-openapi.ts`.
2. Copy the new spec into this repo: `cp ../github-app/apps/api/openapi.cli.json api-spec/openapi.cli.json`.
3. `cargo build` — the compiler will surface every drifted call site as a type error. Fix them.
4. Commit `api-spec/openapi.cli.json` alongside the call-site fixes.

The hand-written exceptions that *do* live in `types.rs` are documented at the top of that file: `StoredToken`, the device-flow request/response pair, and `CreatePullRequestParams`. Auth endpoints (`/api/auth/device/*`) are served by Better Auth, not by the schema-first `/api/cli/*` layer, so they are not in the generated module.

## Questions?

Open an issue or start a discussion on GitHub.

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
