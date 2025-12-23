use reqwest::{Client, Response, StatusCode};
use serde::de::DeserializeOwned;
use std::time::Duration;
use tracing::warn;

use super::error::ApiError;
use super::types::{
    ApiCommit, ApiPullRequest, CheckInstallationResponse, CommitsResponse, CreatePullRequestParams,
    CreatePullRequestRequest, CreatePullRequestResponse, DeviceCodeRequest, DeviceCodeResponse,
    GitCredentialsResponse, GitPushRequest, GitPushResponse, LatestReleaseResponse,
    PullRequestsResponse, RepoInfo, StoredToken, TokenPollRequest, TokenPollResponse, UserInfo,
};

const API_BASE_URL: &str = "https://api.belaf.dev";
const CLIENT_ID: &str = "belaf-cli";
const DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_PAGINATION_PAGES: u32 = 100;

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

        Ok(Self {
            client,
            base_url: std::env::var("BELAF_API_URL").unwrap_or_else(|_| API_BASE_URL.to_string()),
        })
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

        if !response.status().is_success() {
            return Err(ApiError::ApiResponse {
                status: response.status().as_u16(),
                message: response.text().await.unwrap_or_default(),
            });
        }

        Ok(response.json().await?)
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

    /// Gets repository information including default branch and installation status.
    ///
    /// # Errors
    ///
    /// Returns [`ApiError::Unauthorized`] if the token is invalid or expired.
    pub async fn get_repo_info(
        &self,
        token: &StoredToken,
        owner: &str,
        repo: &str,
    ) -> Result<RepoInfo, ApiError> {
        let response = self
            .client
            .get(format!(
                "{}/api/cli/repos/{}/{}",
                self.base_url, owner, repo
            ))
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

    /// Gets all commits for a repository, handling pagination automatically.
    ///
    /// Limited to [`MAX_PAGINATION_PAGES`] pages to prevent infinite loops.
    pub async fn get_all_commits(
        &self,
        token: &StoredToken,
        owner: &str,
        repo: &str,
        ref_name: Option<&str>,
    ) -> Result<Vec<ApiCommit>, ApiError> {
        let mut all_commits = Vec::new();
        let mut page = 1;
        let per_page = 100;

        loop {
            if page > MAX_PAGINATION_PAGES {
                warn!(
                    "Reached maximum pagination limit ({} pages) while fetching commits",
                    MAX_PAGINATION_PAGES
                );
                break;
            }

            let commits = self
                .get_commits(token, owner, repo, ref_name, page, per_page)
                .await?;

            if commits.is_empty() {
                break;
            }

            all_commits.extend(commits);
            page += 1;
        }

        Ok(all_commits)
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

    /// Gets all closed pull requests for a repository.
    ///
    /// Limited to [`MAX_PAGINATION_PAGES`] pages to prevent infinite loops.
    pub async fn get_all_pull_requests(
        &self,
        token: &StoredToken,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<ApiPullRequest>, ApiError> {
        let mut all_prs = Vec::new();
        let mut page = 1;
        let per_page = 100;

        loop {
            if page > MAX_PAGINATION_PAGES {
                warn!(
                    "Reached maximum pagination limit ({} pages) while fetching pull requests",
                    MAX_PAGINATION_PAGES
                );
                break;
            }

            let prs = self
                .get_pull_requests(token, owner, repo, page, per_page)
                .await?;

            if prs.is_empty() {
                break;
            }

            all_prs.extend(prs);
            page += 1;
        }

        Ok(all_prs)
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
                body: params.body.to_string(),
            })
            .send()
            .await?;

        Self::handle_response(response).await
    }

    /// Gets the latest CLI release information.
    ///
    /// Returns `None` if no releases exist.
    pub async fn get_latest_release(&self) -> Result<Option<LatestReleaseResponse>, ApiError> {
        let response = self
            .client
            .get(format!("{}/api/cli/releases/latest", self.base_url))
            .send()
            .await?;

        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }

        if !response.status().is_success() {
            return Err(ApiError::ApiResponse {
                status: response.status().as_u16(),
                message: response.text().await.unwrap_or_default(),
            });
        }

        Ok(Some(response.json().await?))
    }

    /// Pushes files to a repository via the API.
    pub async fn git_push(
        &self,
        token: &StoredToken,
        owner: &str,
        repo: &str,
        request: &GitPushRequest,
    ) -> Result<GitPushResponse, ApiError> {
        let response = self
            .client
            .post(format!(
                "{}/api/cli/repos/{}/{}/git/push",
                self.base_url, owner, repo
            ))
            .bearer_auth(&token.access_token)
            .json(request)
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
}

impl Default for ApiClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod client_tests;
