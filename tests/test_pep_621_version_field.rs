//! `version_field = "pep_621"` — first-class read/write of
//! `pyproject.toml` `[project].version` from explicit
//! `[release_unit]` blocks (no `generic_regex` workaround).

mod common;

use belaf::core::config::NamedReleaseUnitConfig;
use belaf::core::git::repository::Repository;
use belaf::core::release_unit::resolver::resolve;
use belaf::core::release_unit::syntax::{ManifestFileConfig, ManifestList, ReleaseUnitConfig};
use belaf::core::release_unit::{VersionFieldSpec, VersionSource};
use common::TestRepo;

fn open_repo(t: &TestRepo) -> Repository {
    Repository::open(&t.path).expect("Repository::open must succeed")
}

fn pypa_unit(name: &str, manifest_path: &str, version_field: &str) -> NamedReleaseUnitConfig {
    NamedReleaseUnitConfig {
        name: name.to_string(),
        config: ReleaseUnitConfig {
            ecosystem: Some("pypa".into()),
            name: None,
            glob: None,
            manifests: Some(ManifestList::Explicit(vec![ManifestFileConfig {
                path: manifest_path.into(),
                ecosystem: None,
                version_field: version_field.into(),
                regex_pattern: None,
                regex_replace: None,
            }])),
            external: None,
            fallback_manifests: vec![],
            version_field: None,
            satellites: vec![],
            tag_format: None,
            visibility: None,
            cascade_from: None,
        },
    }
}

#[test]
fn pep_621_resolves_in_explicit_pypa_block() {
    let repo = TestRepo::new();
    repo.write_file(
        "packages/my-pkg/pyproject.toml",
        "[project]\nname = \"my-pkg\"\nversion = \"1.2.3\"\n",
    );
    repo.commit("seed");

    let r = open_repo(&repo);
    let out = resolve(
        &r,
        &[pypa_unit(
            "my-pkg",
            "packages/my-pkg/pyproject.toml",
            "pep_621",
        )],
    )
    .expect("ok");
    let resolved = out.resolved;

    assert_eq!(resolved.len(), 1);
    let VersionSource::Manifests(ms) = &resolved[0].unit.source else {
        panic!("expected Manifests source");
    };
    assert_eq!(ms.len(), 1);
    assert!(matches!(ms[0].version_field, VersionFieldSpec::Pep621));
}

#[test]
fn pep_621_rejects_non_pypa_ecosystem() {
    let repo = TestRepo::new();
    repo.write_file(
        "pyproject.toml",
        "[project]\nname = \"my-pkg\"\nversion = \"1.2.3\"\n",
    );
    repo.commit("seed");

    let r = open_repo(&repo);

    let bad = NamedReleaseUnitConfig {
        name: "my-pkg".into(),
        config: ReleaseUnitConfig {
            ecosystem: Some("npm".into()),
            name: None,
            glob: None,
            manifests: Some(ManifestList::Explicit(vec![ManifestFileConfig {
                path: "pyproject.toml".into(),
                ecosystem: None,
                version_field: "pep_621".into(),
                regex_pattern: None,
                regex_replace: None,
            }])),
            external: None,
            fallback_manifests: vec![],
            version_field: None,
            satellites: vec![],
            tag_format: None,
            visibility: None,
            cascade_from: None,
        },
    };
    let err = resolve(&r, &[bad]).unwrap_err();
    // npm + pep_621 hits the explicit blocklist (`("npm", "pep_621")`),
    // surfacing as the same `ecosystem_mismatch_version_field` rule.
    assert_eq!(err.rule(), "ecosystem_mismatch_version_field");
}

#[test]
fn pep_621_round_trip_read_then_write() {
    use belaf::core::version_field::{read, write};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pyproject.toml");
    std::fs::write(
        &path,
        "[project]\nname = \"x\"\nversion = \"0.1.0\"\nauthors = []\n",
    )
    .unwrap();

    let v = read(&VersionFieldSpec::Pep621, &path).expect("read pep_621");
    assert_eq!(v, "0.1.0");

    write(&VersionFieldSpec::Pep621, &path, "0.2.0").expect("write pep_621");
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(after.contains("version = \"0.2.0\""));
    assert!(after.contains("name = \"x\""), "name preserved");
    assert!(after.contains("authors"), "other fields preserved");
}
