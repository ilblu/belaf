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

#[test]
fn auto_detect_suppresses_hits_under_existing_ignore_paths() {
    use belaf::cmd::init::auto_detect::{self, ExistingDecisions};

    let repo = seeded_clikd();
    let r = open_repo(&repo);

    // An existing config already ignores the kotlin SDK. Scan-level
    // suppression: no block, no re-emitted path, no counter — the path
    // does not exist for belaf.
    let existing = ExistingDecisions {
        ignore_paths: vec!["sdks/kotlin/".into()],
        ..Default::default()
    };
    let result = auto_detect::run_against(&r, &existing);

    assert!(
        !result.toml_snippet.contains("jvm-library"),
        "kotlin SDK is under [ignore_paths]; no jvm-library block expected, got:\n{}",
        result.toml_snippet
    );
    assert!(
        !result.toml_snippet.contains("sdks/kotlin"),
        "an ignored path must not resurface anywhere in the snippet, got:\n{}",
        result.toml_snippet
    );
    assert_eq!(result.counters.jvm_library, 0);
    assert_eq!(
        result.counters.already_covered, 0,
        "[ignore_paths] hits are dropped before counting, not tallied as already_covered"
    );

    // Unrelated detection is unaffected.
    assert!(
        result.toml_snippet.contains("glob = \"apps/services/*\""),
        "cargo services glob must still be emitted"
    );
}

#[test]
fn auto_detect_suppresses_emission_under_existing_allow_uncovered() {
    use belaf::cmd::init::auto_detect::{self, ExistingDecisions};

    let repo = seeded_clikd();
    let r = open_repo(&repo);

    // The mobile app was already classified as externally managed.
    // Emission-level suppression: no [allow_uncovered] table may be
    // re-emitted (a duplicate table would be a TOML parse error on
    // append), but the hit is still counted as already covered.
    let existing = ExistingDecisions {
        allow_uncovered: vec!["apps/mobile-ios/".into()],
        ..Default::default()
    };
    let result = auto_detect::run_against(&r, &existing);

    assert!(
        !result.toml_snippet.contains("[allow_uncovered]"),
        "already-covered mobile app must not re-emit an [allow_uncovered] table, got:\n{}",
        result.toml_snippet
    );
    assert!(
        !result.toml_snippet.contains("apps/mobile-ios"),
        "covered path must not appear in the snippet, got:\n{}",
        result.toml_snippet
    );
    assert_eq!(result.counters.mobile_ios, 0);
    assert!(
        result.counters.already_covered >= 1,
        "allow_uncovered hits must surface in the already_covered counter"
    );

    // Unrelated detection is unaffected.
    assert!(
        result.toml_snippet.contains("glob = \"apps/services/*\""),
        "cargo services glob must still be emitted"
    );
}

#[test]
fn auto_detect_suppresses_hits_covered_by_configured_units() {
    use belaf::cmd::init::auto_detect::{self, ExistingDecisions};

    let repo = seeded_clikd();
    let r = open_repo(&repo);

    // The Tauri desktop app already has a [release_unit] block —
    // re-detection must not emit a second one.
    let existing = ExistingDecisions {
        ignore_paths: vec![],
        allow_uncovered: vec![],
        unit_paths: vec!["apps/desktop".into()],
    };
    let result = auto_detect::run_against(&r, &existing);

    assert!(
        !result.toml_snippet.contains("apps/desktop"),
        "unit-covered path must not resurface, got:\n{}",
        result.toml_snippet
    );
    assert_eq!(
        result.counters.tauri_single_source + result.counters.tauri_legacy,
        0
    );
    assert!(result.counters.already_covered >= 1);
}

#[test]
fn auto_detect_rerun_against_own_output_emits_no_config_blocks() {
    use belaf::cmd::init::auto_detect::{self, ExistingDecisions};
    use belaf::core::config::ConfigurationFile;

    let repo = seeded_clikd();
    let r = open_repo(&repo);

    // First init: write the emitted snippet as the config.
    let first = auto_detect::run(&r);
    let cfg_path = repo.path.join("belaf").join("config.toml");
    std::fs::create_dir_all(cfg_path.parent().unwrap()).unwrap();
    std::fs::write(&cfg_path, &first.toml_snippet).unwrap();

    // Re-detect against that config: every block emitted in round one
    // (release units via unit coverage, mobile app via allow_uncovered)
    // must now be suppressed — the closed loop that makes
    // `init --ci --auto-detect --force` per-path idempotent.
    let cfg = ConfigurationFile::get(&cfg_path).expect("emitted config must load");
    let existing =
        ExistingDecisions::from_config_with_units(&cfg, &r).expect("emitted units must resolve");
    let second = auto_detect::run_against(&r, &existing);

    assert_eq!(
        second.body_snippet, "",
        "re-run must be a config no-op — hints are advice, not body"
    );
    assert!(second.allow_uncovered_paths.is_empty());
    assert!(
        !second.advice.is_empty(),
        "unaddressed sdk-cascade hints must surface as advice"
    );
    assert!(
        second.counters.already_covered >= 4,
        "hexagonal services + tauri + jvm + mobile hits must all count as covered, got {}",
        second.counters.already_covered
    );
}

#[test]
fn force_rerun_merges_new_mobile_app_into_existing_allow_uncovered() {
    use belaf::cmd::init::auto_detect::{self, ExistingDecisions};
    use belaf::core::config::ConfigurationFile;

    let repo = seeded_clikd();
    let r = open_repo(&repo);

    // Config covers everything except the iOS app and already has an
    // [allow_uncovered] table from an earlier decision.
    let cfg_path = repo.path.join("belaf").join("config.toml");
    std::fs::create_dir_all(cfg_path.parent().unwrap()).unwrap();
    std::fs::write(
        &cfg_path,
        "[ignore_paths]\npaths = [\"apps/services/\", \"apps/desktop/\", \"sdks/\"]\n\n[allow_uncovered]\npaths = [\"apps/android-legacy/\"]\n",
    )
    .unwrap();

    let cfg = ConfigurationFile::get(&cfg_path).expect("config must load");
    let existing = ExistingDecisions::from_config_with_units(&cfg, &r).expect("resolve");
    let result = auto_detect::run_against(&r, &existing);
    assert_eq!(
        result.allow_uncovered_paths,
        vec!["apps/mobile-ios/".to_string()],
        "only the uncovered mobile app may be emitted"
    );

    auto_detect::apply_to_config(&cfg_path, &result, false).expect("force apply");
    let after = std::fs::read_to_string(&cfg_path).unwrap();
    assert_eq!(
        after.matches("[allow_uncovered]").count(),
        1,
        "existing table must be merged into, not duplicated:\n{after}"
    );
    let reloaded = ConfigurationFile::get(&cfg_path).expect("merged config must still load");
    assert_eq!(
        reloaded.allow_uncovered.paths,
        vec![
            "apps/android-legacy/".to_string(),
            "apps/mobile-ios/".to_string()
        ]
    );
}
