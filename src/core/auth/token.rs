use crate::core::api::StoredToken;
use crate::error::{CliError, Result};
use keyring::Entry;

const SERVICE_NAME: &str = "belaf";
const TOKEN_KEY: &str = "api-token";

fn is_keyring_disabled() -> bool {
    std::env::var("BELAF_NO_KEYRING").is_ok()
}

pub fn save_token(token: &StoredToken) -> Result<()> {
    if is_keyring_disabled() {
        return Ok(());
    }

    let entry = Entry::new(SERVICE_NAME, TOKEN_KEY)
        .map_err(|e| CliError::TokenStorage(format!("Failed to create keyring entry: {}", e)))?;

    let json = serde_json::to_string(token)
        .map_err(|e| CliError::TokenStorage(format!("Failed to serialize token: {}", e)))?;

    entry
        .set_password(&json)
        .map_err(|e| CliError::TokenStorage(format!("Failed to save token: {}", e)))?;

    Ok(())
}

pub fn load_token() -> Result<Option<StoredToken>> {
    if is_keyring_disabled() {
        return Ok(None);
    }

    let entry = Entry::new(SERVICE_NAME, TOKEN_KEY)
        .map_err(|e| CliError::TokenStorage(format!("Failed to create keyring entry: {}", e)))?;

    match entry.get_password() {
        Ok(json) => {
            let token: StoredToken = serde_json::from_str(&json)
                .map_err(|e| CliError::TokenStorage(format!("Failed to parse token: {}", e)))?;
            Ok(Some(token))
        }
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(CliError::TokenStorage(format!(
            "Failed to load token: {}",
            e
        ))),
    }
}

pub fn delete_token() -> Result<()> {
    let entry = Entry::new(SERVICE_NAME, TOKEN_KEY)
        .map_err(|e| CliError::TokenStorage(format!("Failed to create keyring entry: {}", e)))?;

    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(CliError::TokenStorage(format!(
            "Failed to delete token: {}",
            e
        ))),
    }
}
