## 3.0.2 (2026-05-02)

Bugfix: the `belaf init` ReleaseUnit selection screen rendered
duplicate entries when a Bundle covers manifests the loaders also
pick up independently. The user-visible symptom was a Tauri triplet
appearing three times — once as the Bundle, twice as inner/outer
manifests — and `sdks/kotlin` showing as two separate Bundle rows.

### What changed

- **Standalone units covered by a Bundle path are now hidden.** The
  Tauri detector hits at `apps/clients/desktop/`; the npm loader
  finds the outer `package.json` and the cargo loader finds
  `src-tauri/Cargo.toml`. Pre-fix, all three rendered. Post-fix,
  only the Bundle row shows — its `[[release_unit]]` block on
  config emit covers both inner manifests so the loaders skip them
  at release time anyway.
- **Same-path Bundle dedup.** When two detectors fire on the same
  path (e.g. `sdks/kotlin` matching both `jvm_library` and
  `sdk_cascade_member`), the wizard now keeps only the first hit.
  `detect_all` runs higher-specificity scanners first, so
  `jvm-library/build.gradle.kts` wins over `sdk-cascade-member` —
  the more useful label.
- `SingleProject` and `NestedMonorepo` hits don't shadow standalone
  units. They describe the repo shape rather than a multi-manifest
  bundle, so a `single-project` repo still surfaces its Cargo crate
  as a Standalone row.

### Tests

- New regression test `tauri_bundle_hides_inner_and_outer_standalones`
  pins the Tauri-triplet behaviour against `apps/clients/desktop/`
  with an unrelated standalone that *must not* be hidden.
- New regression test `same_path_bundles_dedup_keeping_first_emission`
  pins the `sdks/kotlin` jvm-library + sdk-cascade-member case.
- 398 tests total green; clikd smoke run all 5 phases pass.

---

## 3.0.1 (2026-05-02)

Wizard polish — visual cleanup of the `belaf init` ReleaseUnit
selection screen reported as cluttered after the 3.0 ship. No
behavioural changes; pure rendering refactor + the icon-mode opt-in.

### What changed

- **Universal Unicode icons by default.** The selection screen used
  emojis (`✅`, `⬜`, `🔍`, `📦`, `📱`) which render at variable widths
  and break column alignment. Replaced with single-cell Unicode shapes
  that work on every terminal + font without setup:
  - `⬢` (Black Hexagon) for the Bundles header
  - `◆` (Black Diamond) for Standalone
  - `◇` (Outline Diamond) for Externally-managed
  - `●` / `○` for checked / unchecked rows
  - `—` em-dash for non-togglable mobile rows
  - `❖` for the screen header banner
- **Layout: blank-line spacing between categories** plus consistent
  4-space indent under each header. Labels in each row are padded to
  the longest-label width so the secondary column always lines up.
- **Per-row ecosystem icon column** (rust crab, npm logo, TypeScript,
  Swift, Kotlin, Java, Python, Go, Elixir, C#, Tauri) is **opt-in**
  via `BELAF_ICONS=nerd`. Only renders when the user has a Nerd Font
  installed; otherwise the column is empty and other modes still look
  clean.
- **`BELAF_ICONS=ascii`** — pure-ASCII fallback (`[x]`, `[ ]`, `[*]`)
  for CI logs / SSH sessions / dumb terminals.
- **`DetectedUnit.ecosystem: Option<String>`** — new field on the
  wizard state struct, populated from the loader's qualified-name
  pair so each row knows which language icon to show.

### Scope of the icon-mode env var

`BELAF_ICONS` is read once per process at first wizard render via
`OnceLock` — switching it mid-session has no effect (the wizard is a
short-lived flow anyway). Valid values: `unicode` (default), `nerd`,
`ascii`. Any other value falls back to `unicode`.

### Tests

- Insta snapshot for the unified-selection layout regenerated.
- New unit test in `wizard::glyphs` that asserts every glyph getter
  returns a non-empty string in every mode (guards against silently
  losing a category mapping when extending the enum).
- Smoke run against `clikd-project/clikd` (37 ReleaseUnits) green —
  all 5 phases (`init`, `status`, `explain`, `graph`, `prepare`).

---

## 3.0.0 (2026-05-01)

Belaf 3.0 — clean architectural reset across CLI, github-app API, and
dashboard. No backward compatibility with 2.x; a fresh `belaf init`
is required for any repo previously on 2.x. Coordinated big-bang
release across all three components per ADR 0005's audit (no
production data).

See `docs/adr/0001..0005-*.md` for the architectural decisions.

### Breaking changes

