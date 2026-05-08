use reqwest::{Client, Response, StatusCode};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::time::Duration;

use super::error::ApiError;
use super::oidc::fetch_actions_oidc_jwt;
use super::types::{
    ApiCommit, ApiPullRequest, CheckInstallationResponse, CommitsResponse, CreatePullRequestParams,
    CreatePullRequestRequest, CreatePullRequestResponse, DeviceCodeRequest, DeviceCodeResponse,
    GitCredentialsResponse, OidcExchangeRequest, OidcExchangeResponse, PullRequestsResponse,
    StoredToken, TokenPollRequest, TokenPollResponse, UserInfo,
};

#[derive(Deserialize)]
struct LimitExceededPayload {
    code: Option<String>,
    tier: Option<String>,
    current: Option<i64>,
    limit: Option<String>,
    upgrade_url: Option<String>,
}

const API_BASE_URL: &str = "https://api.belaf.dev";
const CLIENT_ID: &str = "belaf-cli";
const DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
const DEFAULT_TIMEOUT_SECS: u64 = 30;

#[derive(Debug)]
pub struct ApiClient {
    client: Client,
    base_url: String,
}

impl ApiClient {
    /// Creates a new API client with default configuration.
    ///
    /// Uses `BELAF_API_URL` environment variable if set, otherwise defaults
    /// to the production API URL.
    ///
    /// # Panics
    ///
    /// This function will panic if the HTTP client cannot be created, which
    /// should only happen in exceptional circumstances (e.g., TLS configuration
    /// failure on the system).
    pub fn new() -> Self {
        Self::try_new().expect("Failed to create HTTP client - TLS or system configuration error")
    }

    /// Creates a new API client, returning an error if construction fails.
    ///
    /// Prefer this over `new()` when you need to handle client creation failures
    /// gracefully.
    pub fn try_new() -> Result<Self, ApiError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .build()
            .map_err(|e| ApiError::ClientCreation(e.to_string()))?;

        let base_url = std::env::var("BELAF_API_URL").unwrap_or_else(|_| API_BASE_URL.to_string());

        validate_api_url(&base_url)?;

