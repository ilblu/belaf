//! Per-bundle detector + emit roundtrip.
//!
//! Pins each bundle module's `detect` against a minimal fixture and
//! verifies the emit path produces a `[release_unit.X]` snippet that
//! the `release_unit::syntax` loader parses without errors. Catches
//! drift between a bundle's detection-shape and the config grammar
//! the resolver consumes.

mod common;
mod fixtures;

use std::path::Path;

use belaf::core::release_unit::bundle::{hexagonal, jvm_library, tauri};
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

#[test]
fn tauri_detector_finds_one_bundle() {
    let repo = TestRepo::new();
    fixtures::seed_tauri_app_only(&repo);
    let matches = tauri::detect(&repo.path);
    assert_eq!(
        matches.len(),
        1,
        "expected one Tauri bundle; got {matches:?}"
    );
}

#[test]
fn hexagonal_detector_finds_one_bundle() {
    let repo = TestRepo::new();
    fixtures::seed_hexagonal_cargo_only(&repo);
    let matches = hexagonal::detect(&repo.path);
    assert_eq!(
        matches.len(),
        1,
        "expected one HexagonalCargo bundle; got {matches:?}"
    );
}

#[test]
fn jvm_library_detector_finds_one_bundle() {
    let repo = TestRepo::new();
    fixtures::seed_kotlin_library_only(&repo);
    let matches = jvm_library::detect(&repo.path);
    assert_eq!(
        matches.len(),
        1,
        "expected one JvmLibrary bundle; got {matches:?}"
    );
}

#[test]
fn polyglot_fixture_surfaces_each_bundle_kind_at_least_once() {
    let repo = TestRepo::new();
    fixtures::seed_clikd_shape(&repo);
    assert!(
        !tauri::detect(&repo.path).is_empty(),
        "polyglot fixture must have a Tauri bundle"
    );
    assert!(
        !hexagonal::detect(&repo.path).is_empty(),
        "polyglot fixture must have at least one HexagonalCargo bundle"
    );
    assert!(
        !jvm_library::detect(&repo.path).is_empty(),
        "polyglot fixture must have a JvmLibrary bundle"
    );
}