- **Manifest wire format bumps to schema 3.0.** `schema_version` is
  literally `"3.0"`. Six previously-config-only fields are now
  first-class on each release: `bundle_manifests`, `external_versioner`,
  `version_field_spec`, `cascade_from`, `visibility`, `satellites`.
  github-app v3.0 rejects v2.0 manifests with a clear "upgrade
  producer" error. (ADR 0003)
- **`ReleaseUnit` is the sole declarative primitive.** The internal
  `Project` struct has been renamed `ResolvedReleaseUnit` and is
  treated as resolver-internal. Every reference to `Project*` in the
  public API is gone (`ProjectId` → `ReleaseUnitId`, `ProjectGraph` →
  `ReleaseUnitGraph`, etc.). (ADR 0001)
- **`[projects."<name>"].tag_format` precedence dropped.** New
  precedence: `[group.<id>].tag_format` > ecosystem default.
- **github-app `projects` tenancy tier dropped entirely** — new
  tenant hierarchy: `Workspace → Repo → ReleaseUnit → Release`.
  ~1700 LOC removed. Per-repo `tags: text[]` column added as the
  replacement UI grouping mechanism. (ADR 0005)
- **Dashboard `/projects/$slug/*` routes deleted** — 17 routes plus
  6 project-aware components.

### New features

- **`UnifiedSelectionStep` in `belaf init`** — one categorized
  selection screen replaces the 2.x split between manual project list
  + auto-detect bundle review. Three categories: 🔍 Bundles
  (multi-manifest auto-detected) / 📦 Standalone (single-manifest
  loader output) / 📱 Externally-managed (mobile, read-only).
- **`belaf config explain --format=json`** emits a serde-derived
  payload for github-app dashboard consumption.
- **Repo-tags grouping in github-app dashboard** — multi-select tag
  chips filter the workspace overview.

### Resolved follow-ups (delivered in this release)

What was originally framed as "deferred 3.1 work" landed in 3.0:

- `bootstrap.toml` retired (ADR 0002). Per-unit baseline tags + manifest
  version-at-runtime replace it; `core/release_unit/initial_state.rs`
  derives the baseline.
- `src/core/ecosystem/maven.rs` (1521 LOC) split into `maven.rs` +
  `pom_parser.rs` + `pom_rewriter.rs` + `property_resolver.rs`.
- `src/core/release_unit/detector.rs` (1192 LOC) split into the slim
  orchestrator + `scanners.rs` + `walk.rs`.
- Producer wiring for `bundle_manifests`, `external_versioner`,
  `version_field_spec`, `cascade_from`, `visibility`, `satellites` —
  the resolver populates them and the manifest writer emits typed
  values instead of empty defaults.
- Dashboard `/repositories/$repoId` route with 5 tabs (Release Units,
  Cascade Graph, Drift, Explain, Automations) plus 8 visualizer
  components (`ReleaseUnitCard`, `BundleBadge`, `ExternalVersionerBadge`,
  `CascadeArrow`, `DriftWarning`, `SatelliteList`, `TagFormatPreview`,
  `ManifestFileList`).

### Final-polish refactors

Identifier sweep so `git grep -i project` returns only legitimate hits
(csproj XML `<Project>`, pypa PEP-621 `[project]`, third-party
`directories::ProjectDirs`, ADR 0001 historical doc). Every internal
type/field/method got the rename:

- Wire/config: `BumpDecision.project` → `release_unit`,
  `BumpSourceConfig.project` → `release_unit`. CLI flag
  `--project` → `--release-unit` (short `-p` kept).
- Public types: `SelectedProject` → `SelectedReleaseUnit`,
  `ProjectSelection` → `ReleaseUnitSelection`, `DetectedProject` →
  `DetectedUnit`, `ProjectRelease` alias → `ReleaseEntry`.
- Wizard: `WizardStep::ProjectSelection`/`ProjectConfig` →
  `UnitSelection`/`UnitConfig`, `BackRef::Project` → `Standalone`,
  `state.projects` → `state.standalone_units`.
- Graph: `GraphQueryBuilder.project_type` → `ecosystem_filter`,
  `fn project_type()` → `fn ecosystem()`.
- Locals: 248 `let proj` / `proj.x` / `proj_id` / `proj_idx` /
  `proj_builder` → `let unit` / `unit.*`.
- Codegen filename: `manifest_v2_codegen.rs` → `manifest_v3_codegen.rs`
  (the "one cycle" workaround from Wave 2 retired).
- Test file: `tests/test_manifest_v2.rs` → `test_manifest_v3.rs`.

