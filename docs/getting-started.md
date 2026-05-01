# Getting started with belaf

This is the five-minute walkthrough: install the CLI, install the
GitHub App, run `belaf init`, ship a release.

## 1. Install the CLI

### macOS / Linux

```bash
curl -LsSf https://github.com/ilblu/belaf/releases/latest/download/belaf-installer.sh | sh
# or via Homebrew:
brew install ilblu/tap/belaf
```

### Windows

```powershell
irm https://github.com/ilblu/belaf/releases/latest/download/belaf-installer.ps1 | iex
# or via Scoop:
scoop bucket add belaf https://github.com/ilblu/scoop-bucket
scoop install belaf
```

### From source

```bash
cargo install belaf
```

Verify:

```bash
belaf --version
# belaf 3.0.0
```

## 2. Authenticate + install the GitHub App

```bash
cd path/to/your/repo
belaf install
```

This opens the device-flow auth in your browser, asks which workspace
to attach the install to, then redirects you to GitHub to install the
app on the repo.

When the redirect completes you'll see a green confirmation in the
TUI and the app shows up at `https://github.com/<owner>/<repo>/settings/installations`.

## 3. Initialize the repo

```bash
belaf init
```

The wizard runs the auto-detector against your working tree and
walks you through:

1. **Welcome** — workspace + repo summary.
2. **Unified selection** — every detected Release Unit, categorised:
   - 🔍 **Bundles** — multi-manifest units (Tauri, hexagonal Cargo, …)
   - 📦 **Standalone** — one-manifest units (typical npm / cargo packages)
   - 📱 **Externally-managed** — JVM SDKs with plugin-managed versions
   - ⚠️ **Drift** — paths that look released but aren't claimed by a unit
3. **Preset selection** — pick conventional-commits (default) or release-please-style.
4. **Upstream config** — confirm the upstream URL belaf detected.
5. **Confirmation** — reviews the TOML belaf will write.

Outputs:

```
belaf/
└── config.toml      # the canonical config (committed)
```

For CI / non-interactive use:

```bash
belaf init --ci --auto-detect --force
```

## 4. Ship a release

```bash
belaf prepare
```

This:

1. Walks every Release Unit's commits since its last tag.
2. Infers a semver bump per unit from conventional-commit prefixes.
3. Generates a changelog per unit.
4. Writes a manifest to `belaf/releases/<uuid>.json`.
5. Opens a PR titled `chore(release): N units` with the changelog as the
   body.

Merge the PR. The GitHub App takes it from there:

1. Tags every Release Unit at its new version.
2. Creates a GitHub Release per unit (or per Group, atomically).
3. Posts a comment with the released artifacts.

The dashboard at <https://app.belaf.dev> shows pending and completed
releases with the new typed metadata (bundles, cascades, satellites,
visibility) per unit.

## What's next

- [`docs/configuration.md`](configuration.md) — every config knob.
- [`docs/architecture.md`](architecture.md) — the moving parts.
- `belaf graph --web` — interactive visualization of your dependency
  graph.
- `belaf config explain --format json` — feed your CI a structured
  view of the resolved config.

## Common follow-ups

| Goal | Command |
|------|---------|
| Re-detect after adding a new package | `belaf init --auto-detect --force` |
| Skip a directory entirely | Add to `[ignore_paths]` in `belaf/config.toml` |
| Acknowledge an externally-released path (mobile, etc.) | Add to `[allow_uncovered]` |
| Bundle several manifests into one unit | Edit the unit's `source.manifests` array |
| Cascade a downstream SDK on schema bump | Set `cascade_from` on the SDK unit |
| Group atomic releases across ecosystems | Add a `[[group]]` block |

## Trouble?

- `belaf prepare` aborts with "uncovered release artifacts" → run
  `belaf init --auto-detect --force` and commit the updated config.
- `belaf install` can't find the workspace → check
  `https://app.belaf.dev/settings` to confirm the install attached.
- Manifest schema mismatch → make sure the CLI and GitHub App are
  both on 3.x. The protocol is incompatible across major versions
  by design.
