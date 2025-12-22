use reqwest::Client;
use std::time::Duration;

use super::error::ApiError;
use super::types::{
    ApiCommit, ApiPullRequest, CheckInstallationResponse, CommitsResponse, CreatePullRequestParams,
    CreatePullRequestRequest, CreatePullRequestResponse, DeviceCodeRequest, DeviceCodeResponse,
    LatestReleaseResponse, PullRequestsResponse, RepoInfo, StoredToken, TokenPollRequest,
    TokenPollResponse, UserInfo,
};

const API_BASE_URL: &str = "https://api.belaf.dev";
const CLIENT_ID: &str = "belaf-cli";
const DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";

#[derive(Debug)]
pub struct ApiClient {
    client: Client,
    base_url: String,
}

impl ApiClient {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("Failed to create HTTP client"),
            base_url: std::env::var("BELAF_API_URL").unwrap_or_else(|_| API_BASE_URL.to_string()),
        }
    }

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

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
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

    pub async fn get_user_info(&self, token: &StoredToken) -> Result<UserInfo, ApiError> {
        let response = self
            .client
            .get(format!("{}/api/cli/me", self.base_url))
            .bearer_auth(&token.access_token)
            .send()
            .await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
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

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
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

    pub async fn get_commits(
        &self,
        token: &StoredToken,
        owner: &str,
        repo: &str,
        ref_name: Option<&str>,
        page: u32,
        per_page: u32,
    ) -> Result<Vec<ApiCommit>, ApiError> {
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

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ApiError::Unauthorized);
        }

        if !response.status().is_success() {
            return Err(ApiError::ApiResponse {
                status: response.status().as_u16(),
                message: response.text().await.unwrap_or_default(),
            });
        }

        let result: CommitsResponse = response.json().await?;
        Ok(result.commits)
    }

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

    pub async fn get_pull_requests(
        &self,
        token: &StoredToken,
        owner: &str,
        repo: &str,
        page: u32,
        per_page: u32,
    ) -> Result<Vec<ApiPullRequest>, ApiError> {
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

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ApiError::Unauthorized);
        }

        if !response.status().is_success() {
            return Err(ApiError::ApiResponse {
                status: response.status().as_u16(),
                message: response.text().await.unwrap_or_default(),
            });
        }

        let result: PullRequestsResponse = response.json().await?;
        Ok(result.pull_requests)
    }

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

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
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

    pub async fn get_latest_release(&self) -> Result<Option<LatestReleaseResponse>, ApiError> {
        let response = self
            .client
            .get(format!("{}/api/cli/releases/latest", self.base_url))
            .send()
            .await?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
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
}

impl Default for ApiClient {
    fn default() -> Self {
        Self::new()
    }
}
