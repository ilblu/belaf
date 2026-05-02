//! End-to-end TOML round-trip for the 1.0 release_unit config syntax.
//! Verifies that `[release_unit.<name>]` (named-entry, with optional
//! `glob` field), `[ignore_paths]`, `[allow_uncovered]`, and
//! `[ecosystems.*]` all parse, every field is preserved through
//! `ConfigurationFile::get` → `into_toml` → re-parse, and all 5
//! `VersionFieldSpec` variants plus both source-form variants
//! (manifests / external) survive the round-trip.

use std::fs;

use belaf::core::config::ConfigurationFile;
use belaf::core::release_unit::syntax::ManifestList;
use tempfile::TempDir;

/// User-supplied overlay covering every section we ship in 1.0.
/// `ConfigurationFile::get` merges this on top of the embedded baseline,
/// so the existing `repo`/`changelog`/`bump`/`commit_attribution` fields
/// don't need to be set here.
const FULL_OVERLAY: &str = r#"
# ---------------------------------------------------------------------------
# Explicit ReleaseUnit — Manifests source
# ---------------------------------------------------------------------------
[release_unit.aura]
ecosystem = "cargo"
satellites = ["apps/services/aura/crates"]
tag_format = "{name}-v{version}"
visibility = "public"
manifests = [
  { path = "apps/services/aura/crates/bin/Cargo.toml", version_field = "cargo_toml" },
]

# ---------------------------------------------------------------------------
# Multi-manifest ReleaseUnit (Tauri legacy)
# ---------------------------------------------------------------------------
[release_unit.desktop]
ecosystem = "tauri"
satellites = ["apps/clients/desktop"]
manifests = [
  { path = "apps/clients/desktop/package.json", ecosystem = "npm", version_field = "npm_package_json" },
  { path = "apps/clients/desktop/src-tauri/Cargo.toml", ecosystem = "cargo", version_field = "cargo_toml" },
  { path = "apps/clients/desktop/src-tauri/tauri.conf.json", version_field = "tauri_conf_json" },
]

# ---------------------------------------------------------------------------
# JVM library SDK with cascade — covers GradleProperties + CascadeRule
# ---------------------------------------------------------------------------
[release_unit.sdk-kotlin]
ecosystem = "jvm-library"
satellites = ["sdks/kotlin"]
cascade_from = { source = "schema", bump = "floor_minor" }
manifests = [
  { path = "sdks/kotlin/gradle.properties", version_field = "gradle_properties" },
]

# ---------------------------------------------------------------------------
# External-source ReleaseUnit
# ---------------------------------------------------------------------------
[release_unit.schema]
ecosystem = "external"
satellites = ["proto/events"]

[release_unit.schema.external]
tool = "buf"
read_command = "buf mod info --format json"
write_command = "buf mod set-version {version}"
cwd = "proto/events"
timeout_sec = 90

# ---------------------------------------------------------------------------
# GenericRegex VersionFieldSpec (escape hatch)
# ---------------------------------------------------------------------------
[release_unit.version-txt]
ecosystem = "external"
manifests = [
  { path = "config/version.txt", version_field = "generic_regex", regex_pattern = '^VERSION=(\d+\.\d+\.\d+)$', regex_replace = "VERSION={version}" },
]

# ---------------------------------------------------------------------------
# Hidden satellite-aggregator unit (Visibility::Hidden)
# ---------------------------------------------------------------------------
[release_unit.internal-aggregator]
ecosystem = "cargo"
satellites = ["packages/internal"]
visibility = "hidden"
manifests = [
  { path = "packages/internal/Cargo.toml", version_field = "cargo_toml" },
]

# ---------------------------------------------------------------------------
# Glob form — services-as-bundles pattern. Same `[release_unit.<name>]`
# table, just with `glob` set.
# ---------------------------------------------------------------------------
[release_unit.services]
ecosystem = "cargo"
glob = "apps/services/*"
name = "{basename}"
manifests = ["{path}/crates/bin/Cargo.toml"]
fallback_manifests = ["{path}/crates/workers/Cargo.toml"]
satellites = ["{path}/crates"]

# ---------------------------------------------------------------------------
# ignore_paths and allow_uncovered (distinct semantics)
# ---------------------------------------------------------------------------
[ignore_paths]
paths = ["examples/", "internal-tools/", "third_party/"]

[allow_uncovered]
paths = ["legacy/old-thing/", "apps/clients/ios/", "apps/clients/android/"]

# ---------------------------------------------------------------------------
# Per-ecosystem smart defaults
# ---------------------------------------------------------------------------
[ecosystems.cargo]
hexagonal_pattern = "auto"
workspace_mode = "separate"

