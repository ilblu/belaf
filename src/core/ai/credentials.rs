use anyhow::{Context, Result};
use keyring::Entry;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

const SERVICE_NAME: &str = "belaf";
const CREDENTIAL_NAME: &str = "anthropic-oauth";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClaudeCredential {
    ApiKey(String),
    OAuthToken {
        access_token: String,
        refresh_token: String,
        expires_at: i64,
    },
}

impl ClaudeCredential {
    pub fn is_expired(&self) -> bool {
        match self {
            ClaudeCredential::ApiKey(_) => false,
            ClaudeCredential::OAuthToken { expires_at, .. } => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64;
                now >= *expires_at
            }
        }
    }

    pub fn access_token(&self) -> &str {
        match self {
            ClaudeCredential::ApiKey(key) => key,
            ClaudeCredential::OAuthToken { access_token, .. } => access_token,
        }
    }

    pub fn refresh_token(&self) -> Option<&str> {
        match self {
            ClaudeCredential::ApiKey(_) => None,
            ClaudeCredential::OAuthToken { refresh_token, .. } => Some(refresh_token),
        }
    }

    pub fn credential_type(&self) -> &'static str {
        match self {
            ClaudeCredential::ApiKey(_) => "API Key",
            ClaudeCredential::OAuthToken { .. } => "OAuth (Max/Pro Subscription)",
        }
    }
}

pub fn store_credentials(creds: &ClaudeCredential) -> Result<()> {
    let entry =
        Entry::new(SERVICE_NAME, CREDENTIAL_NAME).context("Failed to create keyring entry")?;

    let json = serde_json::to_string(creds).context("Failed to serialize credentials")?;

    entry
        .set_password(&json)
        .context("Failed to store credentials in keyring")?;

    Ok(())
}

pub fn load_credentials() -> Result<Option<ClaudeCredential>> {
    let entry = match Entry::new(SERVICE_NAME, CREDENTIAL_NAME) {
        Ok(e) => e,
        Err(_) => return Ok(None),
    };

    match entry.get_password() {
        Ok(json) => {
            let creds: ClaudeCredential =
                serde_json::from_str(&json).context("Failed to parse stored credentials")?;
            Ok(Some(creds))
        }
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(anyhow::anyhow!("Failed to read credentials: {}", e)),
    }
}

pub fn delete_credentials() -> Result<()> {
    let entry =
        Entry::new(SERVICE_NAME, CREDENTIAL_NAME).context("Failed to create keyring entry")?;

    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(anyhow::anyhow!("Failed to delete credentials: {}", e)),
    }
}

pub fn resolve_credential() -> Result<ClaudeCredential> {
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        return Ok(ClaudeCredential::ApiKey(key));
    }

    if let Some(creds) = load_credentials()? {
        if creds.is_expired() {
            if let ClaudeCredential::OAuthToken { refresh_token, .. } = &creds {
                tracing::info!("OAuth token expired, attempting refresh...");
                let refreshed = refresh_oauth_token(refresh_token)?;
                return Ok(refreshed);
            }
        }
        return Ok(creds);
    }

    anyhow::bail!("No credentials found. Run `belaf ai login` or set ANTHROPIC_API_KEY")
}

fn refresh_oauth_token(refresh_token: &str) -> Result<ClaudeCredential> {
    use crate::core::ai::oauth;

    let future = oauth::refresh_token(refresh_token);

    let tokens = match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(future)),
        Err(_) => {
            let rt = tokio::runtime::Runtime::new().context("failed to create async runtime")?;
            rt.block_on(future)
        }
    }
    .context("failed to refresh OAuth token")?;

    let credential = ClaudeCredential::OAuthToken {
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        expires_at: now_unix() + tokens.expires_in,
    };

    store_credentials(&credential)?;
    tracing::info!("OAuth token refreshed successfully");

    Ok(credential)
}

pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}
