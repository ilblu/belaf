# ADR 0002 — `belaf/bootstrap.toml` retirement

- **Status**: Accepted in principle, deferred in implementation (belaf 3.0)
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

## Status

**Deferred to a focused follow-up PR**, marked at the top of
`src/cmd/init.rs` as `TODO(belaf-3.0/wave1f)`. Two reasons for the
deferral:

1. The bootstrap writer in `cmd::init::run` and `cmd::init::wizard::run`
   walks the toposorted graph and sets `dep.belaf_requirement =
   Manual(version)`. Retiring the writer requires routing that
   dep-resolution through the resolver pipeline first — a self-contained
   refactor that wants its own attention.
2. `core/git/repository.rs` currently reads `bootstrap.toml`
   conditionally (silently no-ops if missing); the runtime path is
   functional with or without the file. Retiring it is architectural
   cleanup, not a 3.0 functional blocker.

## Consequences (when implemented)

- One less file in `belaf/` after init. No more "what is this for"
  questions from new users.
- Per-unit baseline tags (`belaf-baseline-<unit>`) replace the single
  global `belaf-baseline` tag — better aligned with the multi-unit
  model.
- Resolver pipeline becomes the single source for dep-requirement
  resolution; no separate write step.
