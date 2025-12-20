use anyhow::Context;
use std::{collections::HashMap, fs::File, io::Read, path::Path};

use crate::atry;
use crate::core::release::errors::{Error, Result};

pub mod syntax {
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;

    #[derive(Clone, Debug, Default, Deserialize, Serialize)]
    pub struct UnifiedConfiguration {
        #[serde(skip_serializing_if = "Option::is_none")]
        pub release: Option<ReleaseConfiguration>,
    }

    #[derive(Clone, Debug, Default, Deserialize, Serialize)]
    pub struct ReleaseConfiguration {
        #[serde(default)]
        pub repo: RepoConfiguration,

        #[serde(default)]
        pub changelog: ChangelogConfiguration,

        #[serde(default)]
        pub bump: BumpConfiguration,

        #[serde(default)]
        pub commit_attribution: CommitAttributionConfiguration,

        #[serde(default)]
        pub projects: HashMap<String, ProjectConfiguration>,
    }

    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct BumpConfiguration {
        #[serde(default = "default_true")]
        pub features_always_bump_minor: bool,

        #[serde(default = "default_true")]
        pub breaking_always_bump_major: bool,

        #[serde(default = "default_initial_tag")]
        pub initial_tag: String,
    }

    fn default_initial_tag() -> String {
        "0.1.0".to_string()
    }

    impl Default for BumpConfiguration {
        fn default() -> Self {
            Self {
                features_always_bump_minor: true,
                breaking_always_bump_major: true,
                initial_tag: default_initial_tag(),
            }
        }
    }

    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct ChangelogConfiguration {
        #[serde(default)]
        pub ai_enabled: bool,

        #[serde(default = "default_true")]
        pub conventional_commits: bool,

        #[serde(default = "default_true")]
        pub include_breaking_section: bool,

        #[serde(default = "default_true")]
        pub include_contributors: bool,

        #[serde(default)]
        pub include_statistics: bool,

        #[serde(default = "default_true")]
        pub emoji_groups: bool,

        #[serde(default = "default_changelog_output")]
        pub output: String,

        #[serde(default)]
        pub template: Option<String>,
    }

    fn default_true() -> bool {
        true
    }

    fn default_changelog_output() -> String {
        "CHANGELOG.md".to_string()
    }

    impl Default for ChangelogConfiguration {
        fn default() -> Self {
            Self {
                ai_enabled: false,
                conventional_commits: true,
                include_breaking_section: true,
                include_contributors: true,
                include_statistics: false,
                emoji_groups: true,
                output: default_changelog_output(),
                template: None,
            }
        }
    }

    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct CommitAttributionConfiguration {
        #[serde(default = "default_attribution_strategy")]
        pub strategy: String,

        #[serde(default = "default_scope_matching")]
        pub scope_matching: String,

        #[serde(default)]
        pub scope_mappings: HashMap<String, String>,

        #[serde(default)]
        pub package_scopes: HashMap<String, Vec<String>>,
    }

    fn default_attribution_strategy() -> String {
        "scope_first".to_string()
    }

    fn default_scope_matching() -> String {
        "smart".to_string()
    }

    impl Default for CommitAttributionConfiguration {
        fn default() -> Self {
            Self {
                strategy: default_attribution_strategy(),
                scope_matching: default_scope_matching(),
                scope_mappings: HashMap::new(),
                package_scopes: HashMap::new(),
            }
        }
    }

    #[derive(Clone, Debug, Default, Deserialize, Serialize)]
    pub struct RepoConfiguration {
        #[serde(default)]
        pub upstream_urls: Vec<String>,

        #[serde(default)]
        pub analysis: AnalysisConfig,
    }

    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct AnalysisConfig {
        #[serde(default = "AnalysisConfig::default_commit_cache_size")]
        pub commit_cache_size: usize,

        #[serde(default = "AnalysisConfig::default_tree_cache_size")]
        pub tree_cache_size: usize,
    }

    impl Default for AnalysisConfig {
        fn default() -> Self {
            Self {
                commit_cache_size: Self::default_commit_cache_size(),
                tree_cache_size: Self::default_tree_cache_size(),
            }
        }
    }

    impl AnalysisConfig {
        fn default_commit_cache_size() -> usize {
            512
        }

        fn default_tree_cache_size() -> usize {
            3
        }
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

#[derive(Clone, Debug, Default)]
pub struct ConfigurationFile {
    pub repo: syntax::RepoConfiguration,
    pub changelog: syntax::ChangelogConfiguration,
    pub bump: syntax::BumpConfiguration,
    pub commit_attribution: syntax::CommitAttributionConfiguration,
    pub projects: HashMap<String, syntax::ProjectConfiguration>,
}

impl ConfigurationFile {
    pub fn get<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mut f = match File::open(&path) {
            Ok(f) => f,
            Err(e) => {
                return if e.kind() == std::io::ErrorKind::NotFound {
                    Ok(Self::default())
                } else {
                    Err(Error::new(e).context(format!(
                        "failed to open config file `{}`",
                        path.as_ref().display()
                    )))
                }
            }
        };

        let mut text = String::new();
        f.read_to_string(&mut text)
            .with_context(|| format!("failed to read config file `{}`", path.as_ref().display()))?;

        let unified: syntax::UnifiedConfiguration = toml::from_str(&text).with_context(|| {
            format!(
                "could not parse config file `{}` as TOML",
                path.as_ref().display()
            )
        })?;

        if let Some(release_cfg) = unified.release {
            Ok(ConfigurationFile {
                repo: release_cfg.repo,
                changelog: release_cfg.changelog,
                bump: release_cfg.bump,
                commit_attribution: release_cfg.commit_attribution,
                projects: release_cfg.projects,
            })
        } else {
            Ok(Self::default())
        }
    }

    pub fn into_toml(self) -> Result<String> {
        let unified_cfg = syntax::UnifiedConfiguration {
            release: Some(syntax::ReleaseConfiguration {
                repo: self.repo,
                changelog: self.changelog,
                bump: self.bump,
                commit_attribution: self.commit_attribution,
                projects: self.projects,
            }),
        };
        Ok(atry!(
            toml::to_string_pretty(&unified_cfg);
            ["could not serialize configuration into TOML format"]
        ))
    }
}
