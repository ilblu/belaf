//! Resolver / validator errors for the `release_unit` config.
//!
//! Maps directly to Part VI's 21 edge cases. Each variant carries
//! enough context to produce an actionable diagnostic at the CLI level
//! — paths, conflicting names, glob templates, etc.

use thiserror::Error as ThisError;

#[derive(Debug, ThisError, PartialEq, Eq)]
pub enum ResolverError {
    /// Edge case 1 — a `[[release_unit]]` (or `manifests[i]`) refers to
    /// a path that does not exist in the working tree.
    #[error("release_unit `{unit}`: path `{path}` does not exist in the working tree")]
    PathDoesNotExist { unit: String, path: String },

    /// Edge case 4 — none of the `manifests` paths exist and no
    /// `fallback_manifests` saved it. Lists every path tried.
    #[error("release_unit `{unit}`: none of the listed manifest paths exist (tried: {tried:?})")]
    AllManifestsAndFallbacksMissing { unit: String, tried: Vec<String> },

    /// Edge case 5 — declared ecosystem doesn't match the
    /// `version_field` shape (e.g. ecosystem="npm" with
    /// version_field="cargo_toml").
    #[error(
        "release_unit `{unit}`: ecosystem `{ecosystem}` is incompatible with version_field `{version_field}` ({hint})"
    )]
    EcosystemMismatchVersionField {
        unit: String,
        ecosystem: String,
        version_field: String,
        hint: String,
    },

    /// Edge case 8 — two glob blocks expand to the same path.
    #[error(
        "release_unit_glob conflict: globs `{glob_a}` and `{glob_b}` both match path `{path}` — disambiguate by removing one or making them more specific"
    )]
    TwoGlobsSamePath {
        path: String,
        glob_a: String,
        glob_b: String,
    },

    /// Edge case 9 — one bundle path is a strict prefix of another.
    /// Nested bundles are forbidden.
    #[error(
        "release_unit `{outer}` (path `{outer_path}`) contains nested release_unit `{inner}` (path `{inner_path}`) — nested bundles are not supported"
    )]
    NestedBundlePath {
        outer: String,
        outer_path: String,
        inner: String,
        inner_path: String,
    },

    /// Edge case 10 — bundle path equals the repo root. Degenerate
    /// configuration.
    #[error("release_unit `{unit}`: bundle path equals repo root — use ecosystem-level scanning instead")]
    BundlePathIsRepoRoot { unit: String },

    /// Edge case 12 — Cargo's `version.workspace = true` set on a crate
    /// whose workspace root has no `[workspace.package].version`.
    #[error("release_unit `{unit}`: Cargo manifest at `{path}` uses `version.workspace = true` but the root workspace has no `[workspace.package].version` set — add it to the root Cargo.toml")]
    WorkspaceVersionInheritedButRootMissing { unit: String, path: String },

    /// Edge case 13 — a manifest file exists but cannot be parsed.
    #[error("release_unit `{unit}`: manifest `{path}` could not be parsed: {reason}")]
    ManifestCorrupt {
        unit: String,
        path: String,
        reason: String,
    },

    /// Edge case 18 — `gradle.properties` exists but has no
    /// `^version=` line.
    #[error("release_unit `{unit}`: `{path}` has no `version=...` line — add `version=0.1.0` to enable belaf-managed releases")]
    GradlePropertiesNoVersionLine { unit: String, path: String },

    /// Edge case 19 — cascade cycle. Carries the full SCC membership.
    #[error("cascade cycle detected: {}", members.join(" → "))]
    CascadeCycle { members: Vec<String> },

    /// Edge case 20 — two units (or one glob expansion) produce the
    /// same `name` from different paths.
    #[error(
        "release_unit name `{name}` is produced by multiple paths: {paths:?} — pick a more specific glob `name` template (e.g. `{{parent}}-{{basename}}`) or use explicit `[[release_unit]]` entries"
    )]
    NameCollision { name: String, paths: Vec<String> },

    /// Edge case 21 is a warn (not an error) → no enum variant; the
    /// resolver / rewriter logs it.
    /// `gradle.properties` with multiple `version=` lines is handled
    /// by `gradle_properties.rs` at write-time.

    /// Source-related: both `manifests` and `external` set on the
    /// same `[[release_unit]]`.
    #[error("release_unit `{unit}`: only one of `source.manifests` or `source.external` may be set — found both")]
    SourceBothSet { unit: String },

    /// Source-related: neither `manifests` nor `external` set.
    #[error("release_unit `{unit}`: must set either `source.manifests` or `source.external`")]
    SourceNotSet { unit: String },

    /// Visibility / ecosystem / version_field unknown values.
    #[error("release_unit `{unit}`: unknown {field} `{value}` (allowed: {allowed})")]
    UnknownEnumValue {
        unit: String,
        field: &'static str,
        value: String,
        allowed: &'static str,
    },

    /// `generic_regex` version_field without `regex_pattern` /
    /// `regex_replace`.
    #[error("release_unit `{unit}`: `version_field = \"generic_regex\"` requires both `regex_pattern` and `regex_replace`")]
    GenericRegexMissingPatternOrReplace { unit: String },

    /// `generic_regex` pattern doesn't have exactly one capture group.
    #[error("release_unit `{unit}`: regex pattern `{pattern}` must contain exactly one capture group, found {found}")]
    GenericRegexCaptureCount {
        unit: String,
        pattern: String,
        found: usize,
    },

    /// Unknown template variable used in a glob template field.
    #[error("release_unit_glob #{glob_index}: unknown template variable `{{{var}}}` — supported: {{path}}, {{basename}}, {{parent}}")]
    UnknownTemplateVar { glob_index: usize, var: String },

    /// `manifests[i].path` after template substitution still contains
    /// unresolved `{...}` placeholders.
    #[error("release_unit_glob #{glob_index}: template `{template}` did not fully substitute (result: `{result}`)")]
    TemplateNotFullySubstituted {
        glob_index: usize,
        template: String,
        result: String,
    },

    /// Cascade source name not found among the resolved release units.
    /// Field named `cascade_source` (not `source`) so thiserror's
    /// automatic-source-detection doesn't fire on a plain `String`.
    #[error("release_unit `{unit}`: cascade_from.source `{cascade_source}` is not a known release_unit name")]
    CascadeSourceUnknown {
        unit: String,
        cascade_source: String,
    },

    /// Two globs produce two units with the same name on the same
    /// matched path — this is rare and caught here distinctly so the
    /// error message can mention both globs.
    #[error("two release_unit_glob blocks (#{glob_a} and #{glob_b}) both produce a unit named `{name}` for path `{path}`")]
    TwoGlobsSameName {
        glob_a: usize,
        glob_b: usize,
        name: String,
        path: String,
    },

    /// Unknown bump-strategy in `cascade_from.bump`.
    #[error("release_unit `{unit}`: unknown cascade bump strategy `{strategy}` (allowed: mirror, floor_patch, floor_minor, floor_major)")]
    UnknownCascadeBumpStrategy { unit: String, strategy: String },

    /// Path normalization or canonicalization failure.
    #[error("release_unit `{unit}`: path `{path}` is invalid: {reason}")]
    InvalidPath {
        unit: String,
        path: String,
        reason: String,
    },
}

