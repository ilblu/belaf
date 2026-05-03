use std::path::Path;

use crate::atry;
use crate::core::errors::{Error, Result};

pub mod syntax {
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;

    use crate::core::release_unit::syntax::{
        AllowUncoveredConfig, EcosystemsConfig, IgnorePathsConfig, ReleaseUnitConfig,
    };

    /// Wire-form for the full `belaf/config.toml`. See the README + docs/configuration.md
    /// for the user-facing documentation; this is the literal serde shape.
    #[derive(Clone, Debug, Deserialize, Serialize)]
    #[serde(deny_unknown_fields)]
    pub struct ReleaseConfiguration {
        pub repo: RepoConfiguration,

        pub changelog: ChangelogConfiguration,

        pub bump: BumpConfiguration,

        pub commit_attribution: CommitAttributionConfiguration,

        /// `[group.<id>]` — bundles projects that release together with
        /// synchronised versions. Named-entry form only; the parser
        /// rejects an array-of-tables `[[group]]` shape.
        ///
        /// ```toml
        /// [group.schema]
        /// members = ["@org/schema", "com.org:schema"]
        /// tag_format = "schema-v{version}"
        /// ```
        #[serde(default, rename = "group", skip_serializing_if = "HashMap::is_empty")]
        pub groups: HashMap<String, GroupConfig>,

        #[serde(default, rename = "bump_source", skip_serializing_if = "Vec::is_empty")]
        pub bump_sources: Vec<BumpSourceConfig>,

        /// `[release_unit.<name>]` — named-entry release units. Each
        /// entry is either explicit (no `glob` field) or glob-form
        /// (with `glob` set, expanding at resolve-time into N units
        /// per matching directory). Named-entry only; the parser
        /// rejects array-of-tables `[[release_unit]]` and the separate
        /// `[[release_unit_glob]]` top-level key.
        #[serde(
            default,
            rename = "release_unit",
            skip_serializing_if = "HashMap::is_empty"
        )]
        pub release_units: HashMap<String, ReleaseUnitConfig>,

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

    /// `[group.<id>]` named-entry — the TOML key is the group id.
    #[derive(Clone, Debug, Deserialize, Serialize)]
    #[serde(deny_unknown_fields)]
    pub struct GroupConfig {
        pub members: Vec<String>,

        /// Group-level tag-format override. Wins over the ecosystem
        /// default but loses to per-project overrides. Useful when every
        /// member of a synchronised group should ship under one tag
        /// (e.g. `schema-v{version}` for a multi-ecosystem schema bundle).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub tag_format: Option<String>,
    }

    /// Runtime-adjacent shape: same fields as [`GroupConfig`] plus the
    /// id (lifted out of the TOML key). The rest of the codebase works
    /// in terms of this; the TOML form only exists at deserialize.
    #[derive(Clone, Debug)]
    pub struct ResolvedGroupConfig {
        pub id: String,
        pub members: Vec<String>,
        pub tag_format: Option<String>,
    }

    /// `[[bump_source]]` table: a subprocess belaf runs by default to
    /// gather externally-computed bump decisions (e.g. `graphql-inspector
    /// diff`). `cmd` is required; `release_unit` / `group` are pure
    /// diagnostic labels (the JSON output's own `release_unit` field is
    /// what wires decisions to ReleaseUnits).
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct BumpSourceConfig {
        pub cmd: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub release_unit: Option<String>,
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
}

/// Runtime-adjacent shape: a single named release unit with the name
/// lifted out of the HashMap key. Resolver consumes this.
#[derive(Clone, Debug)]
pub struct NamedReleaseUnitConfig {
    pub name: String,
    pub config: crate::core::release_unit::syntax::ReleaseUnitConfig,
}

#[derive(Clone, Debug)]
pub struct ConfigurationFile {
    pub repo: syntax::RepoConfiguration,
    pub changelog: syntax::ChangelogConfiguration,
    pub bump: syntax::BumpConfiguration,
    pub commit_attribution: syntax::CommitAttributionConfiguration,
    pub groups: Vec<syntax::ResolvedGroupConfig>,
    pub bump_sources: Vec<syntax::BumpSourceConfig>,
    pub release_units: Vec<NamedReleaseUnitConfig>,
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

        // Promote the HashMap keys into runtime-adjacent shapes with a
        // stable iteration order. Sort by name for deterministic
        // resolution + downstream tests.
        let mut groups: Vec<syntax::ResolvedGroupConfig> = cfg
            .groups
            .into_iter()
            .map(|(id, g)| syntax::ResolvedGroupConfig {
                id,
                members: g.members,
                tag_format: g.tag_format,
            })
            .collect();
        groups.sort_by(|a, b| a.id.cmp(&b.id));

        let mut release_units: Vec<NamedReleaseUnitConfig> = cfg
            .release_units
            .into_iter()
            .map(|(name, config)| NamedReleaseUnitConfig { name, config })
            .collect();
        release_units.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(ConfigurationFile {
            repo: cfg.repo,
            changelog: cfg.changelog,
            bump: cfg.bump,
            commit_attribution: cfg.commit_attribution,
            groups,
            bump_sources: cfg.bump_sources,
            release_units,
            ignore_paths: cfg.ignore_paths,
            allow_uncovered: cfg.allow_uncovered,
            ecosystems: cfg.ecosystems,
        })
    }

    pub fn into_toml(self) -> Result<String> {
        use std::collections::HashMap;
        let groups: HashMap<String, syntax::GroupConfig> = self
            .groups
            .into_iter()
            .map(|g| {
                (
                    g.id,
                    syntax::GroupConfig {
                        members: g.members,
                        tag_format: g.tag_format,
                    },
                )
            })
            .collect();
        let release_units: HashMap<String, crate::core::release_unit::syntax::ReleaseUnitConfig> =
            self.release_units
                .into_iter()
                .map(|u| (u.name, u.config))
                .collect();
        let cfg = syntax::ReleaseConfiguration {
            repo: self.repo,
            changelog: self.changelog,
            bump: self.bump,
            commit_attribution: self.commit_attribution,
            groups,
            bump_sources: self.bump_sources,
            release_units,
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
