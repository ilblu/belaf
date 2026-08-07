use anyhow::{anyhow, Context};
use tracing::info;

use crate::core::api::{ApiClient, ApiPullRequest, CreatePullRequestParams, StoredToken};
use crate::core::auth::token::load_or_exchange_token;
use crate::core::errors::Result;
use crate::core::git::repository::Repository;
use crate::core::session::AppSession;

/// What `create_or_update_pull_request` did with the release PR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrAction {
    Created,
    Updated,
}

impl PrAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Updated => "updated",
        }
    }
}

pub struct GitHubInformation {
    owner: String,
    repo: String,
    api_client: ApiClient,
    token: StoredToken,
}

impl GitHubInformation {
    pub fn new(sess: &AppSession) -> Result<Self> {
        let api_client = ApiClient::new();

        // `load_or_exchange_token`, not `load_token`: on a CI runner the
        // keyring is empty and the only credential is the GitHub Actions OIDC
        // JWT, which this exchanges for a belaf token. Using the plain loader
        // here used to let the push succeed (it goes through
        // `fetch_git_credentials`, which does exchange) and then fail on the
        // pull request.
        let token = block_on(load_or_exchange_token(&api_client))?
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
            api_client,
            token,
        })
    }

    pub fn new_with_scopes(sess: &AppSession, _required_scopes: &[&str]) -> Result<Self> {
        Self::new(sess)
    }

    /// The open pull request whose head is `head`, if there is one.
    pub fn find_open_pull_request(&self, head: &str) -> Result<Option<ApiPullRequest>> {
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let token = self.token.clone();
        let head = head.to_string();
        let api_client = self.api_client.clone();

        block_on(async move {
            api_client
                .find_open_pull_request(&token, &owner, &repo, &head)
                .await
                .map_err(map_api_error)
        })?
    }

    /// Point the release pull request at the current state of `head`.
    ///
    /// Updates the open PR from that branch if there is one, otherwise opens
    /// a new one. This is what keeps repeated `prepare` runs down to a single
    /// release PR instead of one per run.
    ///
    /// Returns the PR's URL and which of the two happened.
    pub fn create_or_update_pull_request(
        &self,
        head: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<(String, PrAction)> {
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let token = self.token.clone();
        let title = title.to_string();
        let head = head.to_string();
        let base = base.to_string();
        let body = body.to_string();
        let api_client = self.api_client.clone();

        block_on(async move {
            if let Some(existing) = api_client
                .find_open_pull_request(&token, &owner, &repo, &head)
                .await
                .map_err(map_api_error)?
            {
                let updated = api_client
                    .update_pull_request(&token, &owner, &repo, existing.number, &title, &body)
                    .await
                    .map_err(map_api_error)?;

                info!("updated pull request: {}", updated.html_url);
                return Ok((updated.html_url, PrAction::Updated));
            }

            let create_result = api_client
                .create_pull_request(CreatePullRequestParams {
                    token: &token,
                    owner: &owner,
                    repo: &repo,
                    title: &title,
                    head: &head,
                    base: &base,
                    body: &body,
                })
                .await;

            match create_result {
                Ok(pr) => {
                    info!("created pull request: {}", pr.html_url);
                    Ok((pr.html_url, PrAction::Created))
                }

                // 422 means GitHub already has an open PR for this head. We
                // looked and found none, so another run opened one in the
                // meantime — take the same update path rather than failing a
                // run that has already pushed its commit.
                Err(crate::core::api::ApiError::ApiResponse {
                    status: 422,
                    message,
                }) => {
                    let existing = api_client
                        .find_open_pull_request(&token, &owner, &repo, &head)
                        .await
                        .map_err(map_api_error)?
                        .ok_or_else(|| anyhow!("pull request creation failed: {}", message))?;

                    let updated = api_client
                        .update_pull_request(&token, &owner, &repo, existing.number, &title, &body)
                        .await
                        .map_err(map_api_error)?;

                    info!(
                        "updated pull request opened by a concurrent run: {}",
                        updated.html_url
                    );
                    Ok((updated.html_url, PrAction::Updated))
                }

                Err(e) => Err(map_api_error(e)),
            }
        })?
    }
}

/// Mint a short-lived installation token for git operations against the
/// upstream remote.
///
/// This is *not* the belaf API bearer token: that one authenticates against
/// `api.belaf.dev`, while git needs a GitHub installation token. The API
/// token is only the credential used to ask for this one.
///
/// Used by both the release push and the pre-flight tag fetch — a private
/// repo over HTTPS rejects an unauthenticated fetch, so the fetch needs the
/// same credential the push does.
pub fn fetch_git_credentials(repo: &Repository) -> Result<String> {
    let upstream_url = repo.upstream_url().context("failed to get upstream URL")?;
    let (owner, name) =
        parse_github_url(&upstream_url).context("failed to parse GitHub URL from upstream")?;

    let api_client = ApiClient::new();

    let credentials = block_on(async {
        let token = load_or_exchange_token(&api_client)
            .await
            .context("failed to load token")?
            .context(
                "not authenticated — run 'belaf install' (interactive) or run from a \
                 GitHub Actions job with `permissions: id-token: write` set",
            )?;

        api_client
            .get_git_credentials(&token, &owner, &name)
            .await
            .map_err(|e| {
                if matches!(e, crate::core::api::ApiError::Unauthorized) {
                    anyhow!("authentication expired - run 'belaf login' to re-authenticate")
                } else {
                    map_api_error(e)
                }
            })
    })??;

    Ok(credentials.token)
}

fn map_api_error(e: crate::core::api::ApiError) -> anyhow::Error {
    match &e {
        crate::core::api::ApiError::ApiResponse { status, message } => {
            anyhow!("GitHub API error ({}): {}", status, message)
        }
        _ => anyhow!("{}", e),
    }
}

/// Run an async call from this synchronous module, reusing the ambient
/// runtime when there is one. The outer `Result` is the runtime itself
/// failing to start; the future's own output stays untouched inside it.
fn block_on<F: std::future::Future>(future: F) -> Result<F::Output> {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => Ok(tokio::task::block_in_place(|| handle.block_on(future))),
        Err(_) => {
            let rt = tokio::runtime::Runtime::new().context("failed to create async runtime")?;
            Ok(rt.block_on(future))
        }
    }
}

impl Clone for ApiClient {
    fn clone(&self) -> Self {
        Self::new()
    }
}

pub fn parse_github_url(url: &str) -> Result<(String, String)> {
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
