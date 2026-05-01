use rust_embed::RustEmbed;
use std::str;

use super::config::ConfigurationFile;
use super::errors::{Error, Result};

const DEFAULT_CONFIG_NAME: &str = "default.toml";

#[derive(Debug, RustEmbed)]
#[folder = "config/"]
pub struct EmbeddedConfig;

#[derive(Debug, RustEmbed)]
#[folder = "examples/"]
pub struct EmbeddedPresets;

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
            groups: cfg.groups.into_normalised(),
            bump_sources: cfg.bump_sources,
            release_units: cfg.release_units,
            release_unit_globs: cfg.release_unit_globs,
            ignore_paths: cfg.ignore_paths,
            allow_uncovered: cfg.allow_uncovered,
            ecosystems: cfg.ecosystems,
        })
    }
}

impl EmbeddedPresets {
    pub fn get_preset_string(name: &str) -> Result<String> {
        let filename = if name.ends_with(".toml") {
            name.to_string()
        } else {
            format!("{}.toml", name)
        };

        match Self::get(&filename) {
            Some(file) => {
                let content = str::from_utf8(&file.data)
                    .map_err(|e| Error::new(e).context("preset config contains invalid UTF-8"))?;
                Ok(content.to_string())
            }
            None => Err(Error::msg(format!(
                "preset '{}' not found. Available presets: {}",
                name,
                Self::list_presets().join(", ")
            ))),
        }
    }

    pub fn list_presets() -> Vec<String> {
        Self::iter()
            .filter_map(|name| {
                let name_str = name.as_ref();
                if name_str.ends_with(".toml") {
                    Some(name_str.trim_end_matches(".toml").to_string())
                } else {
                    None
                }
            })
            .collect()
    }
}
