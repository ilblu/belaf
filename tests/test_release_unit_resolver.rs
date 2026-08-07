//! Integration tests for the release_unit resolver.
//! Exercises the end-to-end pipeline: typed `ReleaseUnitConfig` → resolver
//! against a real filesystem layout, with expected `ResolvedReleaseUnit`s.

mod common;

use belaf::core::config::NamedReleaseUnitConfig;
use belaf::core::git::repository::Repository;
use belaf::core::release_unit::resolver::resolve;
use belaf::core::release_unit::syntax::{
    CascadeRuleConfig, ExternalConfig, ManifestFileConfig, ManifestList, ReleaseUnitConfig,
};
use belaf::core::release_unit::{ResolveOrigin, VersionSource};
use belaf::core::resolved_release_unit::UnitKind;
use common::TestRepo;

fn open_repo(t: &TestRepo) -> Repository {
    Repository::open(&t.path).expect("Repository::open must succeed")
}

// Test-builder helpers — keep tests focused on the resolver behaviour
// rather than typed-builder boilerplate.

#[derive(Default)]
struct ExplicitBuilder {
    ecosystem: String,
    manifests: Vec<ManifestFileConfig>,
    external: Option<ExternalConfig>,
    satellites: Vec<String>,
    cascade_from: Option<CascadeRuleConfig>,
    kind: Option<String>,
    paths: Vec<String>,
}

fn explicit(name: &str, ecosystem: &str) -> ExplicitBuilder {
    ExplicitBuilder {
        ecosystem: ecosystem.to_string(),
        ..Default::default()
    }
    .with_name(name)
}

impl ExplicitBuilder {
    fn with_name(self, _: &str) -> Self {
        // Name is captured separately when calling .build()
        self
    }
    fn with_manifest(mut self, path: &str, version_field: &str) -> Self {
        self.manifests.push(ManifestFileConfig {
            path: path.to_string(),
            ecosystem: None,
            version_field: version_field.to_string(),
            regex_pattern: None,
            regex_replace: None,
        });
        self
    }
    fn with_satellite(mut self, sat: &str) -> Self {
        self.satellites.push(sat.to_string());
        self
    }
    fn with_external(mut self, ext: ExternalConfig) -> Self {
        self.external = Some(ext);
        self
    }
    fn with_cascade(mut self, source: &str, bump: &str) -> Self {
        self.cascade_from = Some(CascadeRuleConfig {
            source: source.to_string(),
            bump: bump.to_string(),
        });
        self
    }
    /// A manifest-less paths-only block: no ecosystem required.
    fn paths_only(kind: &str, paths: &[&str]) -> ExplicitBuilder {
        ExplicitBuilder {
            ecosystem: String::new(),
            kind: Some(kind.to_string()),
            paths: paths.iter().map(|p| p.to_string()).collect(),
            ..Default::default()
        }
    }
    fn build(self, name: &str) -> NamedReleaseUnitConfig {
        NamedReleaseUnitConfig {
            name: name.to_string(),
            config: ReleaseUnitConfig {
                ecosystem: if self.ecosystem.is_empty() {
                    None
                } else {
                    Some(self.ecosystem)
                },
                name: None,
                glob: None,
                manifests: if self.manifests.is_empty() {
                    None
                } else {
                    Some(ManifestList::Explicit(self.manifests))
                },
                external: self.external,
                fallback_manifests: vec![],
                version_field: None,
                satellites: self.satellites,
                tag_format: None,
                visibility: None,
                kind: self.kind,
                paths: self.paths,
                cascade_from: self.cascade_from,
                bump: None,
                baseline: None,
            },
        }
    }
}

#[derive(Default)]
struct GlobBuilder {
    ecosystem: String,
    glob: String,
    name_template: String,
    manifests: Vec<String>,
    fallback_manifests: Vec<String>,
    satellites: Vec<String>,
    kind: Option<String>,
}

