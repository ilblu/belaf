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
