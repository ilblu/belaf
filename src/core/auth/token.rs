use crate::error::{CliError, Result};
use keyring::Entry;

const SERVICE_NAME: &str = "belaf";
const TOKEN_KEY: &str = "github-token";

pub fn save_token(token: &str) -> Result<()> {
    let entry = Entry::new(SERVICE_NAME, TOKEN_KEY)
        .map_err(|e| CliError::TokenStorage(format!("Failed to create keyring entry: {}", e)))?;

    entry
        .set_password(token)
        .map_err(|e| CliError::TokenStorage(format!("Failed to save token: {}", e)))?;

    Ok(())
}

pub fn load_token() -> Result<String> {
    let entry = Entry::new(SERVICE_NAME, TOKEN_KEY)
        .map_err(|e| CliError::TokenStorage(format!("Failed to create keyring entry: {}", e)))?;

    entry.get_password().map_err(|e| match e {
        keyring::Error::NoEntry => CliError::AuthenticationRequired,
        _ => CliError::TokenStorage(format!("Failed to load token: {}", e)),
    })
}

pub fn delete_token() -> Result<()> {
    let entry = Entry::new(SERVICE_NAME, TOKEN_KEY)
        .map_err(|e| CliError::TokenStorage(format!("Failed to create keyring entry: {}", e)))?;

    match entry.delete_credential() {
        Ok(_) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(CliError::TokenStorage(format!(
            "Failed to delete token: {}",
            e
        ))),
    }
}