fn glob(config_key: &str, ecosystem: &str, glob: &str, name_template: &str) -> GlobBuilder {
    let _ = config_key; // captured by .build()
    GlobBuilder {
        ecosystem: ecosystem.to_string(),
        glob: glob.to_string(),
        name_template: name_template.to_string(),
        ..Default::default()
    }
}

impl GlobBuilder {
    fn with_manifest(mut self, template: &str) -> Self {
        self.manifests.push(template.to_string());
        self
    }
    fn with_fallback(mut self, template: &str) -> Self {
        self.fallback_manifests.push(template.to_string());
        self
    }
    fn with_satellite(mut self, template: &str) -> Self {
        self.satellites.push(template.to_string());
        self
    }
    fn with_kind(mut self, kind: &str) -> Self {
        self.kind = Some(kind.to_string());
        self
    }
    fn build(self, config_key: &str) -> NamedReleaseUnitConfig {
        NamedReleaseUnitConfig {
            name: config_key.to_string(),
            config: ReleaseUnitConfig {
                ecosystem: Some(self.ecosystem),
                name: Some(self.name_template),
                glob: Some(self.glob),
                manifests: Some(ManifestList::Templates(self.manifests)),
                external: None,
                fallback_manifests: self.fallback_manifests,
                version_field: None,
                satellites: self.satellites,
                tag_format: None,
                visibility: None,
                kind: self.kind,
                paths: vec![],
                cascade_from: None,
                bump: None,
                baseline: None,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Glob expansion + fallback_manifests against a clikd-shape filesystem.
// ---------------------------------------------------------------------------

#[test]
fn clikd_shape_glob_expands_with_fallback_manifests() {
    let repo = TestRepo::new();

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

    let services = glob("services", "cargo", "apps/services/*", "{basename}")
        .with_manifest("{path}/crates/bin/Cargo.toml")
        .with_fallback("{path}/crates/workers/Cargo.toml")
        .with_satellite("{path}/crates")
        .build("services");

    let resolved = resolve(&r, &[services])
        .expect("resolver must succeed")
        .resolved;

    let mut names: Vec<&str> = resolved.iter().map(|u| u.unit.name.as_str()).collect();
    names.sort();
    assert_eq!(names, vec!["aura", "ekko", "mondo"]);

    let mondo = resolved.iter().find(|r| r.unit.name == "mondo").unwrap();
    if let VersionSource::Manifests(ms) = &mondo.unit.source {
        assert_eq!(
            ms[0].path.escaped(),
            "apps/services/mondo/crates/workers/Cargo.toml"
        );
    } else {
        panic!("mondo should be Manifests source");
    }

    let aura = resolved.iter().find(|r| r.unit.name == "aura").unwrap();
    if let VersionSource::Manifests(ms) = &aura.unit.source {
        assert_eq!(
            ms[0].path.escaped(),
            "apps/services/aura/crates/bin/Cargo.toml"
        );
    }

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

    let aura_custom = explicit("aura-custom", "cargo")
        .with_manifest("apps/services/aura/crates/bin/Cargo.toml", "cargo_toml")
        .with_satellite("apps/services/aura/crates")
        .build("aura-custom");

    let services = glob("services", "cargo", "apps/services/*", "{basename}")
        .with_manifest("{path}/crates/bin/Cargo.toml")
        .build("services");

    let resolved = resolve(&r, &[aura_custom, services])
        .expect("must succeed")
        .resolved;

    let mut names: Vec<&str> = resolved.iter().map(|u| u.unit.name.as_str()).collect();
    names.sort();
    assert_eq!(names, vec!["aura-custom", "ekko"]);
}

#[test]
fn missing_manifest_path_is_hard_error() {
    let repo = TestRepo::new();
    repo.write_file("README.md", "x");
    repo.commit("seed");

    let r = open_repo(&repo);

    let missing = explicit("missing", "cargo")
        .with_manifest("does/not/exist/Cargo.toml", "cargo_toml")
        .build("missing");

    let err = resolve(&r, &[missing]).unwrap_err();
    assert_eq!(err.rule(), "path_does_not_exist");
}

#[test]
fn glob_match_without_expected_manifest_is_skipped() {
    // F9 — a directory matched by the glob that carries none of the expected
    // manifests/fallbacks (e.g. a flat test crate like `apps/services/e2e`)
    // is skipped, not a hard error. A sibling that *does* match still resolves.
    let repo = TestRepo::new();
    // `aura` has only crates/api — neither the bin manifest nor the workers
    // fallback the glob expects → skipped.
    repo.write_file(
        "apps/services/aura/crates/api/Cargo.toml",
        "[package]\nname=\"aura-api\"\nversion=\"0.0.0\"\n",
    );
    // `ekko` has the expected bin manifest → resolves.
    repo.write_file(
        "apps/services/ekko/crates/bin/Cargo.toml",
        "[package]\nname=\"ekko-bin\"\nversion=\"0.1.0\"\n",
    );
    repo.commit("seed: one service-shaped dir, one not");

    let r = open_repo(&repo);

    let services = glob("services", "cargo", "apps/services/*", "{basename}")
        .with_manifest("{path}/crates/bin/Cargo.toml")
        .with_fallback("{path}/crates/workers/Cargo.toml")
        .build("services");

    let resolved = resolve(&r, &[services]).expect("must succeed").resolved;
    let names: Vec<&str> = resolved.iter().map(|u| u.unit.name.as_str()).collect();
    assert_eq!(names, vec!["ekko"]);
}

// ---------------------------------------------------------------------------
// F1 — unit kinds + manifest-less `paths = [...]` units.
// ---------------------------------------------------------------------------

#[test]
fn paths_only_internal_unit_resolves() {
    let repo = TestRepo::new();
    repo.write_file("proto/schema.proto", "syntax = \"proto3\";\n");
    repo.commit("seed proto");
    let r = open_repo(&repo);

    let proto = ExplicitBuilder::paths_only("internal", &["proto"]).build("proto");
    let resolved = resolve(&r, &[proto]).expect("must succeed").resolved;

    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].unit.kind, UnitKind::Internal);
    assert!(matches!(
        resolved[0].unit.source,
        VersionSource::PathsOnly(_)
    ));
}

#[test]
fn paths_only_ignore_unit_resolves() {
    let repo = TestRepo::new();
    repo.write_file(
        "apps/services/e2e/Cargo.toml",
        "[package]\nname=\"e2e\"\nversion=\"0.0.0\"\n",
    );
    repo.commit("seed e2e");
    let r = open_repo(&repo);

    let e2e = ExplicitBuilder::paths_only("ignore", &["apps/services/e2e"]).build("e2e");
    let resolved = resolve(&r, &[e2e]).expect("must succeed").resolved;

    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].unit.kind, UnitKind::Ignore);
}

