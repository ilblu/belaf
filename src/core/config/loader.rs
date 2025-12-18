use super::types::Config;
use crate::error::{CliError, Result};
use config::{Config as ConfigBuilder, Environment, File};

pub fn load() -> Result<Config> {
    let mut builder = ConfigBuilder::builder();

    if let Some(config_dir) = dirs::config_dir() {
        let belaf_config = config_dir.join("belaf");
        builder = builder.add_source(File::from(belaf_config.join("config.toml")).required(false));
    }

    let config: Config = builder
        .add_source(
            Environment::with_prefix("BELAF")
                .separator("_")
                .try_parsing(true),
        )
        .build()
        .map_err(CliError::Config)?
        .try_deserialize()
        .unwrap_or_default();

    Ok(config)
}
