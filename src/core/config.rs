use std::{collections::HashMap, path::Path};

use crate::atry;
use crate::core::errors::{Error, Result};

pub mod syntax {
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;

    use crate::core::release_unit::syntax::{
        AllowUncoveredConfig, EcosystemsConfig, ExplicitReleaseUnitConfig,
        GlobReleaseUnitConfig, IgnorePathsConfig,
    };

    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct ReleaseConfiguration {
        pub repo: RepoConfiguration,

        pub changelog: ChangelogConfiguration,

        pub bump: BumpConfiguration,

        pub commit_attribution: CommitAttributionConfiguration,

        #[serde(default)]
        pub projects: HashMap<String, ProjectConfiguration>,

        /// Two TOML surfaces, one Rust shape. Either:
        ///
        /// ```toml
        /// [[group]]
        /// id = "schema"
        /// members = ["@org/schema", "com.org:schema"]
        /// tag_format = "schema-v{version}"
        /// ```
        ///
        /// or named-entry form (plan §8):
        ///
        /// ```toml
        /// [group.schema]
        /// members = ["@org/schema", "com.org:schema"]
        /// tag_format = "schema-v{version}"
        /// ```
        ///
        /// Both forms produce the same `Vec<GroupConfig>` after
        /// normalisation in `ConfigurationFile::get`.
        #[serde(
            default,
            rename = "group",
            skip_serializing_if = "GroupsForm::is_empty"
        )]
        pub groups: GroupsForm,

        #[serde(default, rename = "bump_source", skip_serializing_if = "Vec::is_empty")]
        pub bump_sources: Vec<BumpSourceConfig>,

        /// `[[release_unit]]` — explicit ReleaseUnit entries. Plan
        /// Part I + II.
        #[serde(default, rename = "release_unit", skip_serializing_if = "Vec::is_empty")]
        pub release_units: Vec<ExplicitReleaseUnitConfig>,

        /// `[[release_unit_glob]]` — glob-form ReleaseUnit entries
        /// expanded at resolve-time into N units, one per matching dir.
        /// Plan §2.3.
        #[serde(
            default,
            rename = "release_unit_glob",
            skip_serializing_if = "Vec::is_empty"
        )]
        pub release_unit_globs: Vec<GlobReleaseUnitConfig>,

        /// `[ignore_paths]` — paths belaf does not scan inside.
        #[serde(default, skip_serializing_if = "IgnorePathsConfig::is_empty")]
        pub ignore_paths: IgnorePathsConfig,

        /// `[allow_uncovered]` — paths belaf scans but explicitly
        /// accepts as not mapping to any ReleaseUnit. Mobile apps go
        /// here on init.
        #[serde(default, skip_serializing_if = "AllowUncoveredConfig::is_empty")]
        pub allow_uncovered: AllowUncoveredConfig,

        /// `[ecosystems.*]` — per-ecosystem smart-default knobs.
        #[serde(default, skip_serializing_if = "EcosystemsConfig::is_empty")]
        pub ecosystems: EcosystemsConfig,
    }

    /// `[[group]]` table: bundles projects that must release together.
    /// `id` is the wire-format group id (lowercased pattern); `members`
    /// are user-facing project names (resolved to `ProjectId`s after the
    /// graph is built).
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct GroupConfig {
        pub id: String,
        pub members: Vec<String>,

        /// Group-level tag-format override. Wins over the ecosystem
        /// default but loses to per-project overrides. Useful when every
        /// member of a synchronised group should ship under one tag
        /// (e.g. `schema-v{version}` for a multi-ecosystem schema bundle).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub tag_format: Option<String>,
    }

    /// Named-entry form of `GroupConfig` — same fields without `id`,
    /// because the TOML key `[group.<id>]` carries it.
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct GroupNamedConfig {
        pub members: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub tag_format: Option<String>,
    }

    /// Either array-of-tables (`[[group]]`) or named-entry
    /// (`[group.<id>]`). Both deserialize through this `untagged` enum
    /// and are normalised to a single `Vec<GroupConfig>` by the
    /// `into_normalised` helper before the rest of the codebase sees
    /// them. Plan §8.
    #[derive(Clone, Debug, Deserialize, Serialize)]
    #[serde(untagged)]
    pub enum GroupsForm {
        Array(Vec<GroupConfig>),
        Named(HashMap<String, GroupNamedConfig>),
    }

    impl Default for GroupsForm {
        fn default() -> Self {
            Self::Array(Vec::new())
        }
    }

    impl GroupsForm {
        pub fn is_empty(&self) -> bool {
            match self {
                Self::Array(v) => v.is_empty(),
                Self::Named(m) => m.is_empty(),
            }
        }

        /// Collapse both forms into a single `Vec<GroupConfig>`. Named
        /// entries get their key promoted to `GroupConfig::id`.
        pub fn into_normalised(self) -> Vec<GroupConfig> {
            match self {
                Self::Array(v) => v,
                Self::Named(m) => m
                    .into_iter()
                    .map(|(id, named)| GroupConfig {
                        id,
                        members: named.members,
                        tag_format: named.tag_format,
                    })
                    .collect(),
            }
        }
    }

    /// `[[bump_source]]` table: a subprocess belaf runs by default to
    /// gather externally-computed bump decisions (e.g. `graphql-inspector
    /// diff`). At least one of `cmd` is required; `project` / `group` are
    /// pure diagnostic labels (the JSON output's own `project` field is
    /// what wires decisions to projects).
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct BumpSourceConfig {
        pub cmd: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub project: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub group: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub timeout_sec: Option<u64>,
    }

    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct BumpConfiguration {
        pub features_always_bump_minor: bool,

        pub breaking_always_bump_major: bool,

        pub initial_tag: String,

        #[serde(default)]
        pub bump_type: Option<String>,
    }

    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct ChangelogConfiguration {
        #[serde(default)]
        pub header: Option<String>,

        pub body: String,

        #[serde(default)]
        pub footer: Option<String>,

        pub trim: bool,

        pub output: String,

        pub conventional_commits: bool,

        pub protect_breaking_commits: bool,

        pub filter_unconventional: bool,

        pub filter_commits: bool,

        pub sort_commits: String,

        #[serde(default)]
        pub limit_commits: Option<usize>,

        #[serde(default)]
        pub tag_pattern: Option<String>,

        #[serde(default)]
        pub skip_tags: Option<String>,

        #[serde(default)]
        pub ignore_tags: Option<String>,

        #[serde(default)]
        pub commit_parsers: Vec<CommitParserConfig>,

        #[serde(default)]
        pub link_parsers: Vec<LinkParserConfig>,

        #[serde(default)]
        pub commit_preprocessors: Vec<TextProcessorConfig>,

        #[serde(default)]
        pub postprocessors: Vec<TextProcessorConfig>,

        pub include_breaking_section: bool,

        pub include_contributors: bool,

        pub include_statistics: bool,

        pub emoji_groups: bool,

        #[serde(default)]
        pub group_emojis: std::collections::HashMap<String, String>,
    }

    #[derive(Clone, Debug, Default, Deserialize, Serialize)]
    pub struct CommitParserConfig {
        #[serde(default)]
        pub message: Option<String>,

        #[serde(default)]
        pub body: Option<String>,

        #[serde(default)]
        pub footer: Option<String>,

        #[serde(default)]
        pub group: Option<String>,

        #[serde(default)]
        pub scope: Option<String>,

        #[serde(default)]
        pub default_scope: Option<String>,

        #[serde(default)]
        pub skip: Option<bool>,
    }

    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct LinkParserConfig {
        pub pattern: String,

        pub href: String,

        #[serde(default)]
        pub text: Option<String>,
    }

    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct TextProcessorConfig {
        pub pattern: String,

        #[serde(default)]
        pub replace: Option<String>,
    }

    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct CommitAttributionConfiguration {
        pub strategy: String,

        pub scope_matching: String,

        #[serde(default)]
        pub scope_mappings: HashMap<String, String>,

        #[serde(default)]
        pub package_scopes: HashMap<String, Vec<String>>,
    }

    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct RepoConfiguration {
        #[serde(default)]
        pub upstream_urls: Vec<String>,

        pub analysis: AnalysisConfig,
    }

    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct AnalysisConfig {
        pub commit_cache_size: usize,

        pub tree_cache_size: usize,
    }

    #[derive(Clone, Debug, Default, Deserialize, Serialize)]
    pub struct ProjectConfiguration {
        #[serde(default)]
        pub ignore: bool,

        /// Per-project tag-format override. Wins over the group default
        /// and the ecosystem default. Variables: `{name}`, `{version}`,
        /// `{ecosystem}` everywhere, plus `{groupId}` / `{artifactId}`
        /// for Maven and `{module}` for Go. An unsupported variable for
        /// the project's ecosystem is a hard error at release time.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub tag_format: Option<String>,

        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub npm: Option<NpmProjectConfig>,

        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub cargo: Option<CargoProjectConfig>,
    }

    #[derive(Clone, Debug, Default, Deserialize, Serialize)]
    pub struct NpmProjectConfig {
        #[serde(skip_serializing_if = "Option::is_none")]
        pub internal_dep_protocol: Option<String>,

        #[serde(default)]
        pub strict_dependency_validation: bool,
    }

    #[derive(Clone, Debug, Default, Deserialize, Serialize)]
    pub struct CargoProjectConfig {
        #[serde(default)]
        pub publish: bool,
    }
}

