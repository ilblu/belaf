# ADR 0002 — `belaf/bootstrap.toml` retirement

- **Status**: Implemented in 3.0 (writer retired; runtime reader kept
  as an inert backward-compat shim for old configs)
- **Date**: 2026-05-01

## Context

`belaf/bootstrap.toml` was introduced in 1.x to record the initial
version + qualified names of every project at the moment `belaf init`
ran. The runtime read it at every `belaf prepare` to know each
project's "before" state and to seed `internal_deps` with `Manual(...)`
requirements.

The file had three roles:

1. **Per-project initial version** — needed to compute the next bump
   when the manifest version was already in some odd state at init
   time.
2. **Per-project qnames** — needed to tie back the right ecosystem
   loader to a project after the index walk.
3. **`internal_deps[i].belaf_requirement = Manual(version)`** — set on
   bootstrap so deps could be expressed against a known concrete
   version even before any release tag existed.

In 2.0+ a global `belaf-baseline` git tag absorbed (1) and (2) for
most cases, but the `Manual(...)` dep update at init time still flowed
through `bootstrap.toml`'s writer.

## Decision (target architecture)

Replace `bootstrap.toml` with runtime initial-state derivation in a
new module `core/release_unit/initial_state.rs`:

- For each ReleaseUnit, `initial_state(unit) = max(manifest_version,
  latest_matching_tag)`.
- First-prepare-after-init creates `belaf-baseline-<unit>` tags
  (per-unit instead of one global tag) where no matching tag exists.
- `internal_deps[i].belaf_requirement` resolution moves into the
  resolver pipeline, populated against the freshly-derived initial
  states.

## Status (implemented in 3.0)

The init-time writer was retired in both `cmd::init::run` and
`cmd::init::wizard::run`. The dep-requirement update walk (which
converts each project's `internal_deps[i].belaf_requirement` from
`Commit(...)` to `Manual(version)`) was inlined directly — no
serialisation to disk.

Two pieces are intentionally **not** retired:

1. The runtime reader in `core/git/repository.rs:281-309` stays. It
   silently no-ops when the file doesn't exist (which is the new
   default for fresh 3.0 installs) and reads the file when it does
   (so 2.x configs that still have a bootstrap.toml continue to work
   without surprise).
2. The global `belaf-baseline` git tag stays. With the writer gone,
   it's now the single source of truth for "before any
   project-specific tag existed" — exactly what (1) was redundantly
   tracking in the file. Per-unit `belaf-baseline-<name>` tags
   remain a 3.x architectural improvement rather than a 3.0 ship-blocker.

The `BootstrapConfiguration` / `BootstrapProjectInfo` types stay as
inert backward-compat shims for the reader path.

## Consequences (when implemented)

- One less file in `belaf/` after init. No more "what is this for"
  questions from new users.
- Per-unit baseline tags (`belaf-baseline-<unit>`) replace the single
  global `belaf-baseline` tag — better aligned with the multi-unit
  model.
- Resolver pipeline becomes the single source for dep-requirement
  resolution; no separate write step.