#[test]
fn paths_with_manifest_is_error() {
    let repo = TestRepo::new();
    repo.write_file(
        "proto/Cargo.toml",
        "[package]\nname=\"p\"\nversion=\"0.0.0\"\n",
    );
    repo.commit("seed");
    let r = open_repo(&repo);

    let bad = ExplicitBuilder::paths_only("internal", &["proto"])
        .with_manifest("proto/Cargo.toml", "cargo_toml")
        .build("proto");
    let err = resolve(&r, &[bad]).unwrap_err();
    assert_eq!(err.rule(), "paths_only_invalid");
}

#[test]
fn paths_on_deploy_unit_is_error() {
    let repo = TestRepo::new();
    repo.write_file("proto/schema.proto", "x\n");
    repo.commit("seed");
    let r = open_repo(&repo);

    // `kind = "deploy"` (the default) + `paths` → error: a deploy unit needs
    // a real version source.
    let bad = ExplicitBuilder::paths_only("deploy", &["proto"]).build("proto");
    let err = resolve(&r, &[bad]).unwrap_err();
    assert_eq!(err.rule(), "paths_only_invalid");
}

#[test]
fn glob_unit_carries_internal_kind() {
    let repo = TestRepo::new();
    repo.write_file(
        "packages/clikd-foo/Cargo.toml",
        "[package]\nname=\"clikd-foo\"\nversion=\"0.1.0\"\n",
    );
    repo.commit("seed package");
    let r = open_repo(&repo);

    let packages = glob("packages", "cargo", "packages/*", "{basename}")
        .with_manifest("{path}/Cargo.toml")
        .with_kind("internal")
        .build("packages");
    let resolved = resolve(&r, &[packages]).expect("must succeed").resolved;

    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].unit.name, "clikd-foo");
    assert_eq!(resolved[0].unit.kind, UnitKind::Internal);
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

    let glob_a = glob("a", "cargo", "apps/services/*", "a-{basename}")
        .with_manifest("{path}/crates/bin/Cargo.toml")
        .build("a");
    let glob_b = glob("b", "cargo", "apps/*/aura", "b-{basename}")
        .with_manifest("{path}/crates/bin/Cargo.toml")
        .build("b");

    let err = resolve(&r, &[glob_a, glob_b]).unwrap_err();
    assert_eq!(err.rule(), "two_globs_same_path");
}

