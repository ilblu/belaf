# belaf

Release management CLI for monorepos and multi-language projects.

## Installation

### macOS (Homebrew)

```bash
brew install ilblu/tap/belaf
```

### Linux / macOS (Shell script)

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/ilblu/belaf/releases/latest/download/belaf-installer.sh | sh
```

### Windows (PowerShell)

```powershell
irm https://github.com/ilblu/belaf/releases/latest/download/belaf-installer.ps1 | iex
```

### Windows (Scoop)

```powershell
scoop bucket add belaf https://github.com/ilblu/scoop-bucket
scoop install belaf
```

### Windows (MSI Installer)

Download the latest `.msi` installer from the [releases page](https://github.com/ilblu/belaf/releases).

### From Source

```bash
cargo install --git https://github.com/ilblu/belaf
```

## Usage

### Authentication

```bash
# Login (interactive service selection)
belaf auth login

# Login to specific services
belaf auth login --github
belaf auth login --anthropic
belaf auth login --all

# Logout
belaf auth logout

# Check authentication status
belaf auth status
```

### Release Management

The CLI includes powerful release management for monorepos and multi-language projects with interactive TUI wizards.

```bash
# Initialize release management (interactive TUI wizard)
belaf init

# Check which projects have changes (interactive TUI)
belaf status

# Prepare a new release (interactive 4-step wizard)
belaf prepare

# View dependency graph
belaf graph
```

#### Commands

| Command | Description |
|---------|-------------|
| `belaf init` | Initialize release management with interactive TUI wizard |
| `belaf status` | Show release status with interactive project/commit browser |
| `belaf prepare` | Prepare releases using 4-step TUI wizard with auto-suggestions |
| `belaf graph` | Display project dependency graph |

#### Features

- **Interactive TUI Wizards**: All commands feature rich terminal UIs with keyboard navigation
- **AI-Powered Changelogs**: Generate changelogs with Claude AI (requires Anthropic authentication)
- **Multi-Language Support**: Automatically detects and manages versions for:
  - Rust (Cargo.toml)
  - Node.js (package.json)
  - Python (setup.py, pyproject.toml)
  - Go (go.mod)
  - Elixir (mix.exs)
  - Swift (Package.swift)
- **Dependency Resolution**: Analyzes project dependencies and determines correct release order
- **Automatic Changelog Generation**: Creates and updates CHANGELOG.md files based on Git commits
- **Monorepo-Aware**: Handles complex dependency graphs in monorepos with multiple interconnected projects
- **Git-Tag Based Versioning**: Version information stored in Git tags (`project-v1.2.3`)

#### Example Workflow

```bash
# 1. Initialize release management (interactive wizard)
cd /path/to/your/repo
belaf init

# 2. Make your changes and commit them
git add .
git commit -m "feat: add new feature"

# 3. Check what will be released (interactive browser)
belaf status

# 4. Prepare the release (4-step TUI wizard)
belaf prepare

# 5. Review and push the changes
git push origin main --tags
```

#### CI/CD Mode (--no-tui)

For automated pipelines, use `--no-tui` to skip interactive wizards:

```bash
# Initialize without TUI
belaf init --no-tui

# Status in text/JSON format
belaf status --no-tui
belaf status --format json

# Auto-bump based on conventional commits
belaf prepare --no-tui

# Per-project version bumps (CI mode)
belaf prepare -p gate:major,rig:minor,utils:patch
```

#### Dependency Graph

Visualize your project dependencies:

```bash
# Interactive web view (default)
belaf graph

# ASCII art graph
belaf graph --format ascii

# DOT format for Graphviz
belaf graph --format dot

# JSON format
belaf graph --format json
```

#### Configuration

Release management is configured via `belaf/config.toml` in your repository:

```toml
[release.repo]
upstream_urls = ["https://github.com/your-org/your-repo.git"]

[release.commit_attribution]
strategy = "scope_first"
scope_matching = "smart"

[release.projects.my-crate]
ignore = false
```

Configuration is automatically created when you run `belaf init`.

#### TUI Keyboard Shortcuts

**Status TUI:**
- `Tab` - Switch between Projects and Commits panels
- `↑/↓` or `j/k` - Navigate
- `PgUp/PgDn` - Fast scroll
- `g/G` - Go to top/bottom
- `q` - Quit

**Prepare TUI (4-step wizard):**
- Step 1: Project overview with auto-suggestions
- Step 2: Select bump type per project
- Step 3: Preview changes
- Step 4: Confirm and apply

**Init TUI:**
- `Space` - Toggle project selection
- `a` - Select all projects
- `n` - Deselect all projects
- `Enter` - Proceed to next step
- `Esc` - Go back

## Development

```bash
# Clone the repository
git clone https://github.com/ilblu/belaf.git
cd belaf

# Build
cargo build --release

# Run
cargo run -- --help

# Test
cargo test
```

## License

MIT
