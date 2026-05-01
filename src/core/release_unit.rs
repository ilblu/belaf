//! Release unit primitive — a directory or set of manifest files released
//! as one logical unit. See `BELAF_MASTER_PLAN.md` Part I.
//!
//! Two primitives shape the release model: `ReleaseUnit` (this file) and
//! `Group` (in [`crate::core::group`]). `ReleaseUnit + Group` compose to
//! cover every polyglot monorepo shape — single crates, hexagonal services,
//! Tauri triplets, JVM library SDKs, schema-driven SDK cascades, nested
//! workspaces.
//!
//! A `ReleaseUnit` is the config-side abstraction. Resolution turns each
//! one into a `ResolvedReleaseUnit` that lives in the graph (with stored `version`,
//! `prefix`, rewriters, etc.). The unit itself does not store the current
//! version — that is read at resolve-time from the unit's
//! [`VersionSource`].

use std::collections::HashMap;

use crate::core::git::repository::RepoPathBuf;
use crate::core::wire::known::Ecosystem;

pub mod cascade;
pub mod detector;
pub mod resolver;
pub mod syntax;
pub mod validator;

// ---------------------------------------------------------------------------
// ReleaseUnit
// ---------------------------------------------------------------------------

/// The externally-visible release atom. Each unit produces one git tag
/// and one entry in the emitted release manifest.
#[derive(Clone, Debug)]
pub struct ReleaseUnit {
    /// User-facing name. Drives the tag name and CLI display.
    pub name: String,

    /// What kind of release this is. Drives tag-format defaults and
    /// known-fields. Free-form via [`Ecosystem`] so unknown ecosystems
    /// round-trip through the wire format.
    pub ecosystem: Ecosystem,

    /// Where belaf reads + writes the version. Exactly one variant.
    pub source: VersionSource,

    /// Paths whose commits attribute to this unit but receive no
    /// version writes. Hexagonal services use this to attach
    /// `crates/api`, `crates/core`, etc. to the bin-crate ReleaseUnit.
    pub satellites: Vec<RepoPathBuf>,

    /// Override the per-ecosystem default tag-format template.
    /// Precedence: this > `Group.tag_format` > legacy `[projects.<name>]
    /// .tag_format` > ecosystem default.
    pub tag_format: Option<String>,

    /// Visibility — whether this unit appears in TUI lists, the emitted
    /// manifest, and gets a git tag.
    pub visibility: Visibility,

    /// Optional cascade rule — bump this unit when a named upstream
    /// unit bumps. SDKs use this to follow a schema unit.
    pub cascade_from: Option<CascadeRule>,
}

// ---------------------------------------------------------------------------
// VersionSource
// ---------------------------------------------------------------------------

/// Two variants. A [`ReleaseUnit`] picks exactly one.
#[derive(Clone, Debug)]
pub enum VersionSource {
    /// Belaf reads + writes the version directly in N files in lockstep.
    /// `N >= 1`. For `N > 1`, `MultiManifestRewriter` writes all of
    /// them with heal-forward semantics.
    Manifests(Vec<ManifestFile>),

    /// Belaf shells out to a tool that owns the manifest. Used for
    /// schema-first codegen, custom build phases, anything needing
    /// "run a command at prepare-time".
    External(ExternalVersioner),
}

impl VersionSource {
    /// Does this source have a Manifests variant?
    pub fn is_manifests(&self) -> bool {
        matches!(self, Self::Manifests(_))
    }

    /// Does this source have an External variant?
    pub fn is_external(&self) -> bool {
        matches!(self, Self::External(_))
    }
}

// ---------------------------------------------------------------------------
// ManifestFile
// ---------------------------------------------------------------------------

/// One file that belaf reads + writes the version in. A
/// [`VersionSource::Manifests`] holds 1 or more of these; multi-file
/// units (Tauri legacy multi-file, etc.) write all of them in lockstep.
#[derive(Clone, Debug)]
pub struct ManifestFile {
    /// Repo-relative path to the manifest file.
    pub path: RepoPathBuf,

    /// Which ecosystem this file belongs to. May differ from the
    /// containing [`ReleaseUnit`]'s primary ecosystem (Tauri's
    /// `package.json` is npm-shaped inside a `tauri` unit).
    pub ecosystem: Ecosystem,

    /// How to read + write the version inside this file.
    pub version_field: VersionFieldSpec,
}

// ---------------------------------------------------------------------------
// VersionFieldSpec
// ---------------------------------------------------------------------------

