use std::{collections::HashMap, path::Path};

use crate::atry;
use crate::core::errors::{Error, Result};

pub mod syntax {
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;

    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct ReleaseConfiguration {
        pub repo: RepoConfiguration,

        pub changelog: ChangelogConfiguration,

        pub bump: BumpConfiguration,

        pub commit_attribution: CommitAttributionConfiguration,

        #[serde(default)]
        pub projects: HashMap<String, ProjectConfiguration>,
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
        })
    }

    pub fn into_toml(self) -> Result<String> {
        let cfg = syntax::ReleaseConfiguration {
            repo: self.repo,
            changelog: self.changelog,
            bump: self.bump,
            commit_attribution: self.commit_attribution,
            projects: self.projects,
        };
        Ok(atry!(
            toml::to_string_pretty(&cfg);
            ["could not serialize configuration into TOML format"]
        ))
    }
}
