//! Partial-override `[release_unit.<name>]` blocks: omit `ecosystem`
//! to inherit it (and `manifests`/`source`) from the auto-detected
//! unit with the same name. Ecosystem-agnostic — tested for cargo,
//! npm, pypa.

mod common;

use belaf::core::config::NamedReleaseUnitConfig;
use belaf::core::ecosystem::format_handler::{FormatHandlerRegistry, WorkspaceDiscovererRegistry};
use belaf::core::git::repository::Repository;
use belaf::core::release_unit::discovery::discover_implicit_release_units;
use belaf::core::release_unit::resolver::{resolve, resolve_partial_against_discovered};
use belaf::core::release_unit::syntax::{CascadeRuleConfig, ReleaseUnitConfig};
use belaf::core::release_unit::ResolveOrigin;
use common::TestRepo;

fn open_repo(t: &TestRepo) -> Repository {
    Repository::open(&t.path).expect("Repository::open must succeed")
}

fn partial(name: &str, mutate: impl FnOnce(&mut ReleaseUnitConfig)) -> NamedReleaseUnitConfig {
    let mut config = ReleaseUnitConfig::default();
    mutate(&mut config);
    NamedReleaseUnitConfig {
        name: name.to_string(),
        config,
    }
}

fn discover(repo: &Repository) -> Vec<belaf::core::ecosystem::format_handler::DiscoveredUnit> {
    let handlers = FormatHandlerRegistry::with_defaults();
    let discoverers = WorkspaceDiscovererRegistry::with_defaults();
    discover_implicit_release_units(repo, &handlers, &discoverers, &[]).expect("discover")
}

// ---------------------------------------------------------------------------
// Cargo: tag_format-only override on an auto-detected cargo crate.
// ---------------------------------------------------------------------------

#[test]
fn cargo_tag_format_only_override() {
    let repo = TestRepo::new();
    repo.write_file(
        "Cargo.toml",
        "[package]\nname = \"my-crate\"\nversion = \"1.0.0\"\nedition = \"2021\"\n",
    );
    repo.commit("seed cargo crate");

    let r = open_repo(&repo);
    let discovered = discover(&r);
    assert!(
        discovered.iter().any(|d| d.qnames[0] == "my-crate"),
        "auto-detect must find my-crate"
    );

    let blk = partial("my-crate", |c| {
        c.tag_format = Some("v{version}".into());
    });
    let out = resolve(&r, &[blk]).expect("resolver must succeed");
    assert!(out.resolved.is_empty(), "no fully-resolved units expected");
    assert_eq!(out.partial_overrides.len(), 1);

    let merged = resolve_partial_against_discovered(&out.partial_overrides, &discovered)
        .expect("partial-override merge must succeed");

    assert_eq!(merged.len(), 1);
    let r = &merged[0];
    assert_eq!(r.unit.name, "my-crate");
    assert_eq!(r.unit.ecosystem.as_str(), "cargo", "ecosystem inherited");
    assert_eq!(r.unit.tag_format.as_deref(), Some("v{version}"));
    assert!(matches!(r.origin, ResolveOrigin::PartialOverride { .. }));
}

// ---------------------------------------------------------------------------
// NPM: same shape on a package.json.
// ---------------------------------------------------------------------------

#[test]
fn npm_tag_format_only_override() {
    let repo = TestRepo::new();
    repo.write_file(
        "package.json",
        "{\n  \"name\": \"my-pkg\",\n  \"version\": \"1.0.0\"\n}\n",
    );
    repo.commit("seed npm pkg");

    let r = open_repo(&repo);
    let discovered = discover(&r);
    assert!(discovered.iter().any(|d| d.qnames[0] == "my-pkg"));

    let blk = partial("my-pkg", |c| {
        c.tag_format = Some("release-{version}".into());
    });
    let out = resolve(&r, &[blk]).expect("ok");
    let merged = resolve_partial_against_discovered(&out.partial_overrides, &discovered).unwrap();

    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].unit.ecosystem.as_str(), "npm");
    assert_eq!(
        merged[0].unit.tag_format.as_deref(),
        Some("release-{version}")
    );
}

// ---------------------------------------------------------------------------
// Pypa: tag_format override + the new pep_621 default takes effect on the
// synthesized informational manifest.
// ---------------------------------------------------------------------------

