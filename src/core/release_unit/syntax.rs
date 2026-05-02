//! TOML-facing syntax for `[release_unit.<name>]`, `[ignore_paths]`,
//! `[allow_uncovered]`, and `[ecosystems.*]`.
//!
//! Released as the 1.0 wire shape: one named-entry form, no `[[release_unit]]`
//! / `[[release_unit_glob]]` array-of-tables, no dual `[[group]]` / `[group.<id>]`.
//! Glob-expansion is opt-in via the `glob` field on a regular release_unit
//! entry; the resolver dispatches.
//!
//! ## Example
//!
//! ```toml
//! [release_unit.my-lib]
//! ecosystem = "cargo"
//! manifests = [{ path = "Cargo.toml", version_field = "cargo_toml" }]
//!
//! [release_unit.tauri-apps]
//! ecosystem = "tauri"
//! glob = "apps/*"
//! name = "{basename}"
//! manifests = ["{path}/src-tauri/Cargo.toml"]
//! fallback_manifests = ["{path}/Cargo.toml"]
//!
//! [release_unit.kotlin-sdk]
//! ecosystem = "external"
//! external = { tool = "gradle", read_command = "./gradlew :sdk:printVersion -q", write_command = "./gradlew :sdk:setVersion -PnewVersion={version}" }
//! ```

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Container types embedded in `core::config::syntax::ReleaseConfiguration`
// ---------------------------------------------------------------------------

/// `[ignore_paths]` — paths belaf does not scan inside at all (no
/// detector, no commit attribution).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct IgnorePathsConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
}

impl IgnorePathsConfig {
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }
}

/// `[allow_uncovered]` — paths belaf scans but explicitly accepts as
/// not mapping to any ReleaseUnit. Mobile apps go here on init.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AllowUncoveredConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
}

impl AllowUncoveredConfig {
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Per-ecosystem smart defaults: `[ecosystems.<name>]`
// ---------------------------------------------------------------------------

/// `[ecosystems.cargo]` — Cargo-specific knobs.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct EcosystemCargoConfig {
    /// `auto` (default) | `always` | `never`. When `auto`, the
    /// hexagonal cargo detector flags `D/crates/{bin,lib,workers}`
    /// patterns and suggests a glob-form `[release_unit.<name>]` entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hexagonal_pattern: Option<String>,

    /// `auto` (default) | `single` | `separate`. Overrides the
    /// existing single-vs-separate workspace heuristic in
    /// `cargo.rs::is_workspace_project`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_mode: Option<String>,
}

/// `[ecosystems.npm]` — npm-specific knobs.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct EcosystemNpmConfig {
    /// `auto` (default) detects a top-level `workspaces` field and
    /// suggests a `[group.<id>]` over its members.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_workspaces: Option<String>,
}

/// `[ecosystems.tauri]` — Tauri detector knobs.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct EcosystemTauriConfig {
    /// Detect `package.json + src-tauri/Cargo.toml +
    /// src-tauri/tauri.conf.json` triplets. Default true.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub detect_triplet: bool,

    /// Recommend the single-source pattern (tauri.conf.json refs
    /// `"../package.json"`). Default true.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub prefer_single_source: bool,
}

fn default_true() -> bool {
    true
}
fn is_true(b: &bool) -> bool {
    *b
}

/// `[ecosystems.jvm_library]` — JVM library detector knobs.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct EcosystemJvmLibraryConfig {
    /// Default location relative to the bundle root. Defaults to
    /// `gradle.properties`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gradle_properties_path: Option<String>,
}

/// Aggregate `[ecosystems]` table — one optional sub-table per
/// known ecosystem with knobs.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct EcosystemsConfig {
    #[serde(default, skip_serializing_if = "is_default_cargo")]
    pub cargo: EcosystemCargoConfig,

    #[serde(default, skip_serializing_if = "is_default_npm")]
    pub npm: EcosystemNpmConfig,

    #[serde(default, skip_serializing_if = "is_default_tauri")]
    pub tauri: EcosystemTauriConfig,

    #[serde(
        default,
        rename = "jvm_library",
        skip_serializing_if = "is_default_jvm_library"
    )]
    pub jvm_library: EcosystemJvmLibraryConfig,
}

