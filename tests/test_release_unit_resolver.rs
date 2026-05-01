//! Phase B.5 — integration tests for the release_unit resolver.
//! Exercises the end-to-end pipeline: config TOML → ConfigurationFile
//! → resolver::resolve() against a real filesystem layout, with
//! expected ResolvedReleaseUnits.

mod common;

use belaf::core::git::repository::Repository;
use belaf::core::release_unit::resolver::resolve;
use belaf::core::release_unit::syntax::{
    ExplicitReleaseUnitConfig, GlobReleaseUnitConfig, ManifestFileConfig, SourceConfig,
};
use belaf::core::release_unit::{ResolveOrigin, VersionSource};
use common::TestRepo;

fn open_repo(t: &TestRepo) -> Repository {
    Repository::open(&t.path).expect("Repository::open must succeed")
}

// ---------------------------------------------------------------------------
// Glob expansion + fallback_manifests against a clikd-shape filesystem.
// ---------------------------------------------------------------------------

#[test]
fn clikd_shape_glob_expands_with_fallback_manifests() {
    let repo = TestRepo::new();

    // Three services: aura + ekko (use bin), mondo (no bin, only workers).
    repo.write_file(
        "apps/services/aura/crates/bin/Cargo.toml",
        "[package]\nname=\"aura-bin\"\nversion=\"0.1.0\"\n",
    );
    repo.write_file(
        "apps/services/aura/crates/api/Cargo.toml",
        "[package]\nname=\"aura-api\"\nversion=\"0.1.0\"\n",
    );
    repo.write_file(
        "apps/services/ekko/crates/bin/Cargo.toml",
        "[package]\nname=\"ekko-bin\"\nversion=\"0.0.0\"\n",
    );
    repo.write_file(
        "apps/services/mondo/crates/workers/Cargo.toml",
        "[package]\nname=\"mondo-workers\"\nversion=\"0.0.0\"\n",
    );
    repo.write_file(
        "apps/services/mondo/crates/core/Cargo.toml",
        "[package]\nname=\"mondo-core\"\nversion=\"0.0.0\"\n",
    );
    repo.commit("seed clikd-shape");

    let r = open_repo(&repo);

    let glob_cfg = GlobReleaseUnitConfig {
        glob: "apps/services/*".to_string(),
        ecosystem: "cargo".to_string(),
        manifests: vec!["{path}/crates/bin/Cargo.toml".to_string()],
        fallback_manifests: vec!["{path}/crates/workers/Cargo.toml".to_string()],
        satellites: vec!["{path}/crates".to_string()],
        version_field: None,
        name: "{basename}".to_string(),
        tag_format: None,
        visibility: None,
        cascade_from: None,
    };

    let resolved = resolve(&r, &[], &[glob_cfg]).expect("resolver must succeed");

    let mut names: Vec<&str> = resolved.iter().map(|u| u.unit.name.as_str()).collect();
    names.sort();
    assert_eq!(names, vec!["aura", "ekko", "mondo"]);

    // mondo should pick crates/workers/Cargo.toml via fallback
    let mondo = resolved.iter().find(|r| r.unit.name == "mondo").unwrap();
    if let VersionSource::Manifests(ms) = &mondo.unit.source {
        assert_eq!(
            ms[0].path.escaped(),
            "apps/services/mondo/crates/workers/Cargo.toml"
        );
    } else {
        panic!("mondo should be Manifests source");
    }

    // aura should pick the bin (primary) path
    let aura = resolved.iter().find(|r| r.unit.name == "aura").unwrap();
    if let VersionSource::Manifests(ms) = &aura.unit.source {
        assert_eq!(
            ms[0].path.escaped(),
            "apps/services/aura/crates/bin/Cargo.toml"
        );
    }

    // Provenance preserved on every resolved unit
    for u in &resolved {
        match &u.origin {
            ResolveOrigin::Glob {
                glob_index,
                matched_path,
            } => {
                assert_eq!(*glob_index, 0);
                assert!(matched_path.escaped().starts_with("apps/services/"));
            }
            other => panic!("unexpected origin: {other:?}"),
        }
    }
}

