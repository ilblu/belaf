//! End-to-end validation against the `clikd-shape` polyglot fixture.
//!
//! Pins the detector + drift + auto-detect contract against a real-world
//! repo layout. If any of the heuristics shift, the tests below break and
//! the surface area is reviewable as snapshot diffs.
//!
//! Step 12 of the ReleaseUnit refactor sequence. See
//! `BELAF_MASTER_PLAN.md` for the surrounding plan.

mod common;
mod fixtures;

use std::path::Path;

use belaf::core::git::repository::Repository;
use belaf::core::release_unit::detector::{self, BundleKind, DetectedShape, HintKind};

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

fn seeded_clikd() -> TestRepo {
    let repo = TestRepo::new();
    fixtures::seed_clikd_shape(&repo);
    repo
}

#[test]
fn detector_finds_expected_bundle_kinds() {
    let repo = seeded_clikd();
    let r = open_repo(&repo);
    let report = detector::detect_all(&r);

    let kinds: Vec<&'static str> = report
        .matches
        .iter()
        .map(|m| match &m.shape {
            DetectedShape::Bundle(BundleKind::HexagonalCargo { .. }) => "hexagonal_cargo",
            DetectedShape::Bundle(BundleKind::Tauri { .. }) => "tauri",
            DetectedShape::Bundle(BundleKind::JvmLibrary { .. }) => "jvm_library",
            DetectedShape::ExternallyManaged(_) => "mobile_app",
            DetectedShape::Hint(HintKind::NpmWorkspace) => "nested_npm_workspace",
            DetectedShape::Hint(HintKind::SdkCascade) => "sdk_cascade_member",
            DetectedShape::Hint(HintKind::SingleProject { .. }) => "single_project",
            DetectedShape::Hint(HintKind::NestedMonorepo) => "nested_monorepo",
        })
        .collect();

    // Must hit at least one of each major kind for the fixture to be
    // a useful regression target.
    let has_hex = kinds.contains(&"hexagonal_cargo");
    let has_tauri = kinds.contains(&"tauri");
    let has_jvm = kinds.contains(&"jvm_library");
    let has_mobile = kinds.contains(&"mobile_app");
    let has_sdk = kinds.contains(&"sdk_cascade_member");

    assert!(
        has_hex && has_tauri && has_jvm && has_mobile && has_sdk,
        "clikd-shape must hit all 5 major detector kinds; got: {kinds:?}"
    );
}

#[test]
fn drift_fires_without_coverage() {
    let repo = seeded_clikd();
    let r = open_repo(&repo);
    let report = detector::detect_drift_paths(&r, &[], &[], &[]);
    assert!(
        !report.is_empty(),
        "an unconfigured clikd-shape must drift on every detected bundle"
    );
}

#[test]
fn drift_silenced_by_clikd_canonical_config() {
    // The canonical clikd config: ReleaseUnits cover the cargo
    // services, Tauri, and the JVM SDK; mobile-ios is allow_uncovered.
    let repo = seeded_clikd();
    let r = open_repo(&repo);
    let ignore_paths: Vec<String> = vec![
        "apps/services/aura".into(),
        "apps/services/ekko".into(),
        "apps/services/mondo".into(),
        "apps/desktop".into(),
        "sdks/kotlin".into(),
        "sdks/typescript".into(),
        "sdks/swift".into(),
    ];
    let allow_uncovered: Vec<String> = vec!["apps/mobile-ios/".into()];
    let report = detector::detect_drift_paths(&r, &[], &ignore_paths, &allow_uncovered);
    assert!(
        report.is_empty(),
        "canonical clikd config should silence drift, got: {:#?}",
        report
    );
}

#[test]
fn auto_detect_emits_coherent_snippet() {
    let repo = seeded_clikd();
    let r = open_repo(&repo);
    let result = belaf::cmd::init::auto_detect::run(&r);

    assert!(
        result.toml_snippet.contains("[release_unit."),
        "snippet must contain at least one [release_unit.<name>] block"
    );
    assert!(
        result.toml_snippet.contains("[allow_uncovered]"),
        "snippet must auto-add detected mobile apps to [allow_uncovered]"
    );
    assert!(
        result.toml_snippet.contains("apps/mobile-ios"),
        "snippet must list the iOS path under allow_uncovered"
    );
    assert!(
        result.counters.total_release_unit_candidates() >= 3,
        "expected at least 3 release-unit candidates for clikd-shape, got {}",
        result.counters.total_release_unit_candidates()
    );
    assert!(
        result.counters.mobile_ios >= 1,
        "iOS detector must hit at least once on clikd-shape"
    );
}

#[test]
fn auto_detect_run_filtered_excludes_paths_from_release_units() {
    use std::collections::HashSet;

    use belaf::core::git::repository::RepoPathBuf;

    let repo = seeded_clikd();
    let r = open_repo(&repo);

    // Exclude the JVM SDK and the swift cascade member.
    let mut exclusions: HashSet<RepoPathBuf> = HashSet::new();
    exclusions.insert(RepoPathBuf::new(b"sdks/kotlin"));
    exclusions.insert(RepoPathBuf::new(b"sdks/swift"));

    let result = belaf::cmd::init::auto_detect::run_filtered(&r, &exclusions);

    // Snippet must still contain a glob-form [release_unit.<name>]
    // for apps/services/* (the cargo services aren't excluded).
    assert!(
        result.toml_snippet.contains("glob = \"apps/services/*\""),
        "filtered snippet must still emit the cargo services glob, got:\n{}",
        result.toml_snippet
    );

    // Excluded paths must NOT have a release_unit block.
    assert!(
        !result.toml_snippet.contains("ecosystem = \"jvm-library\""),
        "kotlin SDK was excluded; no jvm-library block expected"
    );

    // [ignore_paths] block must list the exclusions.
    assert!(
        result.toml_snippet.contains("[ignore_paths]"),
        "exclusions must produce an [ignore_paths] block"
    );
    assert!(result.toml_snippet.contains("sdks/kotlin/"));
    assert!(result.toml_snippet.contains("sdks/swift/"));
}