fn is_default_cargo(c: &EcosystemCargoConfig) -> bool {
    c.hexagonal_pattern.is_none() && c.workspace_mode.is_none()
}
fn is_default_npm(c: &EcosystemNpmConfig) -> bool {
    c.sync_workspaces.is_none()
}
fn is_default_tauri(c: &EcosystemTauriConfig) -> bool {
    c.detect_triplet && c.prefer_single_source
}
fn is_default_jvm_library(c: &EcosystemJvmLibraryConfig) -> bool {
    c.gradle_properties_path.is_none()
}

impl EcosystemsConfig {
    pub fn is_empty(&self) -> bool {
        is_default_cargo(&self.cargo)
            && is_default_npm(&self.npm)
            && is_default_tauri(&self.tauri)
            && is_default_jvm_library(&self.jvm_library)
    }
}

// ---------------------------------------------------------------------------
// `[release_unit.<name>]` — single named-entry form.
// ---------------------------------------------------------------------------

/// Wire-form for a `[release_unit.<name>]` entry. The TOML key carries
/// the name; the rest of the fields live here. Setting `glob` switches
/// the entry into glob-expansion mode (one config block produces N
/// units, one per matching directory).
///
/// Source-form is split: `manifests` (zero or more) for
/// `VersionSource::Manifests`, OR `external` (exclusive) for
/// `VersionSource::External`. Validator ensures exactly one is set.
///
/// `deny_unknown_fields` so that a typo like `versoin_field` or
/// `tag_formet` surfaces as a config error instead of being silently
/// dropped at deserialise time.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseUnitConfig {
    pub ecosystem: String,

    /// **Glob form only** — name template (`{basename}` etc).
    /// When `glob` is set, this overrides the TOML key as the per-match
    /// unit name (because one block expands to many units). When `glob`
    /// is not set, the TOML key is the name and this field must not be
    /// set (validator enforces).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Glob pattern (repo-relative). When set, this entry expands at
    /// resolve-time into N units (one per matching directory).
    /// Templates `{path}`, `{basename}`, `{parent}` are available in
    /// `name`, `manifests`, `fallback_manifests`, and `satellites`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glob: Option<String>,

    /// Source: manifest list. Format depends on `glob`:
    ///   - Non-glob form: `manifests = [{ path = "...", version_field = "..." }, ...]`
    ///   - Glob form: `manifests = ["{path}/Cargo.toml", ...]` (templated).
    ///
    /// Mutually exclusive with `external`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifests: Option<ManifestList>,

    /// Source: external versioner (gradle plugin, custom script).
    /// Mutually exclusive with `manifests`. Glob-form entries cannot
    /// use `external` — each match would need its own command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<ExternalConfig>,

    /// **Glob form only** — fallback templates tried in declaration
    /// order if no `manifests` template resolves to an existing file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fallback_manifests: Vec<String>,

    /// **Glob form only** — `version_field` key applied to all
    /// manifests after templates resolve. Defaults to the ecosystem's
    /// canonical version_field if omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_field: Option<String>,

    /// Optional satellite paths (template-substituted in glob form).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub satellites: Vec<String>,

    /// Override per-ecosystem default tag-format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag_format: Option<String>,

    /// `public` (default) | `internal` | `hidden`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,

    /// Optional cascade rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cascade_from: Option<CascadeRuleConfig>,
}

impl ReleaseUnitConfig {
    pub fn is_glob(&self) -> bool {
        self.glob.is_some()
    }
}

/// Manifest list. The two forms are TOML-distinguishable: explicit
/// form uses inline-tables, glob form uses bare strings. Untagged
/// enum so users don't need to write `manifests = { explicit = [...] }`
/// or similar.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ManifestList {
    /// Non-glob form: per-manifest config.
    Explicit(Vec<ManifestFileConfig>),
    /// Glob form: template strings (one `version_field` applied to all
    /// via the unit-level `version_field` field).
    Templates(Vec<String>),
}