#[test]
fn explicit_wins_over_glob_for_overlapping_path() {
    // Edge case 7 — explicit must win, glob silently skips that path.
    let repo = TestRepo::new();
    repo.write_file(
        "apps/services/aura/crates/bin/Cargo.toml",
        "[package]\nname=\"aura-bin\"\nversion=\"0.1.0\"\n",
    );
    repo.write_file(
        "apps/services/ekko/crates/bin/Cargo.toml",
        "[package]\nname=\"ekko-bin\"\nversion=\"0.0.0\"\n",
    );
    repo.commit("seed");

    let r = open_repo(&repo);

    let explicit = ExplicitReleaseUnitConfig {
        name: "aura-custom".to_string(),
        ecosystem: "cargo".to_string(),
        source: SourceConfig {
            manifests: vec![ManifestFileConfig {
                path: "apps/services/aura/crates/bin/Cargo.toml".to_string(),
                ecosystem: None,
                version_field: "cargo_toml".to_string(),
                regex_pattern: None,
                regex_replace: None,
            }],
            external: None,
        },
        satellites: vec!["apps/services/aura/crates".to_string()],
        tag_format: None,
        visibility: None,
        cascade_from: None,
    };

    let glob_cfg = GlobReleaseUnitConfig {
        glob: "apps/services/*".to_string(),
        ecosystem: "cargo".to_string(),
        manifests: vec!["{path}/crates/bin/Cargo.toml".to_string()],
        fallback_manifests: vec![],
        satellites: vec![],
        version_field: None,
        name: "{basename}".to_string(),
        tag_format: None,
        visibility: None,
        cascade_from: None,
    };

    let resolved = resolve(&r, &[explicit], &[glob_cfg]).expect("must succeed");

    // We should see 2 units: explicit "aura-custom" + glob-expanded "ekko".
    // The glob's "aura" expansion is silently skipped because aura's path
    // is already covered by the explicit unit.
    let mut names: Vec<&str> = resolved.iter().map(|u| u.unit.name.as_str()).collect();
    names.sort();
    assert_eq!(names, vec!["aura-custom", "ekko"]);
}

#[test]
fn missing_manifest_path_is_hard_error() {
    let repo = TestRepo::new();
    repo.write_file("README.md", "x"); // just to have something
    repo.commit("seed");

    let r = open_repo(&repo);

    let explicit = ExplicitReleaseUnitConfig {
        name: "missing".to_string(),
        ecosystem: "cargo".to_string(),
        source: SourceConfig {
            manifests: vec![ManifestFileConfig {
                path: "does/not/exist/Cargo.toml".to_string(),
                ecosystem: None,
                version_field: "cargo_toml".to_string(),
                regex_pattern: None,
                regex_replace: None,
            }],
            external: None,
        },
        satellites: vec![],
        tag_format: None,
        visibility: None,
        cascade_from: None,
    };

    let err = resolve(&r, &[explicit], &[]).unwrap_err();
    assert_eq!(err.rule(), "path_does_not_exist");
}

#[test]
fn fallback_exhausted_lists_all_tried_paths() {
    let repo = TestRepo::new();
    repo.write_file(
        "apps/services/aura/crates/api/Cargo.toml",
        "[package]\nname=\"aura-api\"\nversion=\"0.0.0\"\n",
    );
    repo.commit("seed without bin or workers");

    let r = open_repo(&repo);

    let glob_cfg = GlobReleaseUnitConfig {
        glob: "apps/services/*".to_string(),
        ecosystem: "cargo".to_string(),
        manifests: vec!["{path}/crates/bin/Cargo.toml".to_string()],
        fallback_manifests: vec!["{path}/crates/workers/Cargo.toml".to_string()],
        satellites: vec![],
        version_field: None,
        name: "{basename}".to_string(),
        tag_format: None,
        visibility: None,
        cascade_from: None,
    };

    let err = resolve(&r, &[], &[glob_cfg]).unwrap_err();
    assert_eq!(err.rule(), "all_manifests_and_fallbacks_missing");
}

#[test]
fn two_globs_same_path_errors_with_both() {
    let repo = TestRepo::new();
    repo.write_file(
        "apps/services/aura/crates/bin/Cargo.toml",
        "[package]\nname=\"aura-bin\"\nversion=\"0.1.0\"\n",
    );
    repo.commit("seed");

    let r = open_repo(&repo);

    let glob_a = GlobReleaseUnitConfig {
        glob: "apps/services/*".to_string(),
        ecosystem: "cargo".to_string(),
        manifests: vec!["{path}/crates/bin/Cargo.toml".to_string()],
        fallback_manifests: vec![],
        satellites: vec![],
        version_field: None,
        name: "a-{basename}".to_string(),
        tag_format: None,
        visibility: None,
        cascade_from: None,
    };
    let glob_b = GlobReleaseUnitConfig {
        glob: "apps/*/aura".to_string(), // different glob, same matched path
        ecosystem: "cargo".to_string(),
        manifests: vec!["{path}/crates/bin/Cargo.toml".to_string()],
        fallback_manifests: vec![],
        satellites: vec![],
        version_field: None,
        name: "b-{basename}".to_string(),
        tag_format: None,
        visibility: None,
        cascade_from: None,
    };

    let err = resolve(&r, &[], &[glob_a, glob_b]).unwrap_err();
    assert_eq!(err.rule(), "two_globs_same_path");
}

#[test]
fn cascade_source_unknown_errors() {
    let repo = TestRepo::new();
    repo.write_file("sdks/kotlin/gradle.properties", "version=0.1.0\n");
    repo.commit("seed kotlin SDK");

    let r = open_repo(&repo);

    let kotlin = ExplicitReleaseUnitConfig {
        name: "sdk-kotlin".to_string(),
        ecosystem: "jvm-library".to_string(),
        source: SourceConfig {
            manifests: vec![ManifestFileConfig {
                path: "sdks/kotlin/gradle.properties".to_string(),
                ecosystem: None,
                version_field: "gradle_properties".to_string(),
                regex_pattern: None,
                regex_replace: None,
            }],
            external: None,
        },
        satellites: vec!["sdks/kotlin".to_string()],
        tag_format: None,
        visibility: None,
        cascade_from: Some(belaf::core::release_unit::syntax::CascadeRuleConfig {
            source: "ghost-schema".to_string(),
            bump: "floor_minor".to_string(),
        }),
    };

    let err = resolve(&r, &[kotlin], &[]).unwrap_err();
    assert_eq!(err.rule(), "cascade_source_unknown");
}