[ecosystems.npm]
sync_workspaces = "auto"

[ecosystems.tauri]
detect_triplet = true
prefer_single_source = true

[ecosystems.jvm_library]
gradle_properties_path = "gradle.properties"
"#;

#[test]
fn full_overlay_parses_and_preserves_every_field() {
    let dir = TempDir::new().unwrap();
    let cfg_path = dir.path().join("config.toml");
    fs::write(&cfg_path, FULL_OVERLAY).unwrap();

    let cfg = ConfigurationFile::get(&cfg_path).expect("must parse full overlay");

    // 7 release units (6 explicit + 1 glob), all under one named-entry table.
    assert_eq!(
        cfg.release_units.len(),
        7,
        "expected 7 release_unit entries (6 explicit + 1 glob)"
    );

    let services = cfg
        .release_units
        .iter()
        .find(|u| u.name == "services")
        .expect("services glob entry must be present");
    assert!(services.config.is_glob());
    assert_eq!(services.config.glob.as_deref(), Some("apps/services/*"));
    assert_eq!(services.config.name.as_deref(), Some("{basename}"));
    assert_eq!(
        services.config.fallback_manifests,
        vec!["{path}/crates/workers/Cargo.toml"]
    );

    // ignore_paths + allow_uncovered
    assert_eq!(cfg.ignore_paths.paths.len(), 3);
    assert!(cfg.ignore_paths.paths.contains(&"examples/".to_string()));
    assert_eq!(cfg.allow_uncovered.paths.len(), 3);
    assert!(cfg
        .allow_uncovered
        .paths
        .contains(&"apps/clients/ios/".to_string()));

    // ecosystems.*
    assert_eq!(
        cfg.ecosystems.cargo.workspace_mode.as_deref(),
        Some("separate")
    );
    assert!(cfg.ecosystems.tauri.detect_triplet);
    assert_eq!(
        cfg.ecosystems.jvm_library.gradle_properties_path.as_deref(),
        Some("gradle.properties")
    );
}

fn explicit_manifests(
    list: Option<&ManifestList>,
) -> &[belaf::core::release_unit::syntax::ManifestFileConfig] {
    match list {
        Some(ManifestList::Explicit(m)) => m.as_slice(),
        _ => &[],
    }
}

#[test]
fn manifests_source_aura() {
    let dir = TempDir::new().unwrap();
    let cfg_path = dir.path().join("config.toml");
    fs::write(&cfg_path, FULL_OVERLAY).unwrap();
    let cfg = ConfigurationFile::get(&cfg_path).unwrap();

    let aura = cfg
        .release_units
        .iter()
        .find(|u| u.name == "aura")
        .expect("aura must be present");
    assert_eq!(aura.config.ecosystem, "cargo");
    let manifests = explicit_manifests(aura.config.manifests.as_ref());
    assert_eq!(manifests.len(), 1);
    assert!(aura.config.external.is_none());
    assert_eq!(manifests[0].version_field, "cargo_toml");
    assert_eq!(aura.config.tag_format.as_deref(), Some("{name}-v{version}"));
    assert_eq!(aura.config.visibility.as_deref(), Some("public"));
    assert_eq!(
        aura.config.satellites,
        vec!["apps/services/aura/crates".to_string()]
    );
}

#[test]
fn multi_manifest_tauri_legacy() {
    let dir = TempDir::new().unwrap();
    let cfg_path = dir.path().join("config.toml");
    fs::write(&cfg_path, FULL_OVERLAY).unwrap();
    let cfg = ConfigurationFile::get(&cfg_path).unwrap();

    let desktop = cfg
        .release_units
        .iter()
        .find(|u| u.name == "desktop")
        .expect("desktop must be present");
    assert_eq!(desktop.config.ecosystem, "tauri");
    let manifests = explicit_manifests(desktop.config.manifests.as_ref());
    assert_eq!(
        manifests.len(),
        3,
        "Tauri legacy multi-file = 3 manifests in lockstep"
    );
    let kinds: Vec<_> = manifests.iter().map(|m| m.version_field.as_str()).collect();
    assert_eq!(
        kinds,
        vec!["npm_package_json", "cargo_toml", "tauri_conf_json"]
    );
}

#[test]
fn external_source_with_custom_timeout() {
    let dir = TempDir::new().unwrap();
    let cfg_path = dir.path().join("config.toml");
    fs::write(&cfg_path, FULL_OVERLAY).unwrap();
    let cfg = ConfigurationFile::get(&cfg_path).unwrap();

    let schema = cfg
        .release_units
        .iter()
        .find(|u| u.name == "schema")
        .expect("schema must be present");
    let ext = schema
        .config
        .external
        .as_ref()
        .expect("schema must have external source");
    assert_eq!(ext.tool, "buf");
    assert_eq!(ext.timeout_sec, 90);
    assert_eq!(ext.cwd.as_deref(), Some("proto/events"));
    assert!(schema.config.manifests.is_none());
}

