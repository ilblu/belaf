//! TOML-facing syntax types for `[[release_unit]]`,
//! `[[release_unit_glob]]`, `[ignore_paths]`, `[allow_uncovered]`, and
//! `[ecosystems.*]`.
//!
//! Convention mirrors `core/config.rs`'s existing `syntax` module: serde
//! types here, normalisation into runtime types ([`super::ReleaseUnit`]
//! et al.) happens at `ConfigurationFile::get` time.
//!
//! ## TOML mapping notes
//!
//! The plan's §2.3 shows `[[release_unit.glob]]`. Unfortunately TOML
//! doesn't allow that syntax when `[[release_unit]]` is already an
//! array-of-tables — `release_unit` can be either an array OR a table
//! whose children include `.glob`, not both. We resolve this by
//! exposing two separate top-level keys:
//!
//! ```toml
//! [[release_unit]]
//! name = "aura"
//! # ...
//!
//! [[release_unit_glob]]
//! glob = "apps/services/*"
//! # ...
//! ```
//!
//! The runtime resolver merges both into a single
//! `Vec<ResolvedReleaseUnit>`.

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
    /// patterns and suggests an `[[release_unit_glob]]` entry.
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
    /// suggests a `[[group]]` over its members.
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
// `[[release_unit]]` — explicit form
// ---------------------------------------------------------------------------

/// One explicit `[[release_unit]]` table.
///
/// Source-form is split: `manifests` (zero or more) for
/// `VersionSource::Manifests`, OR `external` (exclusive) for
/// `VersionSource::External`. Validator ensures exactly one is set.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExplicitReleaseUnitConfig {
    pub name: String,

    pub ecosystem: String,

    /// Inline / table-array of manifest entries: `[release_unit.source]`
    /// `manifests = [{ path = "...", version_field = "..." }]` OR
    /// `[[release_unit.source.manifests]]`. Empty when source is
    /// External.
    #[serde(default)]
    pub source: SourceConfig,

    /// Optional satellite paths.
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

/// `[[release_unit.source]]` — split surface holding both
/// VersionSource variants. Validator ensures exactly one is set.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SourceConfig {
    /// `manifests = [...]` or `[[release_unit.source.manifests]]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub manifests: Vec<ManifestFileConfig>,

    /// `[release_unit.source.external]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<ExternalConfig>,
}

/// One entry under `source.manifests`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ManifestFileConfig {
    pub path: String,

    /// Optional — when omitted, inherits from the containing
    /// `[[release_unit]]`'s `ecosystem` field.
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

/// `[release_unit.source.external]`.
#[derive(Clone, Debug, Deserialize, Serialize)]
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
pub struct CascadeRuleConfig {
    pub source: String,
    /// `mirror` | `floor_patch` | `floor_minor` | `floor_major`.
    pub bump: String,
}

// ---------------------------------------------------------------------------
// `[[release_unit_glob]]` — glob form
// ---------------------------------------------------------------------------

/// One `[[release_unit_glob]]` table. Each glob expands at resolve-time
/// into N `ResolvedReleaseUnit`s — one per matching directory.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GlobReleaseUnitConfig {
    /// Directory glob, repo-relative. Templates `{path}` (matched
    /// dir), `{basename}` (last segment), `{parent}` (second-to-last)
    /// are available for the other fields below.
    pub glob: String,

    pub ecosystem: String,

    /// Required. Template-substitutable. First-existing-wins among
    /// `manifests` then `fallback_manifests`.
    pub manifests: Vec<String>,

    /// Optional. Tried in declaration order if all `manifests` paths
    /// fail to exist. First-existing-wins.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fallback_manifests: Vec<String>,

    /// Optional. Template-substitutable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub satellites: Vec<String>,

    /// `version_field` to use for each resolved manifest. If a single
    /// string, applied to all. If a list, must match `manifests`
    /// length 1:1. Defaults derived from `ecosystem` if omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_field: Option<String>,

    /// **Required.** Template producing the unit name. Must yield
    /// unique names across all expanded matches; collisions trigger
    /// edge case 20 (validator).
    pub name: String,

    /// Optional override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag_format: Option<String>,

    /// Optional `public | internal | hidden`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,

    /// Optional cascade rule applied to every expanded match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cascade_from: Option<CascadeRuleConfig>,
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
name = "aura"
ecosystem = "cargo"