Drift telemetry runtime-aware: `report_drift_telemetry` now detects
an existing tokio runtime via `Handle::try_current()` and bounces work
onto a dedicated thread instead of nesting runtimes. Future async-context
callers (e.g. wrapping `belaf prepare` from a Tokio main) can no longer
trigger the "Cannot start a runtime from within a runtime" panic.

### Cross-repo

- github-app deploys `feat/3.0` together with this CLI release. Mixed
  versions (CLI 3.0 + github-app 2.x, or vice versa) are not
  supported.
- Drizzle migrations `0003_drop_projects_tier_v3.sql` (drops the
  `projects` table; per the user's audit no production data exists),
  `0004_release_unit_snapshot.sql` (releases.unit_snapshot jsonb), and
  `0005_repository_drift_state.sql` (repositories.last_drift_paths
  text[] + last_drift_at timestamp) run on github-app deploy.

## [2.1.1](https://github.com/ilblu/belaf/compare/v2.1.0...v2.1.1) (2026-05-01)

UX patch for the init wizard.

### Features

* **`DetectorReviewStep` now supports per-item selection.** Previous
  behaviour was all-or-nothing (`Enter` accepted everything,
  `s`/`n` skipped everything). Now you navigate with `↑↓` / `j k`,
  toggle individual items with `Space`, mass-select with `a` /
  deselect with `n`, then `Enter` accepts only the selected set.
  Toggled-OFF detector hits get **no** `[[release_unit]]` block AND
  land in `[ignore_paths]` — silences drift on subsequent
  `belaf prepare` runs without you having to hand-edit the config.
  Mobile-app rows render with a `—` indicator (not togglable; they
  always go to `[allow_uncovered]`).
* **`auto_detect::run_filtered(repo, exclusions)`** new public helper.
  Existing `auto_detect::run` delegates with an empty exclusion set,
  preserving the `--ci --auto-detect` contract. Glob behaviour
  preserved: a glob group with ≥2 non-excluded members still becomes
  one `[[release_unit_glob]]` block; reduced to a single member, it
  falls through to the explicit-block path.

### Fixes

* **`q` quit shortcut works in every wizard step.** The hint lines
  promised `q quit` but only `WelcomeStep` and `SingleMobileStep`
  actually wired it. `PresetSelectionStep`, `ProjectSelectionStep`,
  `TagFormatStep`, `UpstreamConfigStep`, `ConfirmationStep` and
  `DetectorReviewStep` now all accept `q` as a cancellation. In
  `UpstreamConfigStep`, `q` is a literal character while the URL
  field is in input-active mode — the typing guard precedes the
  quit arm so the muscle memory still works for typed URLs that
  contain `q`.

---

## [2.1.0](https://github.com/ilblu/belaf/compare/v2.0.1...v2.1.0) (2026-05-01)

belaf 2.1 — the **ReleaseUnit primitive** plus a modular Step-trait
wizard refactor. ReleaseUnit lets a single config block atomically
claim a directory across heterogeneous file shapes (a hexagonal Rust
service with an `api/` satellite, a Tauri triplet that bumps three
manifests in lockstep, a JVM library whose version lives in
`gradle.properties`, etc.) without per-shape ad-hoc plumbing.

### Features

* **`[[release_unit]]` and `[[release_unit_glob]]` config blocks**
  bundle one or more manifests + satellites into a single release unit.
  Five `version_field` types ship: `cargo_toml`, `npm_package_json`,
  `tauri_conf_json`, `gradle_properties`, `generic_regex`. Plus an
  `external` source for buf / gradle plugin / fastlane / custom-script
  shell-out via `read_command` + `write_command`.
* **Cascade rules** — `cascade_from = { source = "schema", bump = "floor_minor" }`
  on a downstream unit propagates its source's bump per a strategy
  (`mirror` / `floor_patch` / `floor_minor` / `floor_major`). Cycles
  rejected at config-load time via `petgraph::algo::tarjan_scc`.
* **Auto-detectors** — Phase F: hexagonal cargo, Tauri (single-source +
  legacy multi-file), JVM library (gradle.properties /
  build.gradle.kts literal / plugin-managed), mobile-app (warning only),
  single-mobile-repo, nested npm workspace, SDK cascade member,
  single-project. Surfaced in the wizard's new **DetectorReviewStep**
  for review before append.
* **Wizard refactor** — the previous 1340-LOC monolithic wizard is
  split into a `Step` trait + `Vec<Box<dyn Step>>` orchestrator.
  Adding a new screen now means adding one file. Three new steps
  ship: DetectorReviewStep (Phase I.1), TagFormatStep for
  single-project repos (Phase I.3), SingleMobileStep that exits with
  a Bitrise/fastlane/Codemagic suggestion when belaf isn't the right
  fit (Phase I.4). Snapshot harness via `ratatui::backend::TestBackend`
  + insta pins every render.
* **Drift detection always-on** in `belaf prepare` (Phase H).
  Detected bundles that aren't claimed by any `[[release_unit]]`,
  `[ignore_paths]`, or `[allow_uncovered]` abort the prepare run with
  the §3.9 actionable error message — non-bypassable safety net.
* **Cargo.lock auto-refresh** in CargoRewriter (Phase J): per-crate
  `cargo update -p` with workspace fallback, 120s timeout via
  `wait_timeout`, best-effort under Bazel-managed lockfiles.
* **`belaf explain`** subcommand renders attribution per ReleaseUnit
  (origin: explicit / glob / detected, source manifests, satellites,
  tag_format, cascade rule). Useful for debugging unexpected configs.
* **`init --auto-detect` + `--force`** lets CI re-emit detector
  snippets idempotently — the auto-detect marker comment at the head
  of every snippet keeps re-runs from duplicate-appending.

### Fixes

* **Drift coverage** is now bidirectional. Hexagonal-cargo services
  with the canonical `satellites = ["{path}/crates"]` shape have
  their satellite *deeper* than the detector hit (the service dir
  itself); the previous one-way `is_covered` made every such service
  drift on every `belaf prepare`.
* **Auto-detect snippet emission is byte-deterministic** across runs
  (DoD #7). Two latent non-determinism bugs fixed in
  `cmd::init::auto_detect.rs`: HashMap iteration order is no longer
  observable, and the hexagonal-cargo glob picks its `manifests`
  primary by majority-vote (Bin > Lib > Workers > BaseName tie-break)
  instead of from an arbitrary first match — clikd-shape with mixed
  bin/workers services no longer emits a glob that fails to resolve.
* **POSIX shell-quote escape** on every `{version}` / `{bump}` /
  `{name}` substitution in `external_versioner` write commands. A
  malicious version string from a tag can no longer break out and
  execute arbitrary shell. Two regression tests cover the standard
  injection patterns (`1.0.0; rm -rf /` and the embedded-quote
  escape attack).
* **TOML basic-string escape** on every path / name / template
  substituted into `belaf/config.toml` snippets emitted by
  `auto_detect` and the wizard's tag-format builder. A directory or
  project name containing `"` or `]]` can no longer structurally
  inject into the config.
* **Memory leak** removed from `auto_detect`'s `BaseName` branch
  (previous code did `.to_string().leak()` to coerce to `&'static
  str`, which permanently leaked memory on every `belaf init` /
  `prepare`).

### Performance

* Tauri-detector regexes compiled once via `std::sync::LazyLock`
  instead of per-invocation.
* `apply_cascades` builds the cascade graph once instead of twice
  (cycle check + topo walk shared one graph).
* `DetectionReport` cached on `AppSession` so the wizard and the
  drift check don't both walk the filesystem.
* `cargo update -p` now bounded by a 120s wall-clock timeout — an
  offline / unreachable-registry CI can no longer hang `belaf
  prepare` indefinitely.

### Reliability

* Regex `.unwrap()` on capture groups in `version_field/{tauri_conf,
  generic_regex, gradle_properties}` replaced with typed
  `VersionFieldError::VersionFieldMissing` errors — no more
  unreachable panics.
* `Repository::open(path)` documents its assumed defaults (`origin`,
  cache sizes 512/3); new `Repository::open_with(path, upstream,
  AnalysisConfig)` for callers needing overrides.
* `#[serde(deny_unknown_fields)]` on `ExplicitReleaseUnitConfig` and
  `ManifestFileConfig` so a typo like `versoin_field` or `tag_formet`
  surfaces at config-load time instead of being silently dropped.
* `append_to_config` idempotency now anchored to a stable marker
  comment instead of fragile content-equality matching.

### Tests

* 388 unit tests, all green.
* New integration suites:
  `test_drift_detection`, `test_cargo_lock_update`, `test_clikd_shape`,
  `test_clikd_synthetic_commits`, `test_fixture_smoke`,
  `test_explain_clikd`, `test_ci_determinism`,
  `test_tokio_single_end_to_end`, `test_prepare_manifest_emission`.
* Reusable fixtures under `tests/fixtures.rs`: clikd-shape (full
  polyglot), lerna-fixed, tokio-single, cargo-monorepo-independent,
  polyglot-cross-eco-group, kotlin-library-only, ios-only.
* Real-repo dogfood: `scripts/smoke-clikd.sh` runs every read-only
  smoke command (`init --ci --auto-detect`, `status`, `explain`,
  `graph`, `prepare --ci`) against a `git clone` of any source repo
  pointed at via `BELAF_TEST_CLIKD_PATH`. Original is never touched.

---

## [2.0.0](https://github.com/ilblu/belaf/compare/v1.3.6...v2.0.0) (2026-04-27)

belaf 2.0 — architectural refactor focused on additive evolution. The
goal is that future ecosystem additions, bump-type extensions, and
metadata fields can ship as one-line changes without a schema bump or a
DB migration.

### ⚠ BREAKING CHANGES

* **manifest schema**: bumped from `1.2` to `2.0`. The wire format is
  reshaped (UUID v7 `manifest_id` filename, top-level `groups[]`,
  mandatory `tag_name`, `x` extension namespace on every object,
  `is_prerelease` instead of the old `prerelease` field). Producers
  before 2.0 cannot be read by 2.0 consumers and vice versa. Migration:
  drop the old `belaf/releases/*.json` files (no production data was on
  the line — pre-2.0 had no public users).
* **EcosystemType enum removed**. `core::ecosystem::types` is gone;
  consumers now go through `Ecosystem` (the wire-format discriminated
  union, `wire::known::Ecosystem`) for identity and through the
  `Ecosystem` trait + `EcosystemRegistry` for behaviour. Adding a
  language no longer touches a closed enum.
* **manifest field `prefix` removed.** The CLI is responsible for the
  full `tag_name` (now mandatory). Pre-2.0 manifests with `prefix` and
  no `tag_name` won't parse.
* **`belaf prepare` produces ecosystem-aware tag names by default**.
  Maven projects now tag as `<groupId>/<artifactId>@v<version>` (slash,
  not colon) so `git check-ref-format` accepts them. Existing scripts
  that assumed the v1 default `<prefix>v<version>` shape need updating.

### Features

* **schema-as-source-of-truth**: `belaf/schemas/manifest.v2.0.schema.json`
  is the canonical wire format. Rust types are generated by `typify` in
  `build.rs`; the github-app vendors a copy. Drift between producer and
  consumer is now a compile error.
* **strict + extensible JSON schema**: every object has
  `additionalProperties: false` plus an explicit `x` object for
  forward-compatible vendor extensions (OpenAPI `x-*` pattern). New
  fields land via either schema bump (rare) or experimental escape
  through `x` (validated structure, undefined content).
* **discriminated unions** for `ecosystem`, `bump_type`,
  `release_status`: wire-format strings carry no `enum` constraint so
  new values are forward-compatible. Consumers dispatch via
  `Known(...) | Unknown(string)` with a hand-maintained whitelist
  (`KNOWN_ECOSYSTEMS`, etc.) — adding `gradle` is one line, no schema
  edit needed.
* **`Ecosystem` trait + `EcosystemRegistry`** replaces the v1.x closed
  `EcosystemType` enum. Adding a new language is one new
  `impl Ecosystem for FooLoader` block plus one `register(...)` call;
  the registry's default set covers cargo, npm, pypa, go, elixir,
  swift, csproj, and the new maven loader. (Side effect: fixes a v1.x
  bug where Swift was missing from `EcosystemType::from_qname` despite
  `swift.rs` existing — Swift is now structurally present in the
  registry, with a regression test.)
* **`maven` ecosystem**: full multi-module support, parent-POM cycle
  detection via Tarjan-SCC, CI-friendly properties (`${revision}`,
  `${sha1}`, `${changelist}`, `${project.version}`),
  `<dependencyManagement>` resolution. Read+write via `quick_xml`
  event-streaming so whitespace and comments stay byte-stable across
  rewrite. Out of scope: `-D` system properties, env-var resolution,
  profiles, `settings.xml`. Unsupported property names in `<version>`
  fields hard-error with the supported set listed.
* **groups as a first-class manifest concept**: `[[group]]` in
  `belaf/config.toml` bundles projects that must release together
  (canonical: a GraphQL schema published as both an npm package and a
  Maven artifact). The manifest emits `groups[]` plus a `group_id` on
  every member's release entry. Validated at session-init: invalid id
  pattern, unknown member name, overlapping membership all hard-error
  before any release work.
* **group atomicity**: every member of a group must end up with the
  same resolved bump. The wizard auto-syncs sibling bumps when one
  member's bump is edited; `belaf prepare --ci` validates at finalize
  and rejects conflicting `--project` flags with a diagnostic naming
  every conflicting member.
* **external bump-source plumbing** (`--bump-source <FILE|->`,
  `--bump-source-cmd <CMD>`, `[[bump_source]]` in config). Consumes a
  `version: 1` JSON envelope of explicit bump decisions
  (`{ project, bump, reason, source }`), so signals living outside git
  history (GraphQL schema diffs, dep audits) can drive belaf without
  forking the bump logic. 60s default timeout, configurable per
  `[[bump_source]]`. stderr drained line-by-line into tracing INFO.
  Non-zero exit or timeout = hard error, no silent fallback.
  Precedence: conv-commits → `[[bump_source]]` → `--bump-source*` CLI →
  `--project name:bump`.
* **per-ecosystem tag formats with project + group overrides**.
  Defaults driven off the `Ecosystem` trait
  (`tag_format_default()`, `tag_template_vars()`). Variables: `{name}`,
  `{version}`, `{ecosystem}` everywhere; `{groupId}`, `{artifactId}`
  for Maven; `{module}` for Go. Per-project override:
  `[projects."<name>".tag_format]`. Per-group override:
  `[group.<id>].tag_format`. Two layers of validation: ecosystem
  variable whitelist + `git check-ref-format --allow-onelevel` on the
  resolved string.
* **stability suffixes**: `_v1alpha*` / `_v1beta*` naming convention
  documented in the schema for unstable subsystem fields (Kubernetes
  pattern). Mature fields move to unsuffixed names without a schema
  bump.
* **pluggable post-release hooks** — server-side only. Hook
  configuration (event filter, target URL, signing-secret reference)
  lives in the github-app's database and is managed through the
  dashboard, **not** through `belaf/config.toml`. Hooks fire after a
  manifest is processed by the GitHub App and the CLI is uninvolved at
  that point — putting hook config in the user-repo would just be a
  redundant copy of state the App already owns. The CLI's contribution
  is shaping the manifest so the App can drive atomic, group-aware
  delivery (`group_id`, `manifest_id`, `tag_name`, `is_prerelease`
  fields are all consumed by the hook event payload).

### Code Refactoring

* `Project::ecosystem` now flows through `wire::known::Ecosystem`; a
  pre-existing bug in `workflow.rs` that was writing
  `display_name()` ("Rust (Cargo)") into the manifest's `ecosystem`
  field instead of the wire string ("cargo") is fixed as a side effect.
* Loaders keep their inherent helpers (`record_path` / `into_projects`)
  so unit tests don't need a real `Repository`/`ProjectGraphBuilder`.
  The `Ecosystem` trait impl is a thin shim over them.
* `ProjectGraph` owns a sibling `GroupSet` collection. The plan's
  enum'd `GraphNode { Project, Group }` is **not** implemented here —
  groups affect bump-propagation + manifest rendering only, leaving
  toposort/cycle-detection on the existing `petgraph::DiGraph<ProjectId, ()>`.
  Going to a unified node type is purely additive later (the wire
  format already speaks `groups[]` + `group_id`).

### Tests

* New: `tests/test_groups.rs` (5 cases — id propagation,
  empty-groups omission, invalid id pattern, unknown member,
  conflicting `--project` overrides within a group).
* New: `tests/test_maven.rs` (8 cases — single-module, revision
  property, unsupported property, multi-module aggregator, parent
  cycle, comment preservation, `/`-not-`:` tag default, invalid
  template variable for ecosystem).
* New: `tests/test_bump_source.rs` (7 cases — file overrides
  conv-commits, `--project` beats `--bump-source`, `--bump-source-cmd`
  subprocess, malformed JSON, unsupported version, unknown project,
  `[[bump_source]]` config-default).
* New unit-test modules: `core::ecosystem::registry`,
  `core::ecosystem::maven` (parser/resolver/rewriter), `core::group`,
  `core::bump_source` (parser + subprocess runner with timeout +
  exit-code coverage), `core::tag_format`, `core::wire::known`,
  `core::wire::domain`.

### Migration notes

* Drop your old `belaf/releases/*.json` files; the new schema is
  incompatible.
* If you scripted around the v1 tag default, switch to the new
  ecosystem default or add a `[projects."<name>".tag_format]` override.
* If you used a GitHub App build before 2.0, deploy the matching
  github-app 2.0 release before merging the first 2.0 manifest PR —
  pre-2.0 consumers can't parse the new wire format.

## [1.3.6](https://github.com/ilblu/belaf/compare/v1.3.5...v1.3.6) (2026-04-27)


### Bug Fixes

* **pypa:** detect PEP 621 `[project]` table in `pyproject.toml`. Previously belaf only looked at `[tool.belaf]` and the legacy `setup.cfg`/`setup.py` markers, which forced users on modern Python layouts (uv, hatch, poetry, plain setuptools >=61) to duplicate `name` and `version` in `[tool.belaf]` even though they were already in `[project]`. Now `[project] name` and `[project] version` are picked up natively; `[tool.belaf]` remains as an explicit override. ([commit](https://github.com/ilblu/belaf/commit/HEAD))
* **pypa:** add `PyProjectVersionRewriter` to write back `[project] version = "..."` in `pyproject.toml` for PEP 621-only projects (no setup.py, no setup.cfg). Fixes `failed to open file '...setup.py' for reading` during `belaf prepare` on modern Python projects. ([commit](https://github.com/ilblu/belaf/commit/HEAD))


### Tests

* **pypa:** integration tests for two new scenarios — PEP 621-only projects and `[tool.belaf] name` overriding `[project] name`. ([commit](https://github.com/ilblu/belaf/commit/HEAD))


### Build System

* **release:** bump cargo-dist 0.30.3 → 0.31.0 and harden the host job's `gh release upload` step. The previous releases (1.3.4, 1.3.5) needed a manual `gh release create` + `--failed` rerun because GitHub's release backend wasn't always queryable immediately after `dist host` reported success ([cli/cli#6599](https://github.com/cli/cli/issues/6599)). The step now (a) ensures the release exists with the dist-manifest body if missing and (b) retries `gh release upload` 5× with backoff. ([commit](https://github.com/ilblu/belaf/commit/HEAD))


## [1.3.5](https://github.com/ilblu/belaf/compare/v1.3.4...v1.3.5) (2026-04-27)


### Bug Fixes

* **errors:** error output now shows the full caused-by chain and surfaces actionable hints instead of dropping the real reason. Previously, running `belaf init` in a dirty repo printed only `Error: could not initialize app and project graph` while the actual cause (`refusing to proceed (use --force to override)`) was discarded. Migrated to [`annotate-snippets`](https://github.com/rust-lang/annotate-snippets-rs) (the same renderer rustc uses), so errors now look like:

  ```
  error: refusing to proceed
    |
  help: pass `--force` to override, or commit/stash your changes first
  ```

  Plus context-aware hints for typed errors (`ApiError::RateLimited` shows the retry duration; `ApiError::Unauthorized` suggests `belaf install`; `DirtyRepositoryError` suggests `--force`; `BareRepositoryError` explains the working-tree requirement). ([5d766db](https://github.com/ilblu/belaf/commit/5d766db))


### Code Refactoring

* **errors:** consolidate the user-facing error renderer in `core::errors::display_diagnostic`. Removed the parallel dead `errors::report` path and 9 redundant `atry!` wraps (`could not initialize app and project graph` etc.) where the inner error was already specific. Inner errors now speak directly. ([5d766db](https://github.com/ilblu/belaf/commit/5d766db))
* **deps:** add `annotate-snippets = "0.12"` for diagnostic rendering. Honors `NO_COLOR` env, `--no-color` flag, and stderr-is-a-TTY for color decisions. ([5d766db](https://github.com/ilblu/belaf/commit/5d766db))


### Tests

* **diagnostic:** add `tests/test_diagnostic.rs` with insta snapshot tests pinning the rendered output for 6 representative error cases (dirty repo, bare repo, rate-limit, unauthorized, annotated notes, plain error). Format changes are now reviewed via committed `.snap` files in PRs. ([5d766db](https://github.com/ilblu/belaf/commit/5d766db))


## [1.3.4](https://github.com/ilblu/belaf/compare/v1.3.3...v1.3.4) (2026-04-27)


### Bug Fixes

* **install:** `belaf install` no longer fails with "API did not provide installation URL" on uninstalled repos. Root cause: the API was emitting `installUrl` (camelCase) while the CLI deserialised `install_url` (snake_case); the missing field defaulted to `None`. The `/api/cli/*` contract is now schema-first with snake_case enforced at the type level on both sides. ([2ac07cc](https://github.com/ilblu/belaf/commit/2ac07cc))
* **api:** `belaf install` now receives `username` correctly (was emitted as `githubUsername` — a second silent drift in the same shape). ([2ac07cc](https://github.com/ilblu/belaf/commit/2ac07cc))


### Code Refactoring

* **api:** Replace hand-written wire types with `progenitor`-generated Rust types from a committed OpenAPI spec (`api-spec/openapi.cli.json`). Future drift between the belaf API and CLI surfaces as a Rust compile error instead of a silent `serde(default)` miss. ([2ac07cc](https://github.com/ilblu/belaf/commit/2ac07cc))
* **deps:** Drop unused `octocrab` (0% used in `src/`) and `core/root.rs` (duplicated `version_check.rs`). All GitHub data flows exclusively through the belaf API now. ([2ac07cc](https://github.com/ilblu/belaf/commit/2ac07cc))


### Security

* **deps:** Resolve all open `cargo audit` advisories. Bump `git2 0.20.2 → 0.20.4` (RUSTSEC-2026-0008), `ratatui 0.29 → 0.30` (drops indirect `lru 0.12.5`, RUSTSEC-2026-0002), `indicatif 0.17 → 0.18` (drops `number_prefix`, RUSTSEC-2025-0119). Transitive bumps via `cargo update`: `lru → 0.16.4`, `rand → 0.9.4 / 0.8.6` (RUSTSEC-2026-0097), `time → 0.3.47+` (RUSTSEC-2026-0009), `rustls-webpki → 0.103.13` (RUSTSEC-2026-0049/-0098/-0099/-0104). ([98f3eee](https://github.com/ilblu/belaf/commit/98f3eee))
* **deps:** Drop unused `http-cache-reqwest`, `reqwest-middleware`, and `cacache` (never imported in `src/`) to close the last unmaintained `bincode 1.3.3` warning (RUSTSEC-2025-0141). ([98f3eee](https://github.com/ilblu/belaf/commit/98f3eee))


## [1.3.3](https://github.com/ilblu/belaf/compare/v1.3.2...v1.3.3) (2025-12-31)


### Bug Fixes

* **api:** Improve error messages for API failures. Display actual messages from the API instead of generic "failed to create pull request" or "failed to get git credentials" — e.g. so users see "PR already exists" or "authentication expired" directly. ([e4c5960](https://github.com/ilblu/belaf/commit/e4c5960))


## [1.3.2](https://github.com/ilblu/belaf/compare/v1.3.1...v1.3.2) (2025-12-29)


### Bug Fixes

* **graph:** show dynamic project name in web view and update docs ([3a8888e](https://github.com/ilblu/belaf/commit/3a8888ec9a54079b362e138dcfee2a7cbf3ad586))

## [1.3.1](https://github.com/ilblu/belaf/compare/v1.3.0...v1.3.1) (2025-12-27)

# [1.3.0](https://github.com/ilblu/belaf/compare/v1.2.0...v1.3.0) (2025-12-24)


### Features

* **cli:** add belaf install command with API authentication ([#13](https://github.com/ilblu/belaf/issues/13)) ([146be3e](https://github.com/ilblu/belaf/commit/146be3ecfbf26dfcb2f23496db0a938208cce1ff))

# [1.2.0](https://github.com/ilblu/belaf/compare/v1.1.4...v1.2.0) (2025-12-22)


### Features

* **cli:** add LazyVim-style dashboard when running without arguments ([ca159a8](https://github.com/ilblu/belaf/commit/ca159a8e14ff70e516649385367bdc253ab58fc3))

## [1.1.4](https://github.com/ilblu/belaf/compare/v1.1.3...v1.1.4) (2025-12-22)


### Bug Fixes

* **cli:** unify version output for 'belaf version' and 'belaf --version' ([2c2bf7e](https://github.com/ilblu/belaf/commit/2c2bf7edae86db451eb2aa93f4e0d584554955ec))
* **manifest:** validate previous_tag exists before setting compare_url ([770fb39](https://github.com/ilblu/belaf/commit/770fb39e2860b21283da1d3d0600c3be6a503481))

## [1.1.3](https://github.com/ilblu/belaf/compare/v1.1.2...v1.1.3) (2025-12-22)

## [1.1.2](https://github.com/ilblu/belaf/compare/v1.1.1...v1.1.2) (2025-12-19)


### Bug Fixes

* correct command references and fix release workflow ([bd8f046](https://github.com/ilblu/belaf/commit/bd8f0465dd769786ba76e2e4862a0c5b868983ca))

## [1.1.1](https://github.com/ilblu/belaf/compare/v1.1.0...v1.1.1) (2025-12-19)


### Bug Fixes

* **auth:** update GitHub OAuth client ID to belaf app ([ae78426](https://github.com/ilblu/belaf/commit/ae7842633defcc2de0b1bc30eb593775d637bc6c))

# [1.1.0](https://github.com/ilblu/belaf/compare/v1.0.0...v1.1.0) (2025-12-19)


### Features

* **cli:** improve version output and add install success message ([8612d08](https://github.com/ilblu/belaf/commit/8612d08aec774c2f6ecd72a0827efbd8f45e6b74))

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
