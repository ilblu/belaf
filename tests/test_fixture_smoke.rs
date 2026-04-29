//! L.2-L.7 smoke tests. Each spins up the corresponding fixture in
//! a fresh tempdir and asserts the most important invariant for that
//! shape (detector signal, single-mobile flag, project count, etc.).
//!
//! These don't snapshot the full detector report — they pin the one
//! property each shape was added to exercise.

mod common;
mod fixtures;

use std::path::Path;

use belaf::core::git::repository::Repository;
use belaf::core::release_unit::detector::{self, DetectorKind, MobilePlatform};

use common::TestRepo;
use fixtures::Seedable;

impl Seedable for TestRepo {
    fn root(&self) -> &Path {
        &self.path
    }
    fn write_file(&self, relative: &str, content: &str) {
        TestRepo::write_file(self, relative, content);
    }
    fn commit(&self, message: &str) {
        TestRepo::commit(self, message);
    }
}

fn open_repo(t: &TestRepo) -> Repository {
    Repository::open(&t.path).expect("Repository::open")
}

// ---------------------------------------------------------------------------
// L.2 — lerna-fixed
// ---------------------------------------------------------------------------

#[test]
fn lerna_fixed_does_not_trigger_detector_warnings() {
    // Lerna-fixed repos are pure npm — no hexagonal cargo, no tauri,
    // no mobile, no JVM. The detector should be silent; users can
    // still configure a Group manually.
    let repo = TestRepo::new();
    fixtures::seed_lerna_fixed(&repo);

    let r = open_repo(&repo);
    let report = detector::detect_all(&r);

    let any_loud = report.matches.iter().any(|m| {
        !matches!(
            m.kind,
            DetectorKind::NestedNpmWorkspace | DetectorKind::SdkCascadeMember
        )
    });
    assert!(
        !any_loud,
        "lerna-fixed should not trigger any of the loud detector kinds; got: {:#?}",
        report.matches
    );
    assert!(!report.is_single_mobile_repo());
}

// ---------------------------------------------------------------------------
// L.3 — tokio-single (single-project Cargo)
// ---------------------------------------------------------------------------

#[test]
fn tokio_single_has_no_detector_hits() {
    // A flat single-crate Cargo repo doesn't match any detector
    // heuristic (no `crates/{bin,lib,workers}` layout, no Tauri
    // triplet, etc.). The wizard should surface the single-project
    // tag-format prompt instead.
    let repo = TestRepo::new();
    fixtures::seed_tokio_single(&repo);

    let r = open_repo(&repo);
    let report = detector::detect_all(&r);

    assert!(
        report.matches.is_empty(),
        "tokio-single must produce zero detector hits; got: {:#?}",
        report.matches
    );
    assert_eq!(report.count_release_unit_candidates(), 0);
}

// ---------------------------------------------------------------------------
// L.4 — cargo-monorepo-independent
// ---------------------------------------------------------------------------

#[test]
fn cargo_monorepo_independent_no_hexagonal_match() {
    // The hexagonal-cargo detector requires `D/crates/{bin,lib,workers,
    // <basename>}` underneath a service directory. A flat workspace
    // with `crates/alpha` and `crates/beta` has no service-shaped
    // parent — detector should stay quiet.
    let repo = TestRepo::new();
    fixtures::seed_cargo_monorepo_independent(&repo);

    let r = open_repo(&repo);
    let report = detector::detect_all(&r);

    let any_hex = report
        .matches
        .iter()
        .any(|m| matches!(m.kind, DetectorKind::HexagonalCargo { .. }));
    assert!(
        !any_hex,
        "cargo-monorepo-independent must not match hexagonal-cargo; got: {:#?}",
        report.matches
    );
}

// ---------------------------------------------------------------------------
// L.5 — polyglot-cross-eco-group (npm + maven, manual group)
// ---------------------------------------------------------------------------

#[test]
fn polyglot_cross_eco_group_no_detector_hits() {
    // Detectors don't fire on plain npm + plain Maven layouts; the
    // group is configured manually via `[[group]]` in config.toml.
    // This test pins that the fixture's shape itself doesn't trigger
    // any heuristic — anyone reading config.toml later sees a clean,
    // explicit group definition rather than a noisy auto-detect path.
    let repo = TestRepo::new();
    fixtures::seed_polyglot_cross_eco_group(&repo);

    let r = open_repo(&repo);
    let report = detector::detect_all(&r);

    assert!(
        report.matches.is_empty(),
        "polyglot-cross-eco-group should produce zero detector hits; got: {:#?}",
        report.matches
    );
}

// ---------------------------------------------------------------------------
// L.6 — kotlin-library-only
// ---------------------------------------------------------------------------

#[test]
fn kotlin_library_only_hits_jvm_library_detector() {
    let repo = TestRepo::new();
    fixtures::seed_kotlin_library_only(&repo);

    let r = open_repo(&repo);
    let report = detector::detect_all(&r);

    let jvm_hits: Vec<_> = report
        .matches
        .iter()
        .filter(|m| matches!(m.kind, DetectorKind::JvmLibrary { .. }))
        .collect();
    assert!(
        !jvm_hits.is_empty(),
        "kotlin-library-only must hit the JVM library detector; got: {:#?}",
        report.matches
    );

    // Sanity: nothing else should fire.
    let mobile = report
        .matches
        .iter()
        .any(|m| matches!(m.kind, DetectorKind::MobileApp { .. }));
    assert!(!mobile, "kotlin-library-only is NOT a mobile repo");
    assert!(!report.is_single_mobile_repo());
}

// ---------------------------------------------------------------------------
// L.7 — ios-only (single-mobile-repo path)
// ---------------------------------------------------------------------------

#[test]
fn ios_only_triggers_single_mobile_repo() {
    let repo = TestRepo::new();
    fixtures::seed_ios_only(&repo);

    let r = open_repo(&repo);
    let report = detector::detect_all(&r);

    assert!(
        report.is_single_mobile_repo(),
        "ios-only fixture must report is_single_mobile_repo() = true; got: {:#?}",
        report.matches
    );

    let any_ios = report.matches.iter().any(|m| {
        matches!(
            m.kind,
            DetectorKind::MobileApp {
                platform: MobilePlatform::Ios
            }
        )
    });
    assert!(any_ios, "ios-only must hit the iOS detector");
}
