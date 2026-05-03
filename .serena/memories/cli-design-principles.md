# CLI Design Principles - Clikd Development CLI

## Core Philosophy: TUI-First Approach
"Ratatui überall wo geht" - Use Ratatui everywhere possible for modern, interactive developer experience.

## Hybrid Command Execution Pattern

### 1. Default Behavior (No Arguments)
```bash
clikd
```
**Result:** Interactive TUI command selector with:
- Visual menu of all available commands
- Keyboard navigation (arrow keys, Enter)
- Command descriptions and help text
- Direct execution from menu
- Modern, professional interface

### 2. Direct Command Execution (Scripts/CI-CD)
```bash
clikd start
clikd deploy production
clikd db migrate
```
**Result:** Direct command execution without TUI overhead
**Purpose:** Automation, scripts, CI/CD pipelines

### 3. Traditional Help (Optional)
```bash
clikd --help
clikd start --help
```
**Result:** Classic text output for pipe/grep/documentation
**Purpose:** Quick reference, scripting, integration with other tools

## Technical Implementation

### Entry Point Decision Tree
1. **No arguments** → Launch TUI command selector
2. **Valid command** → Execute command (with TUI if appropriate)
3. **--help flag** → Show classic help text
4. **--no-tui flag** → Force non-interactive mode (future)

### TUI Integration Libraries
- **ratatui** - Primary TUI framework
- **crossterm** - Terminal manipulation
- **clap** - Argument parsing (still needed for direct commands)
- **tui-clap** / **tuify** - Optional integration crates

## User Experience Goals

### For Frontend Developers
- Single command `clikd` shows everything available
- No need to remember exact command names
- Visual feedback and progress indicators
- Beautiful, modern interface

### For Backend Developers / DevOps
- Fast direct command execution for scripts
- Traditional CLI behavior still available
- No TUI overhead when not needed
- Automation-friendly

## Examples from Inspiration

### Supabase CLI Pattern
- `supabase` alone shows menu
- `supabase start` direct execution
- Branch-isolated development

### Modern Rust CLI Tools (2024-2025)
- **lazygit** - Full TUI for git operations
- **gitui** - Interactive git client
- **k9s** - Kubernetes TUI dashboard
- **bottom** - System monitor TUI

## Implementation Priority
Phase 1: Interactive command selector (default behavior)
Phase 2: Individual command TUIs (status, logs, db management)
Phase 3: Unified TUI application mode (`clikd tui`)

## Key Principle
**Professional, not flashy.** Avoid marketing language like "Amazing" or "Beautiful" in descriptions. Use technical, precise language appropriate for internal enterprise tooling.