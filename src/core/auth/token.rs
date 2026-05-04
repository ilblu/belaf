use crate::core::api::oidc::is_actions_oidc_available;
use crate::core::api::{ApiClient, StoredToken};
use crate::error::{CliError, Result};
use keyring::Entry;
use tracing::{info, warn};

const SERVICE_NAME: &str = "belaf";
const TOKEN_KEY: &str = "api-token";

fn is_keyring_disabled() -> bool {
    std::env::var("BELAF_NO_KEYRING").is_ok()
}

pub fn save_token(token: &StoredToken) -> Result<()> {
    if is_keyring_disabled() {
        warn!("BELAF_NO_KEYRING is set - token will not be persisted");
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

/// Loads a token for outbound `/api/cli/*` calls, with an OIDC fallback for CI.
///
/// Resolution order:
/// 1. Existing token from the OS keyring (interactive `belaf install` path).
/// 2. If `ACTIONS_ID_TOKEN_REQUEST_*` env vars are set (GitHub Actions runner
///    with `permissions: id-token: write`), mint an OIDC JWT and exchange it
///    via `POST /api/cli/auth/oidc/exchange`. The result is **not** persisted
///    to the keyring — CI tokens are short-lived and tied to the run.
/// 3. Otherwise return `Ok(None)` — caller is expected to bail with a
///    "run `belaf install` first" message.
///
/// If OIDC env vars are present but the exchange fails, the error is surfaced
/// rather than silently swallowed: we know the user *intended* CI auth, so a
/// misleading "run belaf install" message would be unhelpful.
pub async fn load_or_exchange_token(client: &ApiClient) -> Result<Option<StoredToken>> {
    if let Some(token) = load_token()? {
        return Ok(Some(token));
    }

    if !is_actions_oidc_available() {
        return Ok(None);
    }

    info!("no keyring token; falling back to GitHub Actions OIDC exchange");
    match client.fetch_and_exchange_actions_oidc().await {
        Ok(token) => Ok(Some(token)),
        Err(e) => Err(CliError::TokenStorage(format!(
            "GitHub Actions OIDC exchange failed: {e}"
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