#[test]
fn ecosystem_mismatch_with_version_field_errors() {
    let repo = TestRepo::new();
    repo.write_file(
        "packages/x/package.json",
        "{\n  \"name\":\"x\",\n  \"version\":\"0.1.0\"\n}\n",
    );
    repo.commit("seed npm package");

    let r = open_repo(&repo);

    // ecosystem=npm but version_field=cargo_toml — clear mismatch.
    let bad = ExplicitReleaseUnitConfig {
        name: "x".to_string(),
        ecosystem: "npm".to_string(),
        source: SourceConfig {
            manifests: vec![ManifestFileConfig {
                path: "packages/x/package.json".to_string(),
                ecosystem: None, // inherits npm
                version_field: "cargo_toml".to_string(),
                regex_pattern: None,
                regex_replace: None,
            }],
            external: None,
        },
        satellites: vec![],
        tag_format: None,
        visibility: None,
        cascade_from: None,
    };

    let err = resolve(&r, &[bad], &[]).unwrap_err();
    assert_eq!(err.rule(), "ecosystem_mismatch_version_field");
}

#[test]
fn external_source_no_filesystem_check() {
    // External-source ReleaseUnits don't reference a file path, so
    // the resolver should accept them without filesystem validation.
    let repo = TestRepo::new();
    repo.commit("seed empty");

    let r = open_repo(&repo);

    let ext = ExplicitReleaseUnitConfig {
        name: "schema".to_string(),
        ecosystem: "external".to_string(),
        source: SourceConfig {
            manifests: vec![],
            external: Some(belaf::core::release_unit::syntax::ExternalConfig {
                tool: "buf".to_string(),
                read_command: "buf mod info".to_string(),
                write_command: "buf mod set-version {version}".to_string(),
                cwd: Some("proto/events".to_string()),
                timeout_sec: 60,
                env: Default::default(),
            }),
        },
        satellites: vec![],
        tag_format: None,
        visibility: None,
        cascade_from: None,
    };

    let resolved = resolve(&r, &[ext], &[]).expect("external-source must resolve");
    assert_eq!(resolved.len(), 1);
    assert!(resolved[0].unit.source.is_external());
}

#[test]
fn nested_bundle_paths_rejected() {
    let repo = TestRepo::new();
    repo.write_file(
        "apps/services/Cargo.toml",
        "[package]\nname=\"outer\"\nversion=\"0.1.0\"\n",
    );
    repo.write_file(
        "apps/services/aura/Cargo.toml",
        "[package]\nname=\"inner\"\nversion=\"0.1.0\"\n",
    );
    repo.commit("seed nested layout");

    let r = open_repo(&repo);

    let outer = ExplicitReleaseUnitConfig {
        name: "outer".to_string(),
        ecosystem: "cargo".to_string(),
        source: SourceConfig {
            manifests: vec![ManifestFileConfig {
                path: "apps/services/Cargo.toml".to_string(),
                ecosystem: None,
                version_field: "cargo_toml".to_string(),
                regex_pattern: None,
                regex_replace: None,
            }],
            external: None,
        },
        satellites: vec![],
        tag_format: None,
        visibility: None,
        cascade_from: None,
    };
    let inner = ExplicitReleaseUnitConfig {
        name: "inner".to_string(),
        ecosystem: "cargo".to_string(),
        source: SourceConfig {
            manifests: vec![ManifestFileConfig {
                path: "apps/services/aura/Cargo.toml".to_string(),
                ecosystem: None,
                version_field: "cargo_toml".to_string(),
                regex_pattern: None,
                regex_replace: None,
            }],
            external: None,
        },
        satellites: vec![],
        tag_format: None,
        visibility: None,
        cascade_from: None,
    };

    let err = resolve(&r, &[outer, inner], &[]).unwrap_err();
    assert_eq!(err.rule(), "nested_bundle_path");
}

#[test]
fn glob_zero_matches_does_not_error() {
    let repo = TestRepo::new();
    repo.write_file("README.md", "x");
    repo.commit("seed");

    let r = open_repo(&repo);

    let glob_cfg = GlobReleaseUnitConfig {
        glob: "no-such-dir/*".to_string(),
        ecosystem: "cargo".to_string(),
        manifests: vec!["{path}/Cargo.toml".to_string()],
        fallback_manifests: vec![],
        satellites: vec![],
        version_field: None,
        name: "{basename}".to_string(),
        tag_format: None,
        visibility: None,
        cascade_from: None,
    };

    // Edge case 6 — glob matches zero dirs is NOT an error.
    let resolved = resolve(&r, &[], &[glob_cfg]).expect("must succeed");
    assert!(resolved.is_empty());
}