#[test]
fn cascade_rule_floor_minor() {
    let dir = TempDir::new().unwrap();
    let cfg_path = dir.path().join("config.toml");
    fs::write(&cfg_path, FULL_OVERLAY).unwrap();
    let cfg = ConfigurationFile::get(&cfg_path).unwrap();

    let kotlin = cfg
        .release_units
        .iter()
        .find(|u| u.name == "sdk-kotlin")
        .expect("sdk-kotlin must be present");
    let cascade = kotlin
        .config
        .cascade_from
        .as_ref()
        .expect("kotlin SDK must have cascade_from");
    assert_eq!(cascade.source, "schema");
    assert_eq!(cascade.bump, "floor_minor");
}

#[test]
fn generic_regex_with_pattern_and_replace() {
    let dir = TempDir::new().unwrap();
    let cfg_path = dir.path().join("config.toml");
    fs::write(&cfg_path, FULL_OVERLAY).unwrap();
    let cfg = ConfigurationFile::get(&cfg_path).unwrap();

    let vtxt = cfg
        .release_units
        .iter()
        .find(|u| u.name == "version-txt")
        .expect("version-txt must be present");
    let manifests = explicit_manifests(vtxt.config.manifests.as_ref());
    let mf = &manifests[0];
    assert_eq!(mf.version_field, "generic_regex");
    assert_eq!(
        mf.regex_pattern.as_deref(),
        Some(r"^VERSION=(\d+\.\d+\.\d+)$")
    );
    assert_eq!(mf.regex_replace.as_deref(), Some("VERSION={version}"));
}

#[test]
fn hidden_visibility() {
    let dir = TempDir::new().unwrap();
    let cfg_path = dir.path().join("config.toml");
    fs::write(&cfg_path, FULL_OVERLAY).unwrap();
    let cfg = ConfigurationFile::get(&cfg_path).unwrap();

    let agg = cfg
        .release_units
        .iter()
        .find(|u| u.name == "internal-aggregator")
        .expect("internal-aggregator must be present");
    assert_eq!(agg.config.visibility.as_deref(), Some("hidden"));
}

#[test]
fn into_toml_round_trip_preserves_all_release_units() {
    let dir = TempDir::new().unwrap();
    let cfg_path = dir.path().join("config.toml");
    fs::write(&cfg_path, FULL_OVERLAY).unwrap();

    let cfg1 = ConfigurationFile::get(&cfg_path).unwrap();

    let serialised = cfg1.clone().into_toml().expect("into_toml must serialise");

    let cfg2_path = dir.path().join("roundtrip.toml");
    fs::write(&cfg2_path, &serialised).unwrap();
    let cfg2 = ConfigurationFile::get(&cfg2_path).unwrap();

    assert_eq!(
        cfg1.release_units.len(),
        cfg2.release_units.len(),
        "release_unit count mismatch after round-trip\n--- serialised ---\n{serialised}"
    );
    for (a, b) in cfg1.release_units.iter().zip(cfg2.release_units.iter()) {
        assert_eq!(a.name, b.name);
        assert_eq!(a.config.ecosystem, b.config.ecosystem);
        assert_eq!(
            explicit_manifests(a.config.manifests.as_ref()).len(),
            explicit_manifests(b.config.manifests.as_ref()).len()
        );
        assert_eq!(a.config.external.is_some(), b.config.external.is_some());
        assert_eq!(a.config.satellites, b.config.satellites);
        assert_eq!(a.config.tag_format, b.config.tag_format);
        assert_eq!(a.config.visibility, b.config.visibility);
    }

    assert_eq!(cfg1.ignore_paths.paths, cfg2.ignore_paths.paths);
    assert_eq!(cfg1.allow_uncovered.paths, cfg2.allow_uncovered.paths);
    assert_eq!(
        cfg1.ecosystems.cargo.workspace_mode,
        cfg2.ecosystems.cargo.workspace_mode
    );
}

#[test]
fn no_release_unit_section_means_empty_collections() {
    let dir = TempDir::new().unwrap();
    let cfg_path = dir.path().join("config.toml");
    fs::write(&cfg_path, "# empty user overlay\n").unwrap();

    let cfg = ConfigurationFile::get(&cfg_path).unwrap();
    assert!(cfg.release_units.is_empty());
    assert!(cfg.ignore_paths.paths.is_empty());
    assert!(cfg.allow_uncovered.paths.is_empty());
}
