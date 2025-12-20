use std::path::PathBuf;

use glob::Pattern;
use regex::Regex;
use serde::{Deserialize, Serialize};

use super::command;
use super::error::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitConfig {
    pub conventional_commits: bool,
    #[serde(default)]
    pub require_conventional: bool,
    pub filter_unconventional: bool,
    #[serde(default)]
    pub split_commits: bool,
    #[serde(default)]
    pub commit_preprocessors: Vec<TextProcessor>,
    pub commit_parsers: Vec<CommitParser>,
    pub protect_breaking_commits: bool,
    #[serde(default)]
    pub link_parsers: Vec<LinkParser>,
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
    pub topo_order_commits: bool,
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
    pub trim: bool,
    #[serde(default)]
    pub render_always: bool,
    #[serde(default)]
    pub postprocessors: Vec<TextProcessor>,
    pub output: Option<PathBuf>,
    pub include_breaking_section: bool,
    pub include_contributors: bool,
    pub include_statistics: bool,
    pub emoji_groups: bool,
    #[serde(default)]
    pub group_emojis: std::collections::HashMap<String, String>,
}

impl CommitParser {
    pub fn from_config(cfg: &crate::core::config::syntax::CommitParserConfig) -> Option<Self> {
        Some(Self {
            sha: None,
            message: cfg.message.as_ref().and_then(|p| Regex::new(p).ok()),
            body: cfg.body.as_ref().and_then(|p| Regex::new(p).ok()),
            footer: cfg.footer.as_ref().and_then(|p| Regex::new(p).ok()),
            group: cfg.group.clone(),
            default_scope: cfg.default_scope.clone(),
            scope: cfg.scope.clone(),
            skip: cfg.skip,
            field: None,
            pattern: None,
        })
    }
}

impl LinkParser {
    pub fn from_config(cfg: &crate::core::config::syntax::LinkParserConfig) -> Option<Self> {
        let pattern = Regex::new(&cfg.pattern).ok()?;
        Some(Self {
            pattern,
            href: cfg.href.clone(),
            text: cfg.text.clone(),
        })
    }
}

impl TextProcessor {
    pub fn from_config(cfg: &crate::core::config::syntax::TextProcessorConfig) -> Self {
        Self {
            pattern: Regex::new(&cfg.pattern)
                .unwrap_or_else(|_| Regex::new("$^").expect("valid regex")),
            replace: cfg.replace.clone(),
            replace_command: None,
        }
    }
}

impl ChangelogConfig {
    pub fn from_user_config(
        user_cfg: &crate::core::config::syntax::ChangelogConfiguration,
    ) -> Self {
        let postprocessors = user_cfg
            .postprocessors
            .iter()
            .map(TextProcessor::from_config)
            .collect();

        Self {
            header: user_cfg.header.clone(),
            body: user_cfg.body.clone(),
            footer: user_cfg.footer.clone(),
            trim: user_cfg.trim,
            render_always: false,
            postprocessors,
            output: Some(PathBuf::from(&user_cfg.output)),
            include_breaking_section: user_cfg.include_breaking_section,
            include_contributors: user_cfg.include_contributors,
            include_statistics: user_cfg.include_statistics,
            emoji_groups: user_cfg.emoji_groups,
            group_emojis: user_cfg.group_emojis.clone(),
        }
    }

    pub fn get_emoji(&self, group: &str) -> Option<&str> {
        self.group_emojis
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(group))
            .map(|(_, v)| v.as_str())
    }
}

impl GitConfig {
    pub fn from_user_config(
        user_cfg: &crate::core::config::syntax::ChangelogConfiguration,
    ) -> Self {
        let commit_parsers = user_cfg
            .commit_parsers
            .iter()
            .filter_map(CommitParser::from_config)
            .collect();

        let link_parsers = user_cfg
            .link_parsers
            .iter()
            .filter_map(LinkParser::from_config)
            .collect();

        let commit_preprocessors = user_cfg
            .commit_preprocessors
            .iter()
            .map(TextProcessor::from_config)
            .collect();

        Self {
            conventional_commits: user_cfg.conventional_commits,
            protect_breaking_commits: user_cfg.protect_breaking_commits,
            filter_unconventional: user_cfg.filter_unconventional,
            filter_commits: user_cfg.filter_commits,
            sort_commits: user_cfg.sort_commits.clone(),
            limit_commits: user_cfg.limit_commits,
            tag_pattern: user_cfg
                .tag_pattern
                .as_ref()
                .and_then(|p| Regex::new(p).ok()),
            skip_tags: user_cfg.skip_tags.as_ref().and_then(|p| Regex::new(p).ok()),
            ignore_tags: user_cfg
                .ignore_tags
                .as_ref()
                .and_then(|p| Regex::new(p).ok()),
            commit_parsers,
            link_parsers,
            commit_preprocessors,
            require_conventional: false,
            split_commits: false,
            fail_on_unmatched_commit: false,
            count_tags: None,
            use_branch_tags: false,
            topo_order: false,
            topo_order_commits: true,
            recurse_submodules: None,
            include_paths: Vec::new(),
            exclude_paths: Vec::new(),
        }
    }
}
