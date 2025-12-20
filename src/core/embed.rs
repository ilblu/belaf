use rust_embed::RustEmbed;
use std::str;

use super::config::ConfigurationFile;
use super::errors::{Error, Result};

const DEFAULT_CONFIG_NAME: &str = "default.toml";

#[derive(Debug, RustEmbed)]
#[folder = "config/"]
pub struct EmbeddedConfig;

impl EmbeddedConfig {
    pub fn get_config_string() -> Result<String> {
        match Self::get(DEFAULT_CONFIG_NAME) {
            Some(file) => {
                let content = str::from_utf8(&file.data)
                    .map_err(|e| Error::new(e).context("embedded config contains invalid UTF-8"))?;
                Ok(content.to_string())
            }
            None => Err(Error::msg("embedded default config not found")),
        }
    }

    pub fn parse() -> Result<ConfigurationFile> {
        let content = Self::get_config_string()?;
        let cfg: super::config::syntax::ReleaseConfiguration = toml::from_str(&content)
            .map_err(|e| Error::new(e).context("failed to parse embedded config as TOML"))?;

        Ok(ConfigurationFile {
            repo: cfg.repo,
            changelog: cfg.changelog,
            bump: cfg.bump,
            commit_attribution: cfg.commit_attribution,
            projects: cfg.projects,
        })
    }
}