#[derive(Clone, Debug)]
pub struct ConfigurationFile {
    pub repo: syntax::RepoConfiguration,
    pub changelog: syntax::ChangelogConfiguration,
    pub bump: syntax::BumpConfiguration,
    pub commit_attribution: syntax::CommitAttributionConfiguration,
    pub projects: HashMap<String, syntax::ProjectConfiguration>,
    pub groups: Vec<syntax::GroupConfig>,
    pub bump_sources: Vec<syntax::BumpSourceConfig>,
    pub release_units: Vec<crate::core::release_unit::syntax::ExplicitReleaseUnitConfig>,
    pub release_unit_globs: Vec<crate::core::release_unit::syntax::GlobReleaseUnitConfig>,
    pub ignore_paths: crate::core::release_unit::syntax::IgnorePathsConfig,
    pub allow_uncovered: crate::core::release_unit::syntax::AllowUncoveredConfig,
    pub ecosystems: crate::core::release_unit::syntax::EcosystemsConfig,
}

impl ConfigurationFile {
    pub fn get<P: AsRef<Path>>(path: P) -> Result<Self> {
        let embedded_config_str = super::embed::EmbeddedConfig::get_config_string()?;

        let mut builder = config::Config::builder().add_source(config::File::from_str(
            &embedded_config_str,
            config::FileFormat::Toml,
        ));

        if path.as_ref().exists() {
            builder = builder.add_source(config::File::from(path.as_ref()));
        }

        let cfg: syntax::ReleaseConfiguration = builder
            .build()
            .map_err(|e| Error::new(e).context("failed to build configuration"))?
            .try_deserialize()
            .map_err(|e| Error::new(e).context("failed to deserialize configuration"))?;

        Ok(ConfigurationFile {
            repo: cfg.repo,
            changelog: cfg.changelog,
            bump: cfg.bump,
            commit_attribution: cfg.commit_attribution,
            projects: cfg.projects,
            groups: cfg.groups.into_normalised(),
            bump_sources: cfg.bump_sources,
            release_units: cfg.release_units,
            release_unit_globs: cfg.release_unit_globs,
            ignore_paths: cfg.ignore_paths,
            allow_uncovered: cfg.allow_uncovered,
            ecosystems: cfg.ecosystems,
        })
    }

