use std::path::PathBuf;

use glob::Pattern;
use regex::Regex;
use serde::{Deserialize, Serialize};

use super::command;
use super::error::Result;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct GitConfig {
    #[serde(default = "default_true")]
    pub conventional_commits: bool,
    #[serde(default)]
    pub require_conventional: bool,
    #[serde(default)]
    pub filter_unconventional: bool,
    #[serde(default)]
    pub split_commits: bool,
    #[serde(default)]
    pub commit_preprocessors: Vec<TextProcessor>,
    #[serde(default = "default_commit_parsers")]
    pub commit_parsers: Vec<CommitParser>,
    #[serde(default = "default_true")]
    pub protect_breaking_commits: bool,
    #[serde(default)]
    pub link_parsers: Vec<LinkParser>,
    #[serde(default)]
    pub filter_commits: bool,
    #[serde(default)]
    pub fail_on_unmatched_commit: bool,
    #[serde(with = "serde_regex", default)]
    pub tag_pattern: Option<Regex>,
    #[serde(with = "serde_regex", default)]
    pub skip_tags: Option<Regex>,
    #[serde(with = "serde_regex", default)]
    pub ignore_tags: Option<Regex>,
    #[serde(with = "serde_regex", default)]
    pub count_tags: Option<Regex>,
    #[serde(default)]
    pub use_branch_tags: bool,
    #[serde(default)]
    pub topo_order: bool,
    #[serde(default = "default_true")]
    pub topo_order_commits: bool,
    #[serde(default = "default_sort_commits")]
    pub sort_commits: String,
    #[serde(default)]
    pub limit_commits: Option<usize>,
    #[serde(default)]
    pub recurse_submodules: Option<bool>,
    #[serde(with = "serde_pattern", default)]
    pub include_paths: Vec<Pattern>,
    #[serde(with = "serde_pattern", default)]
    pub exclude_paths: Vec<Pattern>,
}

mod serde_pattern {
    use glob::Pattern;
    use serde::de::Error;
    use serde::ser::SerializeSeq;
    use serde::Deserialize;

    pub fn serialize<S>(patterns: &[Pattern], serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(patterns.len()))?;
        for pattern in patterns {
            seq.serialize_element(pattern.as_str())?;
        }
        seq.end()
    }

    pub fn deserialize<'de, D>(deserializer: D) -> std::result::Result<Vec<Pattern>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let patterns = Vec::<String>::deserialize(deserializer)?;
        patterns
            .into_iter()
            .map(|pattern| pattern.parse().map_err(D::Error::custom))
            .collect()
    }
}

fn default_true() -> bool {
    true
}

fn default_sort_commits() -> String {
    "oldest".to_string()
}