[[source.manifests]]
path = "apps/services/aura/crates/bin/Cargo.toml"
version_field = "cargo_toml"
"#;
        let cfg: ExplicitReleaseUnitConfig = toml::from_str(toml_in).expect("must deserialize");
        assert_eq!(cfg.name, "aura");
        assert_eq!(cfg.ecosystem, "cargo");
        assert_eq!(cfg.source.manifests.len(), 1);
        assert!(cfg.source.external.is_none());
        assert_eq!(cfg.source.manifests[0].version_field, "cargo_toml");
        assert!(cfg.satellites.is_empty());

        // round-trip
        let serialized = toml::to_string(&cfg).expect("must serialize");
        let cfg2: ExplicitReleaseUnitConfig = toml::from_str(&serialized).expect("must round-trip");
        assert_eq!(cfg2.name, cfg.name);
        assert_eq!(cfg2.source.manifests[0].path, cfg.source.manifests[0].path);
    }

    #[test]
    fn explicit_release_unit_external_source() {
        let toml_in = r#"
name = "schema"
ecosystem = "external"
satellites = ["proto/events"]

[source.external]
tool = "buf"
read_command = "buf mod info --format json"
write_command = "buf mod set-version {version}"
cwd = "proto/events"
timeout_sec = 90
"#;
        let cfg: ExplicitReleaseUnitConfig = toml::from_str(toml_in).unwrap();
        assert_eq!(cfg.name, "schema");
        assert!(cfg.source.manifests.is_empty());
        let ext = cfg.source.external.expect("external must be set");
        assert_eq!(ext.tool, "buf");
        assert_eq!(ext.timeout_sec, 90);
        assert_eq!(cfg.satellites, vec!["proto/events".to_string()]);
    }

    #[test]
    fn external_default_timeout_60() {
        let toml_in = r#"
name = "x"
ecosystem = "external"

[source.external]
tool = "x"
read_command = "echo 1.0.0"
write_command = "echo new"
"#;
        let cfg: ExplicitReleaseUnitConfig = toml::from_str(toml_in).unwrap();
        assert_eq!(cfg.source.external.unwrap().timeout_sec, 60);
    }

    #[test]
    fn glob_release_unit_full() {
        let toml_in = r#"
glob = "apps/services/*"
ecosystem = "cargo"
manifests = ["{path}/crates/bin/Cargo.toml"]
fallback_manifests = ["{path}/crates/workers/Cargo.toml"]
satellites = ["{path}/crates"]
name = "{basename}"
"#;
        let cfg: GlobReleaseUnitConfig = toml::from_str(toml_in).unwrap();
        assert_eq!(cfg.glob, "apps/services/*");
        assert_eq!(cfg.manifests, vec!["{path}/crates/bin/Cargo.toml"]);
        assert_eq!(
            cfg.fallback_manifests,
            vec!["{path}/crates/workers/Cargo.toml"]
        );
        assert_eq!(cfg.name, "{basename}");
    }

    #[test]
    fn cascade_rule_inline() {
        let toml_in = r#"
name = "sdk-kotlin"
ecosystem = "jvm-library"
cascade_from = { source = "schema", bump = "floor_minor" }

[[source.manifests]]
path = "sdks/kotlin/gradle.properties"
version_field = "gradle_properties"
"#;
        let cfg: ExplicitReleaseUnitConfig = toml::from_str(toml_in).unwrap();
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
}
