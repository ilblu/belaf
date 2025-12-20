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
                let content = str::from_utf8(&file.data).map_err(|e| {
                    Error::new(e).context("embedded config contains invalid UTF-8")
                })?;
                Ok(content.to_string())
            }
            None => Err(Error::msg("embedded default config not found")),
        }
    }

    pub fn parse() -> Result<ConfigurationFile> {
        let content = Self::get_config_string()?;
        let unified: super::config::syntax::UnifiedConfiguration =
            toml::from_str(&content).map_err(|e| {
                Error::new(e).context("failed to parse embedded config as TOML")
            })?;

        let release_cfg = unified
            .release
            .ok_or_else(|| Error::msg("embedded config missing [release] section"))?;

        Ok(ConfigurationFile {
            repo: release_cfg.repo,
            changelog: release_cfg.changelog,
            bump: release_cfg.bump,
            commit_attribution: release_cfg.commit_attribution,
            projects: release_cfg.projects,
        })
    }
}