    pub fn into_toml(self) -> Result<String> {
        let cfg = syntax::ReleaseConfiguration {
            repo: self.repo,
            changelog: self.changelog,
            bump: self.bump,
            commit_attribution: self.commit_attribution,
            projects: self.projects,
            // Always serialise back as the array-of-tables form so the
            // canonical written-out config matches `belaf init`'s output.
            groups: syntax::GroupsForm::Array(self.groups),
            bump_sources: self.bump_sources,
            release_units: self.release_units,
            release_unit_globs: self.release_unit_globs,
            ignore_paths: self.ignore_paths,
            allow_uncovered: self.allow_uncovered,
            ecosystems: self.ecosystems,
        };
        Ok(atry!(
            toml::to_string_pretty(&cfg);
            ["could not serialize configuration into TOML format"]
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::syntax::GroupsForm;
    use serde::Deserialize;

    /// Tight harness around just the `[group]` surface — avoids dragging
    /// the full `ReleaseConfiguration` schema into the test, which has
    /// dozens of unrelated required fields.
    #[derive(Debug, Deserialize)]
    struct GroupsOnly {
        #[serde(default, rename = "group")]
        groups: GroupsForm,
    }

    /// Both TOML surfaces — array-of-tables `[[group]]` and
    /// named-entry `[group.<id>]` — must produce identical
    /// `Vec<GroupConfig>` after `into_normalised()`. Plan §8.
    #[test]
    fn group_config_array_and_named_forms_roundtrip_to_same_shape() {
        let array_form = r#"
[[group]]
id = "schema"
members = ["@org/schema", "com.org:schema"]
tag_format = "schema-v{version}"
"#;

        let named_form = r#"
[group.schema]
members = ["@org/schema", "com.org:schema"]
tag_format = "schema-v{version}"
"#;

        let from_array: GroupsOnly =
            toml::from_str(array_form).expect("array-of-tables form should parse");
        let from_named: GroupsOnly =
            toml::from_str(named_form).expect("named-entry form should parse");

        let arr = from_array.groups.into_normalised();
        let nam = from_named.groups.into_normalised();

        assert_eq!(arr.len(), 1);
        assert_eq!(nam.len(), 1);
        assert_eq!(arr[0].id, "schema");
        assert_eq!(nam[0].id, "schema");
        assert_eq!(arr[0].members, nam[0].members);
        assert_eq!(arr[0].tag_format, nam[0].tag_format);
        assert_eq!(arr[0].tag_format.as_deref(), Some("schema-v{version}"));
    }

    #[test]
    fn empty_groups_roundtrip_through_either_default() {
        // Default `GroupsForm::Array(vec![])` is the only possible
        // empty state — the parser sees no `[group]` key at all.
        let g = GroupsForm::default();
        assert!(g.is_empty());
        assert_eq!(g.into_normalised().len(), 0);
    }
}
