<p align="center">
  <img src="https://raw.githubusercontent.com/ilblu/belaf/main/.github/assets/logo.svg" alt="belaf" width="400">
</p>

<p align="center">
  <strong>Semantic release management for monorepos.</strong>
</p>

<p align="center">
  <a href="https://github.com/ilblu/belaf/actions/workflows/ci.yml"><img src="https://github.com/ilblu/belaf/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/ilblu/belaf/releases"><img src="https://img.shields.io/github/v/release/ilblu/belaf?color=blue" alt="Release"></a>
  <a href="https://github.com/ilblu/belaf/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-MIT-green" alt="License"></a>
  <a href="https://crates.io/crates/belaf"><img src="https://img.shields.io/crates/d/belaf?color=orange" alt="Downloads"></a>
</p>

<p align="center">
  <a href="#installation">Installation</a> •
  <a href="#quick-start">Quick Start</a> •
  <a href="#features">Features</a> •
  <a href="#supported-languages">Languages</a> •
  <a href="#documentation">Docs</a>
</p>

---

## The Problem

Managing releases in a monorepo is painful:

- **Version chaos** — Which packages changed? What versions should they be?
- **Dependency hell** — Package A depends on B, which depends on C. Release order matters.
- **Changelog fatigue** — Writing changelogs manually is tedious and error-prone.
- **Multi-language mess** — Your repo has Rust, TypeScript, and Python. Good luck.

## The Solution

**belaf** analyzes your monorepo, detects changes, resolves dependencies, and prepares releases with a single command. It understands conventional commits, generates changelogs, and handles the entire release workflow through an intuitive TUI.

```bash
belaf prepare
```

That's it. belaf figures out the rest.

---

## Features

- **Smart Detection** — Automatically discovers projects across 6 languages
- **Dependency Resolution** — Determines correct release order based on inter-project dependencies
- **Conventional Commits** — Analyzes commit history to suggest semantic version bumps
- **Interactive TUI** — Beautiful terminal interface with keyboard navigation
- **CI/CD Ready** — Full automation support with `--no-tui` mode and JSON output
- **PR Workflow** — Creates pull requests with release manifests for team review

---

## Quick Start

```bash
# Install
brew install ilblu/tap/belaf

# Initialize in your monorepo
cd your-monorepo
belaf init

# See what changed
belaf status

# Prepare releases
belaf prepare
```

---

## Installation

### macOS

```bash
brew install ilblu/tap/belaf
```

### Linux / macOS (Shell)

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/ilblu/belaf/releases/latest/download/belaf-installer.sh | sh
```

### Windows

```powershell
# PowerShell
irm https://github.com/ilblu/belaf/releases/latest/download/belaf-installer.ps1 | iex

# Scoop
scoop bucket add belaf https://github.com/ilblu/scoop-bucket
scoop install belaf
```

### Cargo

```bash
cargo install belaf
```

---

## Supported Languages

| Language | Manifest | Version Source |
|----------|----------|----------------|
| **Rust** | `Cargo.toml` | `version` field |
| **Node.js** | `package.json` | `version` field |
| **Python** | `pyproject.toml`, `setup.py` | PEP 440 version |
| **Go** | `go.mod` | Git tags |
| **Elixir** | `mix.exs` | `version` in project |
| **Swift** | `Package.swift` | Git tags |

---

## Commands

| Command | Description |
|---------|-------------|
| `belaf init` | Initialize release management in your repo |
| `belaf status` | Show which projects have unreleased changes |
| `belaf prepare` | Prepare releases with version bumps and changelogs |
| `belaf graph` | Visualize project dependency graph |

### CI/CD Mode

All commands support `--no-tui` for automation:

```bash
# JSON output for scripts
belaf status --format json

# Auto-bump based on commits
belaf prepare --no-tui

# Explicit version control
belaf prepare -p api:major,sdk:minor,utils:patch
```

---

## How It Works

1. **Discover** — belaf scans your repo for supported manifest files
2. **Analyze** — Parses dependencies between projects
3. **Detect** — Identifies changes since last release using git history
4. **Suggest** — Recommends version bumps based on conventional commits
5. **Generate** — Creates changelogs from commit messages
6. **Release** — Updates versions, creates tags, opens PR

---

## Configuration

belaf stores configuration in `belaf/config.toml`:

```toml
[release.repo]
upstream_urls = ["https://github.com/your-org/your-repo.git"]

[release.commit_attribution]
strategy = "scope_first"    # How to attribute commits to projects
scope_matching = "smart"    # Fuzzy matching for commit scopes

[release.projects.my-package]
ignore = false              # Set true to exclude from releases
```

---

## Why belaf?

| Feature | belaf | git-cliff | semantic-release | changesets | release-please |
|---------|-------|-----------|------------------|------------|----------------|
| Multi-language | ✅ 6 languages | ❌ | ❌ Node.js only | ❌ Node.js only | ⚠️ Limited |
| Monorepo support | ✅ Native | ⚠️ Manual | ❌ | ⚠️ Basic | ⚠️ Basic |
| Commit-to-package matching | ✅ Smart | ❌ | ❌ | ❌ | ❌ |
| Interactive TUI | ✅ | ❌ | ❌ | ❌ | ❌ |
| Dependency resolution | ✅ | ❌ | ❌ | ⚠️ Basic | ❌ |
| Single binary | ✅ | ✅ | ❌ | ❌ | ❌ |
| No runtime deps | ✅ | ✅ | ❌ Node.js | ❌ Node.js | ❌ Node.js |

---

## belaf vs git-cliff

Looking for a monorepo-friendly alternative to git-cliff? Here's what belaf does differently:

| Capability | git-cliff | belaf |
|------------|-----------|-------|
| **Multi-package config** | ❌ One config per package | ✅ All packages in one file |
| **Automatic commit routing** | ❌ Manual `include_paths` | ✅ Smart scope matching |
| **Scope-to-package mapping** | ❌ | ✅ `feat(api)` → api package |
| **Coordinated bumping** | ❌ Run per package | ✅ Analyzes entire monorepo |
| **Dependency-aware releases** | ❌ | ✅ Correct release order |
| **Multi-ecosystem** | ❌ | ✅ Rust, Node, Python, Go, Elixir, Swift |
| **Changelog generation** | ✅ Tera templates | ✅ Tera templates |
| **Interactive workflow** | ❌ CLI only | ✅ TUI wizard |

### git-cliff monorepo workflow

```bash
# Run separately for each package
git cliff -c packages/api/cliff.toml --include-path "packages/api/**"
git cliff -c packages/sdk/cliff.toml --include-path "packages/sdk/**"
git cliff -c packages/web/cliff.toml --include-path "packages/web/**"
# Hope you got the dependency order right...
```

### belaf monorepo workflow

```bash
belaf prepare
# Done. All packages analyzed, ordered, and released.
```

---

## Contributing

Contributions are welcome! Please read our [Contributing Guide](CONTRIBUTING.md) before submitting a PR.

```bash
# Clone
git clone https://github.com/ilblu/belaf.git
cd belaf

# Build
cargo build

# Test
cargo test

# Run
cargo run -- --help
```

---

## License

MIT © [ilblu](https://github.com/ilblu)