        Ok(Self { client, base_url })
    }

    /// Creates a new API client with a custom base URL.
    ///
    /// Primarily used for testing with mock servers.
    #[cfg(test)]
    pub fn with_base_url(base_url: &str) -> Result<Self, ApiError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .build()
            .map_err(|e| ApiError::ClientCreation(e.to_string()))?;

        Ok(Self {
            client,
            base_url: base_url.to_string(),
        })
    }

    /// Handles common response processing: status code checking and JSON deserialization.
    async fn handle_response<T: DeserializeOwned>(response: Response) -> Result<T, ApiError> {
        if response.status() == StatusCode::UNAUTHORIZED {
            return Err(ApiError::Unauthorized);
        }

        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(60);
            return Err(ApiError::RateLimited {
                retry_after_secs: retry_after,
            });
        }

        // 402 Payment Required is reserved for tier-limit signalling on
        // action-edge endpoints (createPull, git/credentials). The server
        // emits the same `ErrorResponse` envelope but with `code` and the
        // structured tier/current/limit/upgrade_url fields populated.
        if response.status() == StatusCode::PAYMENT_REQUIRED {
            let body = response.text().await.unwrap_or_default();
            if let Ok(payload) = serde_json::from_str::<LimitExceededPayload>(&body) {
                if payload.code.as_deref() == Some("repository_limit_exceeded") {
                    return Err(ApiError::LimitExceeded {
                        tier: payload.tier.unwrap_or_default(),
                        current: payload.current.unwrap_or(0),
                        limit: payload.limit.unwrap_or_else(|| "?".to_string()),
                        upgrade_url: payload.upgrade_url.unwrap_or_default(),
                    });
                }
            }
            return Err(ApiError::ApiResponse {
                status: 402,
                message: body,
            });
        }

        if !response.status().is_success() {
            return Err(ApiError::ApiResponse {
                status: response.status().as_u16(),
                message: response.text().await.unwrap_or_default(),
            });
        }

        Ok(response.json().await?)
    }

    /// Requests a device authorization code from the belaf API.
    ///
    /// This initiates the OAuth 2.0 Device Flow. The returned codes should be
    /// displayed to the user, who will authorize the application via a browser.
    ///
    /// # Errors
    ///
    /// Returns [`ApiError::ApiResponse`] if the API request fails, or
    /// [`ApiError::Request`] for network errors.
    pub async fn request_device_code(&self) -> Result<DeviceCodeResponse, ApiError> {
        let response = self
            .client
            .post(format!("{}/api/auth/device/code", self.base_url))
            .json(&DeviceCodeRequest {
                client_id: CLIENT_ID.to_string(),
                scope: "cli".to_string(),
            })
            .send()
            .await?;

        Self::handle_response(response).await
    }

    /// Polls for an access token during the device authorization flow.
    ///
    /// Should be called repeatedly with the device code from [`request_device_code`]
    /// until an access token is returned or an error occurs.
    ///
    /// # Returns
    ///
    /// Returns `TokenPollResponse` which may contain:
    /// - An access token on success
    /// - An error code like "authorization_pending" or "slow_down"
    pub async fn poll_for_token(&self, device_code: &str) -> Result<TokenPollResponse, ApiError> {
        let response = self
            .client
            .post(format!("{}/api/auth/device/token", self.base_url))
            .json(&TokenPollRequest {
                client_id: CLIENT_ID.to_string(),
                device_code: device_code.to_string(),
                grant_type: DEVICE_GRANT_TYPE.to_string(),
            })
            .send()
            .await?;

        Ok(response.json().await?)
    }

    /// Exchanges a GitHub Actions OIDC JWT for a short-lived belaf CI token.
    ///
    /// This is the CI authentication path: the runner mints an OIDC JWT
    /// (audience = belaf API URL), the API verifies it against GitHub's
    /// JWKS and matches the `repository` claim to an installed GitHub App
    /// installation. The returned token is a self-signed HS256 JWT (30 min
    /// TTL) that the CLI uses as a normal Bearer for `/api/cli/*` calls.
    ///
    /// The returned `StoredToken` should not be persisted to the keyring —
    /// it is short-lived and tied to the specific repo/run.
    pub async fn exchange_oidc_token(&self, oidc_jwt: String) -> Result<StoredToken, ApiError> {
        let response = self
            .client
            .post(format!("{}/api/cli/auth/oidc/exchange", self.base_url))
            .json(&OidcExchangeRequest { token: oidc_jwt })
            .send()
            .await?;

        let parsed: OidcExchangeResponse = Self::handle_response(response).await?;

        // The API returns an absolute RFC3339 `expires_at`. Convert it to
        // the `OffsetDateTime` shape `StoredToken` carries.
        let expires_at = match time::OffsetDateTime::parse(
            &parsed.expires_at,
            &time::format_description::well_known::Rfc3339,
        ) {
            Ok(dt) => Some(dt),
            Err(e) => {
                tracing::warn!(
                    "exchange_oidc_token: could not parse expires_at \"{}\": {}",
                    parsed.expires_at,
                    e
                );
                None
            }
        };

        Ok(StoredToken {
            access_token: parsed.access_token,
            expires_at,
        })
    }

    /// Convenience wrapper: fetches the GitHub Actions OIDC JWT from the
    /// runner and exchanges it for a belaf CI token in one shot.
    ///
    /// Returns `ApiError::InvalidConfiguration` if the runner env vars
    /// (`ACTIONS_ID_TOKEN_REQUEST_*`) are not set.
    pub async fn fetch_and_exchange_actions_oidc(&self) -> Result<StoredToken, ApiError> {
        let jwt = fetch_actions_oidc_jwt(&self.client, &self.base_url).await?;
        self.exchange_oidc_token(jwt).await
    }

    /// Checks if the GitHub App is installed for a repository.
    ///
    /// # Arguments
    ///
    /// * `token` - The authenticated user's API token
    /// * `repo` - Repository in "owner/repo" format
    ///
    /// # Errors
    ///
    /// Returns [`ApiError::Unauthorized`] if the token is invalid or expired.
    pub async fn check_installation(
        &self,
        token: &StoredToken,
        repo: &str,
    ) -> Result<CheckInstallationResponse, ApiError> {
        let response = self
            .client
            .get(format!("{}/api/cli/check-installation", self.base_url))
            .query(&[("repo", repo)])
            .bearer_auth(&token.access_token)
            .send()
            .await?;

        Self::handle_response(response).await
    }

    /// Gets the authenticated user's profile information.
    ///
    /// # Errors
    ///
    /// Returns [`ApiError::Unauthorized`] if the token is invalid or expired.
    pub async fn get_user_info(&self, token: &StoredToken) -> Result<UserInfo, ApiError> {
        let response = self
            .client
            .get(format!("{}/api/cli/me", self.base_url))
            .bearer_auth(&token.access_token)
            .send()
            .await?;

        Self::handle_response(response).await
    }

    /// Gets commits for a repository with pagination.
    ///
    /// # Arguments
    ///
    /// * `page` - 1-indexed page number
    /// * `per_page` - Number of commits per page (max 100)
    pub async fn get_commits(
        &self,
        token: &StoredToken,
        owner: &str,
        repo: &str,
        ref_name: Option<&str>,
        page: u32,
        per_page: u32,
    ) -> Result<Vec<ApiCommit>, ApiError> {
        let per_page = per_page.min(100);
        let mut url = format!(
            "{}/api/cli/repos/{}/{}/commits?per_page={}&page={}",
            self.base_url, owner, repo, per_page, page
        );

        if let Some(r) = ref_name {
            url.push_str(&format!("&ref={}", r));
        }

        let response = self
            .client
            .get(&url)
            .bearer_auth(&token.access_token)
            .send()
            .await?;

        let result: CommitsResponse = Self::handle_response(response).await?;
        Ok(result.commits)
    }

    /// Gets closed pull requests for a repository with pagination.
    pub async fn get_pull_requests(
        &self,
        token: &StoredToken,
        owner: &str,
        repo: &str,
        page: u32,
        per_page: u32,
    ) -> Result<Vec<ApiPullRequest>, ApiError> {
        let per_page = per_page.min(100);
        let url = format!(
            "{}/api/cli/repos/{}/{}/pulls?state=closed&per_page={}&page={}",
            self.base_url, owner, repo, per_page, page
        );

        let response = self
            .client
            .get(&url)
            .bearer_auth(&token.access_token)
            .send()
            .await?;

        let result: PullRequestsResponse = Self::handle_response(response).await?;
        Ok(result.pull_requests)
    }

    /// Creates a new pull request.
    pub async fn create_pull_request(
        &self,
        params: CreatePullRequestParams<'_>,
    ) -> Result<CreatePullRequestResponse, ApiError> {
        let response = self
            .client
            .post(format!(
                "{}/api/cli/repos/{}/{}/pulls",
                self.base_url, params.owner, params.repo
            ))
            .bearer_auth(&params.token.access_token)
            .json(&CreatePullRequestRequest {
                title: params.title.to_string(),
                head: params.head.to_string(),
                base: params.base.to_string(),
                body: Some(params.body.to_string()),
            })
            .send()
            .await?;

        Self::handle_response(response).await
    }

    /// Gets temporary Git credentials for pushing to a repository.
    ///
    /// The returned token can be used with git operations that require
    /// authentication (e.g., push). It has a short expiry time.
    pub async fn get_git_credentials(
        &self,
        token: &StoredToken,
        owner: &str,
        repo: &str,
    ) -> Result<GitCredentialsResponse, ApiError> {
        let response = self
            .client
            .get(format!(
                "{}/api/cli/repos/{}/{}/git/credentials",
                self.base_url, owner, repo
            ))
            .bearer_auth(&token.access_token)
            .send()
            .await?;

        Self::handle_response(response).await
    }

    /// Report the current drift state to the dashboard. `uncovered_paths`
    /// is empty when the local detector found no drift; populated with
    /// the same paths the CLI would print as the abort error otherwise.
    /// Errors are swallowed by the caller — this is best-effort
    /// telemetry, not a release-blocking call.
    pub async fn report_drift(
        &self,
        token: &StoredToken,
        owner: &str,
        repo: &str,
        uncovered_paths: Vec<String>,
    ) -> Result<(), ApiError> {
        let response = self
            .client
            .post(format!(
                "{}/api/cli/repos/{}/{}/drift",
                self.base_url, owner, repo
            ))
            .bearer_auth(&token.access_token)
            .json(&serde_json::json!({ "uncovered_paths": uncovered_paths }))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(ApiError::ApiResponse {
                status,
                message: body,
            });
        }
        Ok(())
    }
}

impl Default for ApiClient {
    fn default() -> Self {
        Self::new()
    }
}

fn validate_api_url(url: &str) -> Result<(), ApiError> {
    if url.starts_with("https://") {
        return Ok(());
    }

    if url.starts_with("http://localhost") || url.starts_with("http://127.0.0.1") {
        return Ok(());
    }

    Err(ApiError::InvalidConfiguration(
        "BELAF_API_URL must use HTTPS (or localhost for development)".into(),
    ))
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod client_tests;