/// One per supported in-file version-field shape.
///
/// Plan §3 lists exactly five:
///
/// 1. `CargoToml`        — `[package].version` or `[workspace.package].version`
/// 2. `NpmPackageJson`   — JSON `$.version`
/// 3. `TauriConfJson`    — JSON5-tolerant `$.version` via regex
/// 4. `GradleProperties` — `^version=(.+)$` line in `gradle.properties`
/// 5. `GenericRegex`     — escape hatch with one capture group
#[derive(Clone, Debug)]
pub enum VersionFieldSpec {
    /// `Cargo.toml` `[package].version` or `[workspace.package].version`.
    /// Implementation: `toml-edit` (preserves comments + ordering).
    CargoToml,

    /// `package.json` `$.version`. Implementation: `serde_json` with
    /// indent-detection (preserves 2-space vs 4-space style).
    NpmPackageJson,

    /// `tauri.conf.json` `$.version` (JSON5-tolerant). Implementation:
    /// regex-based field replace, **not** serde-roundtrip — preserves
    /// comments + unquoted keys that JSON5 allows.
    TauriConfJson,

    /// `gradle.properties` `^version=(.+)$` line. Implementation:
    /// regex (multiline). 0 matches → hard error (edge case 18); 1 →
    /// bump; N > 1 → replace all (idempotent), warn-log line numbers
    /// (edge case 21).
    GradleProperties,

    /// Escape hatch: custom regex with exactly one capture group plus
    /// a `{version}`-substituting replace template.
    GenericRegex {
        /// Must contain exactly one capture group; validated at
        /// config-load time.
        pattern: String,
        /// `{version}` is substituted at write time.
        replace: String,
    },
}

impl VersionFieldSpec {
    /// Stable wire-format key for this spec. Used in TOML
    /// (de)serialization via `version_field = "..."`.
    pub fn wire_key(&self) -> &'static str {
        match self {
            Self::CargoToml => "cargo_toml",
            Self::NpmPackageJson => "npm_package_json",
            Self::TauriConfJson => "tauri_conf_json",
            Self::GradleProperties => "gradle_properties",
            Self::GenericRegex { .. } => "generic_regex",
        }
    }
}

// ---------------------------------------------------------------------------
// ExternalVersioner
// ---------------------------------------------------------------------------

/// The [`VersionSource::External`] payload. Belaf calls
/// `read_command` to learn the current version, runs `write_command`
/// with substitutions to perform the bump, then re-runs `read_command`
/// to confirm. **No format-introspection on belaf's side — the external
/// tool is trusted.**
#[derive(Clone, Debug)]
pub struct ExternalVersioner {
    /// Descriptive label for the tool — diagnostic only.
    pub tool: String,

    /// Shell command that prints the current version to stdout.
    pub read_command: String,

    /// Shell command that performs the bump. Substitutions:
    /// `{version}`, `{bump}`, `{name}`.
    pub write_command: String,

    /// Working directory for both commands. Defaults to the repo root
    /// when `None`.
    pub cwd: Option<RepoPathBuf>,

    /// Hard timeout in seconds. Defaults to 60.
    pub timeout_sec: u64,

    /// Extra environment variables to pass to both subprocesses, on
    /// top of the standard `BELAF_VERSION_*` and `BELAF_UNIT_*`
    /// belaf-injected ones.
    pub env: HashMap<String, String>,
}

impl Default for ExternalVersioner {
    fn default() -> Self {
        Self {
            tool: String::new(),
            read_command: String::new(),
            write_command: String::new(),
            cwd: None,
            timeout_sec: 60,
            env: HashMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// CascadeRule
// ---------------------------------------------------------------------------

/// A bump-this-unit-when-source-bumps rule. SDKs use this to follow a
/// schema unit. Cycle detection at config-load via
/// `petgraph::algo::tarjan_scc` produces full SCC membership for
/// actionable error messages.
#[derive(Clone, Debug)]
pub struct CascadeRule {
    /// Name of the upstream [`ReleaseUnit`] whose bump triggers this
    /// one.
    pub source: String,

    /// How big the cascading bump should be relative to the source's
    /// bump.
    pub bump: CascadeBumpStrategy,
}

/// How a cascade-rooted [`ReleaseUnit`] bumps when its source bumps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CascadeBumpStrategy {
    /// Same bump as the source (source minor → this minor, source
    /// major → this major).
    Mirror,

    /// At least patch; escalate if source is bigger (source minor →
    /// this minor, source major → this major).
    FloorPatch,

    /// At least minor; escalate if source is major.
    FloorMinor,

    /// Always major when source bumps. Rare — used for SDKs that must
    /// always be a major release for any schema change.
    FloorMajor,
}

impl CascadeBumpStrategy {
    /// Stable wire-format key. Used in TOML (de)serialization via
    /// `bump = "..."`.
    pub fn wire_key(&self) -> &'static str {
        match self {
            Self::Mirror => "mirror",
            Self::FloorPatch => "floor_patch",
            Self::FloorMinor => "floor_minor",
            Self::FloorMajor => "floor_major",
        }
    }
}

// ---------------------------------------------------------------------------
// Visibility
// ---------------------------------------------------------------------------

/// Whether a [`ReleaseUnit`] appears in TUI lists, the emitted
/// manifest, and gets a git tag. Plan §6.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Visibility {
    /// Default. Appears in TUI lists, the emitted manifest, and gets a
    /// git tag. Normal release artifacts.
    #[default]
    Public,

