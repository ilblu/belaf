use anyhow::{anyhow, Context};
use tracing::info;

use crate::core::api::{ApiClient, CreatePullRequestParams, StoredToken};
use crate::core::auth::token::load_token;
use crate::core::errors::Result;
use crate::core::session::AppSession;

pub struct GitHubInformation {
    owner: String,
    repo: String,
    api_client: ApiClient,
    token: StoredToken,
}

impl GitHubInformation {
    pub fn new(sess: &AppSession) -> Result<Self> {
        let token = load_token()
            .map_err(|e| anyhow!("Failed to load token: {}", e))?
            .ok_or_else(|| {
                anyhow!("Authentication required. Run 'belaf install' to authenticate.")
            })?;

        if token.is_expired() {
            return Err(anyhow!(
                "Token expired. Run 'belaf install' to re-authenticate."
            ));
        }

        let upstream_url = sess.repo.upstream_url()?;
        info!("upstream url: {}", upstream_url);

        let (owner, repo) = parse_github_url(&upstream_url)?;

        Ok(GitHubInformation {
            owner,
            repo,
            api_client: ApiClient::new(),
            token,
        })
    }

    pub fn new_with_scopes(sess: &AppSession, _required_scopes: &[&str]) -> Result<Self> {
        Self::new(sess)
    }

    pub fn create_pull_request(
        &self,
        head: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<String> {
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let token = self.token.clone();
        let title = title.to_string();
        let head = head.to_string();
        let base = base.to_string();
        let body = body.to_string();
        let api_client = self.api_client.clone();

        let future = async move {
            let params = CreatePullRequestParams {
                token: &token,
                owner: &owner,
                repo: &repo,
                title: &title,
                head: &head,
                base: &base,
                body: &body,
            };

            let pr = api_client
                .create_pull_request(params)
                .await
                .context("failed to create pull request")?;

            info!("created pull request: {}", pr.html_url);
            Ok(pr.html_url)
        };

        match tokio::runtime::Handle::try_current() {
            Ok(handle) => tokio::task::block_in_place(|| handle.block_on(future)),
            Err(_) => {
                let rt =
                    tokio::runtime::Runtime::new().context("failed to create async runtime")?;
                rt.block_on(future)
            }
        }
    }
}

impl Clone for ApiClient {
    fn clone(&self) -> Self {
        Self::new()
    }
}

fn parse_github_url(url: &str) -> Result<(String, String)> {
    if let Some(rest) = url.strip_prefix("git@github.com:") {
        let repo = rest.trim_end_matches(".git");
        let parts: Vec<&str> = repo.split('/').collect();
        if parts.len() == 2 {
            return Ok((parts[0].to_string(), parts[1].to_string()));
        }
    }

    if let Some(rest) = url.strip_prefix("https://github.com/") {
        let repo = rest.trim_end_matches(".git");
        let parts: Vec<&str> = repo.split('/').collect();
        if parts.len() >= 2 {
            return Ok((parts[0].to_string(), parts[1].to_string()));
        }
    }

    Err(anyhow!("Could not parse GitHub URL: {}", url))
}