impl ResolverError {
    /// Stable label used by tests / `belaf config explain` to
    /// identify the rule. Every variant maps to one stable string,
    /// independent of error message wording.
    pub fn rule(&self) -> &'static str {
        match self {
            Self::PathDoesNotExist { .. } => "path_does_not_exist",
            Self::AllManifestsAndFallbacksMissing { .. } => "all_manifests_and_fallbacks_missing",
            Self::EcosystemMismatchVersionField { .. } => "ecosystem_mismatch_version_field",
            Self::TwoGlobsSamePath { .. } => "two_globs_same_path",
            Self::NestedBundlePath { .. } => "nested_bundle_path",
            Self::BundlePathIsRepoRoot { .. } => "bundle_path_is_repo_root",
            Self::WorkspaceVersionInheritedButRootMissing { .. } => {
                "workspace_version_inherited_but_root_missing"
            }
            Self::ManifestCorrupt { .. } => "manifest_corrupt",
            Self::GradlePropertiesNoVersionLine { .. } => "gradle_properties_no_version_line",
            Self::CascadeCycle { .. } => "cascade_cycle",
            Self::NameCollision { .. } => "name_collision",
            Self::SourceBothSet { .. } => "source_both_set",
            Self::SourceNotSet { .. } => "source_not_set",
            Self::UnknownEnumValue { .. } => "unknown_enum_value",
            Self::GenericRegexMissingPatternOrReplace { .. } => {
                "generic_regex_missing_pattern_or_replace"
            }
            Self::GenericRegexCaptureCount { .. } => "generic_regex_capture_count",
            Self::UnknownTemplateVar { .. } => "unknown_template_var",
            Self::TemplateNotFullySubstituted { .. } => "template_not_fully_substituted",
            Self::CascadeSourceUnknown { .. } => "cascade_source_unknown",
            Self::TwoGlobsSameName { .. } => "two_globs_same_name",
            Self::UnknownCascadeBumpStrategy { .. } => "unknown_cascade_bump_strategy",
            Self::InvalidPath { .. } => "invalid_path",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_labels_distinct() {
        let errs = vec![
            ResolverError::PathDoesNotExist {
                unit: "x".into(),
                path: "y".into(),
            },
            ResolverError::AllManifestsAndFallbacksMissing {
                unit: "x".into(),
                tried: vec![],
            },
            ResolverError::EcosystemMismatchVersionField {
                unit: "x".into(),
                ecosystem: "npm".into(),
                version_field: "cargo_toml".into(),
                hint: "?".into(),
            },
            ResolverError::TwoGlobsSamePath {
                path: "x".into(),
                glob_a: "a".into(),
                glob_b: "b".into(),
            },
            ResolverError::NestedBundlePath {
                outer: "o".into(),
                outer_path: "/o".into(),
                inner: "i".into(),
                inner_path: "/o/i".into(),
            },
            ResolverError::BundlePathIsRepoRoot { unit: "x".into() },
            ResolverError::WorkspaceVersionInheritedButRootMissing {
                unit: "x".into(),
                path: "y".into(),
            },
            ResolverError::ManifestCorrupt {
                unit: "x".into(),
                path: "y".into(),
                reason: "?".into(),
            },
            ResolverError::GradlePropertiesNoVersionLine {
                unit: "x".into(),
                path: "y".into(),
            },
            ResolverError::CascadeCycle {
                members: vec!["a".into(), "b".into()],
            },
            ResolverError::NameCollision {
                name: "x".into(),
                paths: vec![],
            },
            ResolverError::SourceBothSet { unit: "x".into() },
            ResolverError::SourceNotSet { unit: "x".into() },
            ResolverError::UnknownEnumValue {
                unit: "x".into(),
                field: "visibility",
                value: "y".into(),
                allowed: "public, internal, hidden",
            },
            ResolverError::GenericRegexMissingPatternOrReplace { unit: "x".into() },
            ResolverError::GenericRegexCaptureCount {
                unit: "x".into(),
                pattern: ".*".into(),
                found: 0,
            },
            ResolverError::UnknownTemplateVar {
                glob_index: 0,
                var: "x".into(),
            },
            ResolverError::TemplateNotFullySubstituted {
                glob_index: 0,
                template: "x".into(),
                result: "x".into(),
            },
            ResolverError::CascadeSourceUnknown {
                unit: "x".into(),
                cascade_source: "y".into(),
            },
            ResolverError::TwoGlobsSameName {
                glob_a: 0,
                glob_b: 1,
                name: "x".into(),
                path: "y".into(),
            },
            ResolverError::UnknownCascadeBumpStrategy {
                unit: "x".into(),
                strategy: "y".into(),
            },
            ResolverError::InvalidPath {
                unit: "x".into(),
                path: "y".into(),
                reason: "z".into(),
            },
        ];

        let labels: Vec<_> = errs.iter().map(|e| e.rule()).collect();
        let mut sorted = labels.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), labels.len(), "rule labels must all be unique");
    }
}