/// One entry under non-glob `manifests = [...]`. `deny_unknown_fields`
/// so a typo'd `path_pattern` or `regex_replece` fails at config-load
/// time instead of being silently ignored.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestFileConfig {
    pub path: String,

    /// Optional — when omitted, inherits from the containing entry's
    /// `ecosystem` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ecosystem: Option<String>,

    /// `cargo_toml` | `npm_package_json` | `tauri_conf_json` |
    /// `gradle_properties` | `generic_regex`. For `generic_regex`,
    /// the `regex_pattern` and `regex_replace` fields below must
    /// also be set.
    pub version_field: String,

    /// Required iff `version_field = "generic_regex"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regex_pattern: Option<String>,

    /// Required iff `version_field = "generic_regex"`. `{version}`
    /// is substituted at write time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regex_replace: Option<String>,
}

/// External versioner — `external = { tool = "...", ... }` inline or
/// `[release_unit.<name>.external]` table form.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalConfig {
    pub tool: String,
    pub read_command: String,
    pub write_command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default = "default_external_timeout", skip_serializing_if = "is_60")]
    pub timeout_sec: u64,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
}

fn default_external_timeout() -> u64 {
    60
}
fn is_60(n: &u64) -> bool {
    *n == 60
}

/// `cascade_from = { source = "...", bump = "..." }` inline form.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CascadeRuleConfig {
    pub source: String,
    /// `mirror` | `floor_patch` | `floor_minor` | `floor_major`.
    pub bump: String,
}

// ---------------------------------------------------------------------------
// Tests — round-trip + default-handling.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_release_unit_minimal_roundtrip() {
        let toml_in = r#"
