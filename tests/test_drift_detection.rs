//! Integration tests for the drift detector wired into
//! `belaf prepare`. Each test seeds a synthetic working tree that
//! triggers one of the detector heuristics, then exercises
//! [`detect_drift_paths`] / [`pre_prepare_drift_check_paths`] under
//! the four common scopes:
//!
//! 1. uncovered (no config) → drift fires
//! 2. covered by `[ignore_paths]`           → silent
//! 3. covered by `[allow_uncovered]`        → silent
//! 4. mobile-allow (iOS app explicitly listed in `[allow_uncovered]`)
//!    is the canonical Phase I.5 shape → silent

use std::process::Command;

use belaf::core::git::repository::Repository;
use belaf::core::release_unit::detector;

mod common;
use common::TestRepo;

fn open_repo(repo: &TestRepo) -> Repository {
    Repository::open(&repo.path).expect("open repository")
}

fn seed_ios_app(repo: &TestRepo, dir: &str) {
    let xcodeproj = format!("{dir}/MyApp.xcodeproj/project.pbxproj");
    repo.write_file(&xcodeproj, "// dummy pbxproj\n");
    Command::new("git")
        .args(["add", "-A"])
        .current_dir(&repo.path)
        .output()
        .expect("git add");
}

fn seed_hexagonal_cargo(repo: &TestRepo, dir: &str) {
    repo.write_file(
        &format!("{dir}/crates/bin/Cargo.toml"),
        "[package]\nname = \"alpha\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    repo.write_file(&format!("{dir}/crates/bin/src/main.rs"), "fn main() {}\n");
    Command::new("git")
        .args(["add", "-A"])
        .current_dir(&repo.path)
        .output()
        .expect("git add");
}

#[test]
fn uncovered_ios_app_triggers_drift() {
    let repo = TestRepo::new();
    seed_ios_app(&repo, "ios");

    let r = open_repo(&repo);
    let report = detector::detect_drift_paths(&r, &[], &[], &[]);

    assert!(
        !report.is_empty(),
        "expected drift to fire, got empty: {:#?}",
        report
    );
    assert_eq!(report.uncovered.len(), 1);
    assert_eq!(
        std::path::Path::new(&*report.uncovered[0].path.escaped()),
        std::path::Path::new("ios")
    );
}

#[test]
fn ignore_paths_silences_drift() {
    let repo = TestRepo::new();
    seed_ios_app(&repo, "ios");

    let r = open_repo(&repo);
    let ignore = vec!["ios/".to_string()];
    let report = detector::detect_drift_paths(&r, &[], &ignore, &[]);

    assert!(
        report.is_empty(),
        "expected ignore_paths to silence drift, got: {:#?}",
        report
    );
}

#[test]
fn allow_uncovered_silences_drift() {
    let repo = TestRepo::new();
    seed_ios_app(&repo, "ios");

    let r = open_repo(&repo);
    let allow = vec!["ios/".to_string()];
    let report = detector::detect_drift_paths(&r, &[], &[], &allow);

    assert!(
        report.is_empty(),
        "expected allow_uncovered to silence drift, got: {:#?}",
        report
    );
}

#[test]
fn mobile_allow_canonical_shape() {
    // Phase I.5 — `belaf init --auto-detect` stuffs every detected
    // mobile app into `[allow_uncovered]` so the drift check stays
    // silent on the same paths the auto-detect step recognised.
    let repo = TestRepo::new();
    seed_ios_app(&repo, "apps/mobile-ios");

    let r = open_repo(&repo);
    let allow = vec!["apps/mobile-ios/".to_string()];
    let report = detector::detect_drift_paths(&r, &[], &[], &allow);

    assert!(
        report.is_empty(),
        "mobile-allow shape (auto-detect output) must silence drift, got: {:#?}",
        report
    );
}

#[test]
fn pre_prepare_drift_check_paths_returns_error_message() {
    let repo = TestRepo::new();
    seed_ios_app(&repo, "ios");

    let r = open_repo(&repo);
    let err = detector::pre_prepare_drift_check_paths(&r, &[], &[], &[])
        .expect_err("expected drift check to fail");

    assert!(
        err.contains("uncovered release artifacts"),
        "error message should mention uncovered artifacts, got: {err}"
    );
    assert!(err.contains("ios"), "error should mention the iOS path");
    assert!(
        err.contains("ignore_paths") || err.contains("allow_uncovered"),
        "error should suggest remediation paths, got: {err}"
    );
}

#[test]
fn pre_prepare_drift_check_paths_silent_when_covered() {
    let repo = TestRepo::new();
    seed_ios_app(&repo, "ios");

    let r = open_repo(&repo);
    detector::pre_prepare_drift_check_paths(&r, &[], &["ios/".to_string()], &[])
        .expect("ignore_paths should silence drift");
}

#[test]
fn hexagonal_cargo_drift_can_be_silenced() {
    // Common case: a hexagonal-cargo service exists, was added to
    // `[[release_unit]]`. Its parent dir lands in the resolved
    // unit's coverage list and the drift check is happy.
    let repo = TestRepo::new();
    seed_hexagonal_cargo(&repo, "apps/services/foo");

    let r = open_repo(&repo);
    // No `[ignore_paths]` / no `[allow_uncovered]` — but pretend the
    // resolved release-unit covers the parent dir.
    let report = detector::detect_drift_paths(&r, &[], &["apps/services/foo".to_string()], &[]);
    assert!(
        report.is_empty(),
        "hexagonal-cargo under a coverage path must not drift, got: {:#?}",
        report
    );
}
