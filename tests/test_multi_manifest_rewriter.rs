//! Phase D.7 — integration tests for the MultiManifestRewriter +
//! VersionFieldSpec dispatch chain. Lockstep behaviour, idempotent
//! re-run, partial-state heal-forward.

mod common;

use belaf::core::git::repository::{RepoPathBuf, Repository};
use belaf::core::release_unit::{ManifestFile, VersionFieldSpec};
use belaf::core::rewriters::multi_manifest;
use belaf::core::wire::known::Ecosystem;
use common::TestRepo;

fn open_repo(t: &TestRepo) -> Repository {
    Repository::open(&t.path).expect("Repository::open")
}

fn manifest(path: &str, ecosystem: &str, spec: VersionFieldSpec) -> ManifestFile {
    ManifestFile {
        path: RepoPathBuf::new(path.as_bytes()),
        ecosystem: Ecosystem::classify(ecosystem),
        version_field: spec,
    }
}

#[test]
fn tauri_legacy_three_files_lockstep() {
    let repo = TestRepo::new();
    repo.write_file(
        "apps/desktop/package.json",
        "{\n  \"name\": \"desktop\",\n  \"version\": \"0.1.0\"\n}\n",
    );
    repo.write_file(
        "apps/desktop/src-tauri/Cargo.toml",
        "[package]\nname = \"desktop\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    repo.write_file(
        "apps/desktop/src-tauri/tauri.conf.json",
        r#"{
  "productName": "desktop",
  "version": "0.1.0"
}
"#,
    );
    repo.commit("seed tauri triplet");

    let r = open_repo(&repo);
    let manifests = vec![
        manifest(
            "apps/desktop/package.json",
            "npm",
            VersionFieldSpec::NpmPackageJson,
        ),
        manifest(
            "apps/desktop/src-tauri/Cargo.toml",
            "cargo",
            VersionFieldSpec::CargoToml,
        ),
        manifest(
            "apps/desktop/src-tauri/tauri.conf.json",
            "tauri",
            VersionFieldSpec::TauriConfJson,
        ),
    ];

    let report = multi_manifest::write_all(&manifests, "0.2.0", &r).expect("must succeed");
    assert_eq!(report.wrote.len(), 3);
    assert!(report.already_at_target.is_empty());

    // Verify each file moved to 0.2.0
    let pkg = repo.read_file("apps/desktop/package.json");
    assert!(pkg.contains("\"version\": \"0.2.0\""));
    let cargo = repo.read_file("apps/desktop/src-tauri/Cargo.toml");
    assert!(cargo.contains("version = \"0.2.0\""));
    let tauri = repo.read_file("apps/desktop/src-tauri/tauri.conf.json");
    assert!(tauri.contains(r#""version": "0.2.0""#));
}

#[test]
fn idempotent_rerun_is_no_op() {
    let repo = TestRepo::new();
    repo.write_file(
        "apps/desktop/package.json",
        "{\n  \"name\": \"desktop\",\n  \"version\": \"0.2.0\"\n}\n",
    );
    repo.commit("seed");

    let r = open_repo(&repo);
    let manifests = vec![manifest(
        "apps/desktop/package.json",
        "npm",
        VersionFieldSpec::NpmPackageJson,
    )];

    let report = multi_manifest::write_all(&manifests, "0.2.0", &r).unwrap();
    assert!(report.wrote.is_empty());
    assert_eq!(report.already_at_target.len(), 1);
}

#[test]
fn partial_state_heal_forward() {
    // 2 of 3 files already at target — third should still be written.
    let repo = TestRepo::new();
    repo.write_file(
        "apps/desktop/package.json",
        "{\n  \"name\": \"desktop\",\n  \"version\": \"0.2.0\"\n}\n",
    );
    repo.write_file(
        "apps/desktop/src-tauri/Cargo.toml",
        "[package]\nname = \"desktop\"\nversion = \"0.2.0\"\nedition = \"2021\"\n",
    );
    repo.write_file(
        "apps/desktop/src-tauri/tauri.conf.json",
        r#"{
  "version": "0.1.0"
}
"#,
    );
    repo.commit("seed partial-state");

    let r = open_repo(&repo);
    let manifests = vec![
        manifest(
            "apps/desktop/package.json",
            "npm",
            VersionFieldSpec::NpmPackageJson,
        ),
        manifest(
            "apps/desktop/src-tauri/Cargo.toml",
            "cargo",
            VersionFieldSpec::CargoToml,
        ),
        manifest(
            "apps/desktop/src-tauri/tauri.conf.json",
            "tauri",
            VersionFieldSpec::TauriConfJson,
        ),
    ];

    let report = multi_manifest::write_all(&manifests, "0.2.0", &r).unwrap();
    assert_eq!(
        report.wrote.len(),
        1,
        "the lagging tauri.conf.json must be written"
    );
    assert_eq!(
        report.already_at_target.len(),
        2,
        "package.json + Cargo.toml were already at target"
    );

    let tauri = repo.read_file("apps/desktop/src-tauri/tauri.conf.json");
    assert!(tauri.contains(r#""version": "0.2.0""#));
}

#[test]
fn gradle_properties_single_file() {
    let repo = TestRepo::new();
    repo.write_file(
        "sdks/kotlin/gradle.properties",
        "group=com.example\nversion=0.1.0\nname=foo\n",
    );
    repo.commit("seed gradle props");

    let r = open_repo(&repo);
    let manifests = vec![manifest(
        "sdks/kotlin/gradle.properties",
        "jvm-library",
        VersionFieldSpec::GradleProperties,
    )];

    let report = multi_manifest::write_all(&manifests, "0.2.0", &r).unwrap();
    assert_eq!(report.wrote.len(), 1);

    let after = repo.read_file("sdks/kotlin/gradle.properties");
    assert!(after.contains("version=0.2.0"));
    assert!(after.contains("group=com.example")); // ordering preserved
    assert!(after.contains("name=foo"));
}

#[test]
fn generic_regex_with_template_substitution() {
    let repo = TestRepo::new();
    repo.write_file("config/version.txt", "VERSION=1.0.0\n");
    repo.commit("seed version.txt");

    let r = open_repo(&repo);
    let manifests = vec![manifest(
        "config/version.txt",
        "external",
        VersionFieldSpec::GenericRegex {
            pattern: r"(?m)^VERSION=(\d+\.\d+\.\d+)$".to_string(),
            replace: "VERSION={version}".to_string(),
        },
    )];

    let report = multi_manifest::write_all(&manifests, "2.0.0", &r).unwrap();
    assert_eq!(report.wrote.len(), 1);

    let after = repo.read_file("config/version.txt");
    assert_eq!(after, "VERSION=2.0.0\n");
}