#[test]
fn pypa_tag_format_only_override() {
    let repo = TestRepo::new();
    repo.write_file(
        "pyproject.toml",
        "[project]\nname = \"discord-bot\"\nversion = \"0.4.0\"\n",
    );
    repo.commit("seed pypa pkg");

    let r = open_repo(&repo);
    let discovered = discover(&r);
    assert!(
        discovered.iter().any(|d| d.qnames[0] == "discord-bot"),
        "auto-detect must find discord-bot from pyproject.toml"
    );

    let blk = partial("discord-bot", |c| {
        c.tag_format = Some("v{version}".into());
    });
    let out = resolve(&r, &[blk]).expect("ok");
    let merged = resolve_partial_against_discovered(&out.partial_overrides, &discovered).unwrap();

    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].unit.name, "discord-bot");
    assert_eq!(merged[0].unit.ecosystem.as_str(), "pypa");
    assert_eq!(merged[0].unit.tag_format.as_deref(), Some("v{version}"));
}

// ---------------------------------------------------------------------------
// Error: partial block names a unit that auto-detect did not find.
// ---------------------------------------------------------------------------

#[test]
fn partial_no_match_errors_clearly() {
    let repo = TestRepo::new();
    repo.write_file(
        "Cargo.toml",
        "[package]\nname = \"actual\"\nversion = \"1.0.0\"\nedition = \"2021\"\n",
    );
    repo.commit("seed");

    let r = open_repo(&repo);
    let discovered = discover(&r);

    let blk = partial("typo-name", |c| {
        c.tag_format = Some("v{version}".into());
    });
    let out = resolve(&r, &[blk]).expect("ok");
    assert_eq!(out.partial_overrides.len(), 1);

    let err = resolve_partial_against_discovered(&out.partial_overrides, &discovered).unwrap_err();
    assert_eq!(err.rule(), "partial_override_no_match");
}

// ---------------------------------------------------------------------------
// Error: partial block sets a structural field.
// ---------------------------------------------------------------------------

#[test]
fn partial_with_structural_field_errors() {
    let repo = TestRepo::new();
    repo.write_file(
        "Cargo.toml",
        "[package]\nname = \"x\"\nversion = \"1.0.0\"\nedition = \"2021\"\n",
    );
    repo.commit("seed");

    let r = open_repo(&repo);

    let blk = partial("x", |c| {
        c.tag_format = Some("v{version}".into());
        c.fallback_manifests = vec!["Cargo.toml".into()];
    });
    let err = resolve(&r, &[blk]).unwrap_err();
    assert_eq!(err.rule(), "partial_override_structural_field");
}

// ---------------------------------------------------------------------------
// Error: empty partial block has no overrides.
// ---------------------------------------------------------------------------

#[test]
fn partial_empty_errors() {
    let repo = TestRepo::new();
    repo.write_file(
        "Cargo.toml",
        "[package]\nname = \"x\"\nversion = \"1.0.0\"\nedition = \"2021\"\n",
    );
    repo.commit("seed");

    let r = open_repo(&repo);
    let blk = partial("x", |_| {});
    let err = resolve(&r, &[blk]).unwrap_err();
    assert_eq!(err.rule(), "partial_override_empty");
}

// ---------------------------------------------------------------------------
// Replacive list semantics: explicit satellites replace nothing here
// (auto-detect doesn't seed satellites for a single-package repo), but
// the override still flows through.
// ---------------------------------------------------------------------------

#[test]
fn partial_satellites_flow_through_replacively() {
    let repo = TestRepo::new();
    repo.write_file(
        "Cargo.toml",
        "[package]\nname = \"y\"\nversion = \"1.0.0\"\nedition = \"2021\"\n",
    );
    repo.write_file("docs/api.md", "# api");
    repo.commit("seed");

    let r = open_repo(&repo);
    let discovered = discover(&r);

    let blk = partial("y", |c| {
        c.satellites = vec!["docs".into()];
    });
    let out = resolve(&r, &[blk]).expect("ok");
    let merged = resolve_partial_against_discovered(&out.partial_overrides, &discovered).unwrap();

    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].unit.satellites.len(), 1);
    assert_eq!(merged[0].unit.satellites[0].escaped(), "docs");
}

// ---------------------------------------------------------------------------
// cascade_from override.
// ---------------------------------------------------------------------------

#[test]
fn partial_cascade_from_override() {
    let repo = TestRepo::new();
    repo.write_file(
        "Cargo.toml",
        "[package]\nname = \"sdk\"\nversion = \"1.0.0\"\nedition = \"2021\"\n",
    );
    repo.commit("seed");

    let r = open_repo(&repo);
    let discovered = discover(&r);

    let blk = partial("sdk", |c| {
        c.cascade_from = Some(CascadeRuleConfig {
            source: "schema".into(),
            bump: "floor_minor".into(),
        });
    });
    let out = resolve(&r, &[blk]).expect("ok");
    let merged = resolve_partial_against_discovered(&out.partial_overrides, &discovered).unwrap();

    assert_eq!(merged.len(), 1);
    let cascade = merged[0]
        .unit
        .cascade_from
        .as_ref()
        .expect("cascade_from must be set");
    assert_eq!(cascade.source, "schema");
}