fn default_commit_parsers() -> Vec<CommitParser> {
    vec![
        CommitParser {
            message: Some(Regex::new("^feat").expect("valid regex")),
            group: Some("✨ Features".to_string()),
            ..Default::default()
        },
        CommitParser {
            message: Some(Regex::new("^fix").expect("valid regex")),
            group: Some("🐛 Bug Fixes".to_string()),
            ..Default::default()
        },
        CommitParser {
            message: Some(Regex::new("^doc").expect("valid regex")),
            group: Some("📚 Documentation".to_string()),
            ..Default::default()
        },
        CommitParser {
            message: Some(Regex::new("^perf").expect("valid regex")),
            group: Some("⚡ Performance".to_string()),
            ..Default::default()
        },
        CommitParser {
            message: Some(Regex::new("^refactor").expect("valid regex")),
            group: Some("♻️ Refactoring".to_string()),
            ..Default::default()
        },
        CommitParser {
            message: Some(Regex::new("^style").expect("valid regex")),
            group: Some("🎨 Styling".to_string()),
            ..Default::default()
        },
        CommitParser {
            message: Some(Regex::new("^test").expect("valid regex")),
            group: Some("🧪 Testing".to_string()),
            ..Default::default()
        },
        CommitParser {
            message: Some(Regex::new(r"^chore\(deps\)").expect("valid regex")),
            skip: Some(true),
            ..Default::default()
        },
        CommitParser {
            message: Some(Regex::new(r"^chore\(release\)").expect("valid regex")),
            skip: Some(true),
            ..Default::default()
        },
        CommitParser {
            message: Some(Regex::new("^chore|^ci").expect("valid regex")),
            group: Some("🔧 Miscellaneous".to_string()),
            ..Default::default()
        },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextProcessor {
    #[serde(with = "serde_regex")]
    pub pattern: Regex,
    pub replace: Option<String>,
    pub replace_command: Option<String>,
}

impl TextProcessor {
    pub fn replace(&self, text: &mut String, envs: Vec<(&str, &str)>) -> Result<()> {
        if let Some(replacement) = &self.replace {
            *text = self.pattern.replace_all(text, replacement).to_string();
        } else if let Some(cmd) = &self.replace_command {
            if self.pattern.is_match(text) {
                *text = command::run(cmd, Some(text.to_string()), envs)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct CommitParser {
    pub sha: Option<String>,
    #[serde(with = "serde_regex", default)]
    pub message: Option<Regex>,
    #[serde(with = "serde_regex", default)]
    pub body: Option<Regex>,
    #[serde(with = "serde_regex", default)]
    pub footer: Option<Regex>,
    pub group: Option<String>,
    pub default_scope: Option<String>,
    pub scope: Option<String>,
    pub skip: Option<bool>,
    pub field: Option<String>,
    #[serde(with = "serde_regex", default)]
    pub pattern: Option<Regex>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkParser {
    #[serde(with = "serde_regex")]
    pub pattern: Regex,
    pub href: String,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangelogConfig {
    pub header: Option<String>,
    pub body: String,
    pub footer: Option<String>,
    #[serde(default)]
    pub trim: bool,
    #[serde(default)]
    pub render_always: bool,
    #[serde(default)]
    pub postprocessors: Vec<TextProcessor>,
    pub output: Option<PathBuf>,
}

impl Default for ChangelogConfig {
    fn default() -> Self {
        Self {
            header: Some("# Changelog\n".to_string()),
            body: super::template::DEFAULT_CHANGELOG_TEMPLATE.to_string(),
            footer: None,
            trim: true,
            render_always: false,
            postprocessors: Vec::new(),
            output: None,
        }
    }
}

impl ChangelogConfig {
    pub fn from_user_config(
        user_cfg: &crate::core::release::config::syntax::ChangelogConfiguration,
    ) -> Self {
        let mut config = Self::default();

        if let Some(ref template) = user_cfg.template {
            config.body = template.clone();
        }

        config.output = Some(PathBuf::from(&user_cfg.output));
        config
    }
}

impl GitConfig {
    pub fn from_user_config(
        user_cfg: &crate::core::release::config::syntax::ChangelogConfiguration,
    ) -> Self {
        let commit_parsers = if user_cfg.emoji_groups {
            default_commit_parsers()
        } else {
            default_commit_parsers_no_emoji()
        };

        Self {
            conventional_commits: user_cfg.conventional_commits,
            commit_parsers,
            ..Default::default()
        }
    }
}

fn default_commit_parsers_no_emoji() -> Vec<CommitParser> {
    vec![
        CommitParser {
            message: Some(Regex::new("^feat").expect("valid regex")),
            group: Some("Features".to_string()),
            ..Default::default()
        },
        CommitParser {
            message: Some(Regex::new("^fix").expect("valid regex")),
            group: Some("Bug Fixes".to_string()),
            ..Default::default()
        },
        CommitParser {
            message: Some(Regex::new("^doc").expect("valid regex")),
            group: Some("Documentation".to_string()),
            ..Default::default()
        },
        CommitParser {
            message: Some(Regex::new("^perf").expect("valid regex")),
            group: Some("Performance".to_string()),
            ..Default::default()
        },
        CommitParser {
            message: Some(Regex::new("^refactor").expect("valid regex")),
            group: Some("Refactoring".to_string()),
            ..Default::default()
        },
        CommitParser {
            message: Some(Regex::new("^style").expect("valid regex")),
            group: Some("Styling".to_string()),
            ..Default::default()
        },
        CommitParser {
            message: Some(Regex::new("^test").expect("valid regex")),
            group: Some("Testing".to_string()),
            ..Default::default()
        },
        CommitParser {
            message: Some(Regex::new(r"^chore\(deps\)").expect("valid regex")),
            skip: Some(true),
            ..Default::default()
        },
        CommitParser {
            message: Some(Regex::new(r"^chore\(release\)").expect("valid regex")),
            skip: Some(true),
            ..Default::default()
        },
        CommitParser {
            message: Some(Regex::new("^chore|^ci").expect("valid regex")),
            group: Some("Miscellaneous".to_string()),
            ..Default::default()
        },
    ]
}