#[test]
fn cascade_source_unknown_errors() {
    let repo = TestRepo::new();
    repo.write_file("sdks/kotlin/gradle.properties", "version=0.1.0\n");
    repo.commit("seed kotlin SDK");

    let r = open_repo(&repo);

    let kotlin = explicit("sdk-kotlin", "jvm-library")
        .with_manifest("sdks/kotlin/gradle.properties", "gradle_properties")
        .with_satellite("sdks/kotlin")
        .with_cascade("ghost-schema", "floor_minor")
        .build("sdk-kotlin");

    let err = resolve(&r, &[kotlin]).unwrap_err();
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

    let bad = explicit("x", "npm")
        .with_manifest("packages/x/package.json", "cargo_toml")
        .build("x");

    let err = resolve(&r, &[bad]).unwrap_err();
    assert_eq!(err.rule(), "ecosystem_mismatch_version_field");
}

#[test]
fn external_source_no_filesystem_check() {
    let repo = TestRepo::new();
    repo.commit("seed empty");

    let r = open_repo(&repo);

    let ext = explicit("schema", "external")
        .with_external(ExternalConfig {
            tool: "buf".to_string(),
            read_command: "buf mod info".to_string(),
            write_command: "buf mod set-version {version}".to_string(),
            cwd: Some("proto/events".to_string()),
            timeout_sec: 60,
            env: Default::default(),
        })
        .build("schema");

    let resolved = resolve(&r, &[ext])
        .expect("external-source must resolve")
        .resolved;
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

    let outer = explicit("outer", "cargo")
        .with_manifest("apps/services/Cargo.toml", "cargo_toml")
        .build("outer");
    let inner = explicit("inner", "cargo")
        .with_manifest("apps/services/aura/Cargo.toml", "cargo_toml")
        .build("inner");

    let err = resolve(&r, &[outer, inner]).unwrap_err();
    assert_eq!(err.rule(), "nested_bundle_path");
}

#[test]
fn glob_zero_matches_does_not_error() {
    let repo = TestRepo::new();
    repo.write_file("README.md", "x");
    repo.commit("seed");

    let r = open_repo(&repo);

    let services = glob("services", "cargo", "no-such-dir/*", "{basename}")
        .with_manifest("{path}/Cargo.toml")
        .build("services");

    let resolved = resolve(&r, &[services]).expect("must succeed").resolved;
    assert!(resolved.is_empty());
}
