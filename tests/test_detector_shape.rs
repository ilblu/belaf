//! Per-fixture detector-shape pinning.
//!
//! `detector::detect_all` walks the repo and returns a
//! `DetectionReport { matches: Vec<DetectorMatch> }` with one of three
//! shapes per match: `Bundle(_)`, `Hint(_)`, or `ExternallyManaged(_)`.
//! These tests assert the *classification* — bundle vs hint vs ext —
//! plus the discriminating sub-kind, for every fixture in the test
//! matrix. A regression where a Standalone-hint accidentally
//! re-classifies as a Bundle (or vice versa) gets caught here before
//! it reaches the wizard.
//!
//! Wider snapshot coverage of the rendered view lives in
//! `test_repo_variants.rs`; this file focuses purely on the
//! `DetectedShape` taxonomy.

mod common;
mod fixtures;

use std::path::Path;

use belaf::core::git::repository::Repository;
use belaf::core::release_unit::detector;
use belaf::core::release_unit::shape::{
    BundleKind, DetectedShape, ExtKind, HexagonalPrimary, HintKind, SingleProjectEcosystem,
};
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

fn shapes(seed: fn(&TestRepo)) -> Vec<DetectedShape> {
    let repo = TestRepo::new();
    seed(&repo);
    let r = Repository::open(&repo.path).expect("open repo");
    let report = detector::detect_all(&r);
    report.matches.into_iter().map(|m| m.shape).collect()
}

fn count_bundles(shapes: &[DetectedShape]) -> usize {
    shapes.iter().filter(|s| s.is_bundle()).count()
}

fn count_hints(shapes: &[DetectedShape]) -> usize {
    shapes
        .iter()
        .filter(|s| matches!(s, DetectedShape::Hint(_)))
        .count()
}

fn count_ext(shapes: &[DetectedShape]) -> usize {
    shapes
        .iter()
        .filter(|s| matches!(s, DetectedShape::ExternallyManaged(_)))
        .count()
}

// ---------------------------------------------------------------------------
// Single-package fixtures: hint only, no bundle, no ext.
// ---------------------------------------------------------------------------

#[test]
fn tokio_single_emits_single_project_hint() {
    let s = shapes(fixtures::seed_tokio_single);
    assert_eq!(count_bundles(&s), 0);
    assert_eq!(count_ext(&s), 0);
    assert!(
        s.iter().any(|m| matches!(
            m,
            DetectedShape::Hint(HintKind::SingleProject {
                ecosystem: SingleProjectEcosystem::Cargo
            })
        )),
        "expected SingleProject(cargo) hint; got {s:?}"
    );
}

#[test]
fn lodash_single_emits_single_project_hint_npm() {
    let s = shapes(fixtures::seed_lodash_single);
    assert_eq!(count_bundles(&s), 0);
    assert_eq!(count_ext(&s), 0);
    assert!(
        s.iter().any(|m| matches!(
            m,
            DetectedShape::Hint(HintKind::SingleProject {
                ecosystem: SingleProjectEcosystem::Npm
            })
        )),
        "expected SingleProject(npm) hint; got {s:?}"
    );
}

// ---------------------------------------------------------------------------
// Bundle fixtures: exactly one bundle, no hints/ext for that variant.
// ---------------------------------------------------------------------------

#[test]
fn hexagonal_cargo_emits_one_hexagonal_bundle() {
    let s = shapes(fixtures::seed_hexagonal_cargo_only);
    assert_eq!(count_ext(&s), 0);
    let hex_count = s
        .iter()
        .filter(|m| matches!(m, DetectedShape::Bundle(BundleKind::HexagonalCargo { .. })))
        .count();
    assert_eq!(hex_count, 1, "expected exactly one HexagonalCargo bundle");
    assert!(
        s.iter().any(|m| matches!(
            m,
            DetectedShape::Bundle(BundleKind::HexagonalCargo {
                primary: HexagonalPrimary::Bin | HexagonalPrimary::BaseName
            })
        )),
        "primary should be Bin or BaseName for the aura fixture; got {s:?}"
    );
}

#[test]
fn tauri_app_emits_one_tauri_bundle() {
    let s = shapes(fixtures::seed_tauri_app_only);
    assert_eq!(count_ext(&s), 0);
    let tauri_count = s
        .iter()
        .filter(|m| matches!(m, DetectedShape::Bundle(BundleKind::Tauri { .. })))
        .count();
    assert_eq!(tauri_count, 1, "expected exactly one Tauri bundle");
}

#[test]
fn kotlin_library_emits_jvm_library_bundle() {
    let s = shapes(fixtures::seed_kotlin_library_only);
    let jvm_count = s
        .iter()
        .filter(|m| matches!(m, DetectedShape::Bundle(BundleKind::JvmLibrary { .. })))
        .count();
    assert_eq!(jvm_count, 1, "expected exactly one JvmLibrary bundle");
}

// ---------------------------------------------------------------------------
// Hint-only fixtures.
// ---------------------------------------------------------------------------

#[test]
fn ts_sdk_cascade_emits_sdk_cascade_hint() {
    let s = shapes(fixtures::seed_ts_sdk_cascade);
    assert_eq!(count_bundles(&s), 0);
    assert_eq!(count_ext(&s), 0);
    assert!(
        s.iter()
            .any(|m| matches!(m, DetectedShape::Hint(HintKind::SdkCascade))),
        "expected SdkCascade hint; got {s:?}"
    );
}

#[test]
fn turbo_workspace_emits_npm_workspace_hint() {
    let s = shapes(fixtures::seed_turbo_workspace);
    assert_eq!(count_bundles(&s), 0);
    assert_eq!(count_ext(&s), 0);
    assert!(
        s.iter()
            .any(|m| matches!(m, DetectedShape::Hint(HintKind::NpmWorkspace))),
        "expected NpmWorkspace hint; got {s:?}"
    );
}

#[test]
fn vendored_monorepo_emits_nested_monorepo_hint() {
    let s = shapes(fixtures::seed_vendored_monorepo);
    assert_eq!(count_bundles(&s), 0);
    assert_eq!(count_ext(&s), 0);
    assert!(
        s.iter()
            .any(|m| matches!(m, DetectedShape::Hint(HintKind::NestedMonorepo))),
        "expected NestedMonorepo hint; got {s:?}"
    );
}

// ---------------------------------------------------------------------------
// Externally-managed: mobile-only.
// ---------------------------------------------------------------------------

#[test]
fn ios_only_emits_mobile_ios_ext() {
    let s = shapes(fixtures::seed_ios_only);
    assert_eq!(count_bundles(&s), 0);
    assert!(
        s.iter()
            .any(|m| matches!(m, DetectedShape::ExternallyManaged(ExtKind::MobileIos))),
        "expected ExternallyManaged(MobileIos); got {s:?}"
    );
}

// ---------------------------------------------------------------------------
// Polyglot — exercises every class at once.
// ---------------------------------------------------------------------------

#[test]
fn clikd_shape_polyglot_classification() {
    let s = shapes(fixtures::seed_clikd_shape);
    assert!(
        count_bundles(&s) >= 1,
        "polyglot fixture must surface at least one bundle"
    );
    assert!(
        count_hints(&s) >= 1,
        "polyglot fixture must surface at least one hint"
    );
    assert!(
        count_ext(&s) >= 1,
        "polyglot fixture must surface at least one ExternallyManaged entry"
    );
}