    /// Visible in TUI + manifest, but **no git tag is created**. Used
    /// for internal SDKs that act as cascade sources for other units
    /// but aren't released to a registry themselves.
    Internal,

    /// Hidden everywhere — not in TUI, not in the manifest, no tag.
    /// Used for pure satellite-aggregator units that exist only to
    /// attribute a satellite tree to a parent. Rare.
    Hidden,
}

impl Visibility {
    /// Stable wire-format key.
    pub fn wire_key(&self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Internal => "internal",
            Self::Hidden => "hidden",
        }
    }

    /// Parse from the lowercase wire form. None for unknown strings.
    pub fn from_wire(s: &str) -> Option<Self> {
        match s {
            "public" => Some(Self::Public),
            "internal" => Some(Self::Internal),
            "hidden" => Some(Self::Hidden),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Resolved units — the runtime form after [`resolver::resolve`] runs.
// Each carries a [`ResolveOrigin`] so `belaf config explain` can attribute
// it back to the user's config.
// ---------------------------------------------------------------------------

/// A [`ReleaseUnit`] plus provenance metadata (which config block /
/// glob-expansion / detector produced it).
#[derive(Clone, Debug)]
pub struct ResolvedReleaseUnit {
    /// The runtime release unit.
    pub unit: ReleaseUnit,

    /// Where this resolved unit came from. Used by `belaf config
    /// explain` (Phase K) to attribute each unit back to a TOML line
    /// or detector pattern.
    pub origin: ResolveOrigin,
}

/// Provenance of a [`ResolvedReleaseUnit`].
#[derive(Clone, Debug)]
pub enum ResolveOrigin {
    /// Came from an explicit `[[release_unit]]` block at the given
    /// index in the config.
    Explicit { config_index: usize },

    /// Came from a `[[release_unit_glob]]` block at the given index,
    /// matched the given repo-relative directory.
    Glob {
        glob_index: usize,
        matched_path: RepoPathBuf,
    },

    /// Came from an init detector (Phase F). Carries the detector's
    /// stable label.
    Detected { detector: &'static str },
}

impl ResolveOrigin {
    /// Short human-readable label for diagnostics.
    pub fn label(&self) -> String {
        match self {
            Self::Explicit { config_index } => {
                format!("explicit [[release_unit]] #{config_index}")
            }
            Self::Glob {
                glob_index,
                matched_path,
            } => format!(
                "glob [[release_unit_glob]] #{} matched {}",
                glob_index,
                matched_path.escaped()
            ),
            Self::Detected { detector } => format!("detector {detector}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests — fast unit-level coverage of the plain types. Integration tests
// live in `tests/test_release_unit_config.rs`.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_field_spec_wire_keys_are_distinct_and_stable() {
        let keys: Vec<_> = [
            VersionFieldSpec::CargoToml.wire_key(),
            VersionFieldSpec::NpmPackageJson.wire_key(),
            VersionFieldSpec::TauriConfJson.wire_key(),
            VersionFieldSpec::GradleProperties.wire_key(),
            VersionFieldSpec::GenericRegex {
                pattern: String::new(),
                replace: String::new(),
            }
            .wire_key(),
        ]
        .into_iter()
        .collect();

        let mut sorted = keys.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), keys.len(), "wire keys must be unique");
    }

    #[test]
    fn cascade_strategy_wire_keys_are_distinct() {
        let keys = [
            CascadeBumpStrategy::Mirror.wire_key(),
            CascadeBumpStrategy::FloorPatch.wire_key(),
            CascadeBumpStrategy::FloorMinor.wire_key(),
            CascadeBumpStrategy::FloorMajor.wire_key(),
        ];
        let mut sorted: Vec<_> = keys.into_iter().collect();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), keys.len());
    }

    #[test]
    fn visibility_default_is_public() {
        assert_eq!(Visibility::default(), Visibility::Public);
    }

    #[test]
    fn version_source_classifiers() {
        let manifests = VersionSource::Manifests(vec![]);
        let external = VersionSource::External(ExternalVersioner::default());
        assert!(manifests.is_manifests() && !manifests.is_external());
        assert!(external.is_external() && !external.is_manifests());
    }

    #[test]
    fn external_versioner_default_timeout_is_60s() {
        assert_eq!(ExternalVersioner::default().timeout_sec, 60);
    }
}
