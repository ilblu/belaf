use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
}

impl StoredToken {
    /// Creates a new stored token with an optional expiry time.
    ///
    /// If `expires_in_secs` is provided, the expiry timestamp is calculated
    /// from the current time. The belaf API should always provide this value.
    pub fn new(access_token: String, expires_in_secs: Option<u64>) -> Self {
        let expires_at =
            expires_in_secs.map(|secs| Utc::now() + chrono::Duration::seconds(secs as i64));
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
            Some(exp) => exp < Utc::now() + chrono::Duration::seconds(EXPIRY_BUFFER_SECS),
            None => {
                tracing::warn!("Token has no expiry timestamp - treating as expired for safety");
                true
            }
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CheckInstallationResponse {
    pub installed: bool,
    #[serde(default)]
    pub installation_id: Option<i64>,
    #[serde(default)]
    pub repository_id: Option<i64>,
    #[serde(default)]
    pub install_url: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UserInfo {
    pub id: String,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
}

impl UserInfo {
    pub fn display_name(&self) -> &str {
        self.username
            .as_deref()
            .or(self.name.as_deref())
            .unwrap_or("Unknown")
    }
}

#[derive(Debug, Deserialize)]
pub struct RepoInfo {
    pub full_name: String,
    pub default_branch: String,
    #[serde(default)]
    pub private: bool,
    #[serde(default)]
    pub installation_id: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommitAuthor {
    #[serde(default)]
    pub login: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiCommit {
    pub sha: String,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub author: Option<CommitAuthor>,
    #[serde(default)]
    pub timestamp: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CommitsResponse {
    pub commits: Vec<ApiCommit>,
    #[serde(default)]
    pub has_more: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiPullRequest {
    pub number: i64,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub merge_commit_sha: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub merged_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PullRequestsResponse {
    pub pull_requests: Vec<ApiPullRequest>,
    #[serde(default)]
    pub has_more: bool,
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

#[derive(Debug, Serialize)]
pub struct CreatePullRequestRequest {
    pub title: String,
    pub head: String,
    pub base: String,
    pub body: String,
}

#[derive(Debug, Deserialize)]
pub struct CreatePullRequestResponse {
    pub number: i64,
    pub html_url: String,
    #[serde(default)]
    pub state: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GitPushFile {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct GitPushRequest {
    pub branch: String,
    pub base: String,
    pub files: Vec<GitPushFile>,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct GitPushResponse {
    pub sha: String,
    pub branch: String,
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GitRef {
    #[serde(rename = "ref")]
    pub ref_name: String,
    pub sha: String,
}

#[derive(Debug, Deserialize)]
pub struct RefsResponse {
    pub refs: Vec<GitRef>,
}

#[derive(Debug, Deserialize)]
pub struct CompareResponse {
    pub ahead_by: i64,
    pub behind_by: i64,
    pub commits: Vec<ApiCommit>,
}

#[derive(Debug, Deserialize)]
pub struct LatestReleaseResponse {
    pub tag_name: String,
    pub version: String,
    pub html_url: String,
    #[serde(default)]
    pub published_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GitCredentialsResponse {
    pub token: String,
    pub expires_at: String,
}
