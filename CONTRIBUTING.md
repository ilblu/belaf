# Contributing to belaf

Thank you for your interest in contributing to belaf! This document provides guidelines and instructions for contributing.

## Code of Conduct

Please be respectful and constructive in all interactions. We welcome contributors of all experience levels.

## Getting Started

### Prerequisites

- Rust 1.75 or later
- Git

### Setup

```bash
# Clone the repository
git clone https://github.com/ilblu/belaf.git
cd belaf

# Build
cargo build

# Run tests
cargo test

# Run with debug output
RUST_LOG=debug cargo run -- --help
```

## Development Workflow

### Running Tests

```bash
# All tests
cargo test

# Specific test
cargo test test_name

# With output
cargo test -- --nocapture
```

### Code Style

We use standard Rust formatting and linting:

```bash
# Format code
cargo fmt

# Run clippy
cargo clippy -- -D warnings
```

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

```
src/
├── cli.rs              # CLI argument parsing (clap)
├── cmd/                # Command implementations
│   ├── auth.rs         # Authentication commands
│   ├── init.rs         # Repository initialization
│   ├── prepare.rs      # Release preparation
│   ├── status.rs       # Status display
│   └── graph.rs        # Dependency graph
├── core/
│   ├── ai/             # Claude AI integration
│   ├── auth/           # GitHub/Anthropic auth
│   ├── ecosystem/      # Language-specific parsers
│   ├── git/            # Git operations
│   ├── release/        # Release logic
│   └── ui/             # TUI components
└── utils/              # Shared utilities
```

## Adding Language Support

To add support for a new language:

1. Create a new file in `src/core/ecosystem/`
2. Implement the `Ecosystem` trait
3. Add detection logic in `src/core/ecosystem/types.rs`
4. Add tests in the same file
5. Update documentation

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
