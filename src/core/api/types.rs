use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

// ----------------------------------------------------------------------------
// Wire types — re-exported from the progenitor-generated module.
// These mirror the Zod schemas in github-app/apps/api/src/routes/cli/schemas.ts
// 1:1; any drift is caught at compile time when the generated module rebuilds.
// Do NOT add hand-written wire structs here.
// ----------------------------------------------------------------------------

pub use crate::core::api::generated::types::{
    ApiCommit, ApiPullRequest, CheckInstallationResponse, CommitAuthor, CommitsResponse,
    CreatePullRequestRequest, CreatePullRequestResponse, ErrorResponse, GitCredentialsResponse,
    OidcExchangeRequest, OidcExchangeResponse, PullRequestsResponse, UserInfo,
};

impl UserInfo {
    pub fn display_name(&self) -> &str {
        self.username
            .as_deref()
            .or(self.name.as_deref())
            .unwrap_or("Unknown")
    }
}

// ----------------------------------------------------------------------------
// Hand-written types
//
// `DeviceCode*` / `TokenPoll*` belong to Better-Auth's device-flow endpoints
// (`/api/auth/device/*`), which are NOT under `/api/cli/*` and therefore not
// part of the OpenAPI contract. They stay hand-written.
//
// `StoredToken` is local on-disk storage, not a wire format.
//
// `CreatePullRequestParams` is a borrowed builder used only inside
// `client.rs` — pure ergonomic, no wire involvement.
// ----------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct DeviceCodeRequest {
    pub client_id: String,
    pub scope: String,
}

#[derive(Debug, Deserialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Debug, Serialize)]
pub struct TokenPollRequest {
    pub client_id: String,
    pub device_code: String,
    pub grant_type: String,
}

#[derive(Debug, Deserialize)]
pub struct TokenPollResponse {
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub expires_in: Option<u64>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub error_description: Option<String>,
}

impl TokenPollResponse {
    pub fn is_success(&self) -> bool {
        self.access_token.is_some()
    }

    pub fn error_code(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

/// A stored API token with optional expiration timestamp.
///
/// The belaf API always returns `expires_in` with tokens, so `expires_at` should
/// always be `Some` for tokens obtained through normal authentication flows.
/// The `None` case is handled defensively for backwards compatibility with
/// tokens stored before expiry tracking was added.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredToken {
    pub access_token: String,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,
}

impl StoredToken {
    /// Creates a new stored token with an optional expiry time.
    ///
    /// If `expires_in_secs` is provided, the expiry timestamp is calculated
    /// from the current time. The belaf API should always provide this value.
    ///
    /// Values larger than `i64::MAX` seconds are clamped to prevent overflow.
    pub fn new(access_token: String, expires_in_secs: Option<u64>) -> Self {
        let expires_at = expires_in_secs.map(|secs| {
            let safe_secs = i64::try_from(secs).unwrap_or(i64::MAX);
            OffsetDateTime::now_utc() + time::Duration::seconds(safe_secs)
        });
        Self {
            access_token,
            expires_at,
        }
    }

    /// Checks if the token has expired or will expire soon.
    ///
    /// # Behavior
    ///
    /// - Returns `true` if the token expires within 60 seconds
    /// - Returns `true` if no expiry time is set (fail-safe: assumes expired)
    ///
    /// The 60-second buffer prevents race conditions where a token passes
    /// the expiry check but expires before the API request completes.
    ///
    /// The fail-safe behavior for missing expiry ensures that tokens without
    /// proper expiration tracking (e.g., legacy tokens or API bugs) will
    /// trigger re-authentication rather than potentially using invalid tokens.
    pub fn is_expired(&self) -> bool {
        const EXPIRY_BUFFER_SECS: i64 = 60;

        match self.expires_at {
            Some(exp) => {
                exp < OffsetDateTime::now_utc() + time::Duration::seconds(EXPIRY_BUFFER_SECS)
            }
            None => {
                tracing::warn!("Token has no expiry timestamp - treating as expired for safety");
                true
            }
        }
    }
}

#[derive(Debug)]
pub struct CreatePullRequestParams<'a> {
    pub token: &'a StoredToken,
    pub owner: &'a str,
    pub repo: &'a str,
    pub title: &'a str,
    pub head: &'a str,
    pub base: &'a str,
    pub body: &'a str,
}