ecosystem = "cargo"
manifests = [{ path = "apps/services/aura/crates/bin/Cargo.toml", version_field = "cargo_toml" }]
"#;
        let cfg: ReleaseUnitConfig = toml::from_str(toml_in).expect("must deserialize");
        assert_eq!(cfg.ecosystem, "cargo");
        assert!(!cfg.is_glob());
        let Some(ManifestList::Explicit(manifests)) = &cfg.manifests else {
            panic!("expected explicit manifests");
        };
        assert_eq!(manifests.len(), 1);
        assert!(cfg.external.is_none());
        assert_eq!(manifests[0].version_field, "cargo_toml");
        assert!(cfg.satellites.is_empty());

        // round-trip
        let serialized = toml::to_string(&cfg).expect("must serialize");
        let cfg2: ReleaseUnitConfig = toml::from_str(&serialized).expect("must round-trip");
        let Some(ManifestList::Explicit(manifests2)) = &cfg2.manifests else {
            panic!("expected explicit manifests after roundtrip");
        };
        assert_eq!(manifests2[0].path, manifests[0].path);
    }

    #[test]
    fn explicit_release_unit_external_source() {
        let toml_in = r#"
ecosystem = "external"
satellites = ["proto/events"]

[external]
tool = "buf"
read_command = "buf mod info --format json"
write_command = "buf mod set-version {version}"
cwd = "proto/events"
timeout_sec = 90
"#;
        let cfg: ReleaseUnitConfig = toml::from_str(toml_in).unwrap();
        assert_eq!(cfg.ecosystem, "external");
        assert!(cfg.manifests.is_none());
        let ext = cfg.external.expect("external must be set");
        assert_eq!(ext.tool, "buf");
        assert_eq!(ext.timeout_sec, 90);
        assert_eq!(cfg.satellites, vec!["proto/events".to_string()]);
    }

    #[test]
    fn external_default_timeout_60() {
        let toml_in = r#"
ecosystem = "external"

[external]
tool = "x"
read_command = "echo 1.0.0"
write_command = "echo new"
"#;
        let cfg: ReleaseUnitConfig = toml::from_str(toml_in).unwrap();
        assert_eq!(cfg.external.unwrap().timeout_sec, 60);
    }

    #[test]
    fn glob_release_unit_full() {
        let toml_in = r#"
ecosystem = "cargo"
glob = "apps/services/*"
name = "{basename}"
manifests = ["{path}/crates/bin/Cargo.toml"]
fallback_manifests = ["{path}/crates/workers/Cargo.toml"]
satellites = ["{path}/crates"]
"#;
        let cfg: ReleaseUnitConfig = toml::from_str(toml_in).unwrap();
        assert!(cfg.is_glob());
        assert_eq!(cfg.glob.as_deref(), Some("apps/services/*"));
        assert_eq!(cfg.name.as_deref(), Some("{basename}"));
        let Some(ManifestList::Templates(templates)) = &cfg.manifests else {
            panic!("glob form expects ManifestList::Templates");
        };
        assert_eq!(templates, &vec!["{path}/crates/bin/Cargo.toml".to_string()]);
        assert_eq!(
            cfg.fallback_manifests,
            vec!["{path}/crates/workers/Cargo.toml"]
        );
    }

    #[test]
    fn cascade_rule_inline() {
        let toml_in = r#"
ecosystem = "jvm-library"
cascade_from = { source = "schema", bump = "floor_minor" }
manifests = [{ path = "sdks/kotlin/gradle.properties", version_field = "gradle_properties" }]
"#;
        let cfg: ReleaseUnitConfig = toml::from_str(toml_in).unwrap();
        let cascade = cfg.cascade_from.expect("cascade_from must be set");
        assert_eq!(cascade.source, "schema");
        assert_eq!(cascade.bump, "floor_minor");
    }

    #[test]
    fn ignore_paths_and_allow_uncovered_minimal() {
        let toml_in = r#"
paths = ["examples/", "internal-tools/"]
"#;
        let cfg: IgnorePathsConfig = toml::from_str(toml_in).unwrap();
        assert_eq!(cfg.paths.len(), 2);
        assert_eq!(cfg.paths[0], "examples/");

        let cfg2: AllowUncoveredConfig = toml::from_str(toml_in).unwrap();
        assert_eq!(cfg2.paths, cfg.paths);
    }

    #[test]
    fn ecosystems_config_full() {
        let toml_in = r#"
[cargo]
hexagonal_pattern = "auto"
workspace_mode = "separate"

[npm]
sync_workspaces = "auto"

[tauri]
detect_triplet = true
prefer_single_source = true

[jvm_library]
gradle_properties_path = "gradle.properties"
"#;
        let cfg: EcosystemsConfig = toml::from_str(toml_in).unwrap();
        assert_eq!(cfg.cargo.workspace_mode.as_deref(), Some("separate"));
        assert_eq!(cfg.npm.sync_workspaces.as_deref(), Some("auto"));
        assert!(cfg.tauri.detect_triplet);
        assert_eq!(
            cfg.jvm_library.gradle_properties_path.as_deref(),
            Some("gradle.properties")
        );
    }

    #[test]
    fn manifest_file_with_generic_regex() {
        let toml_in = r#"
path = "config/version.txt"
ecosystem = "external"
version_field = "generic_regex"
regex_pattern = "^VERSION=(\\d+\\.\\d+\\.\\d+)$"
regex_replace = "VERSION={version}"
"#;
        let cfg: ManifestFileConfig = toml::from_str(toml_in).unwrap();
        assert_eq!(cfg.version_field, "generic_regex");
        assert!(cfg.regex_pattern.is_some());
        assert!(cfg.regex_replace.is_some());
    }

    #[test]
    fn name_field_only_set_when_glob_set() {
        // The validator enforces this, not deserialize. Here we just
        // pin the deserialise path is happy with `name` set in non-glob
        // form (so the validator catches it later).
        let toml_in = r#"
ecosystem = "cargo"
name = "weird"
manifests = [{ path = "Cargo.toml", version_field = "cargo_toml" }]
"#;
        let cfg: ReleaseUnitConfig = toml::from_str(toml_in).unwrap();
        assert_eq!(cfg.name.as_deref(), Some("weird"));
        assert!(!cfg.is_glob());
    }
}
