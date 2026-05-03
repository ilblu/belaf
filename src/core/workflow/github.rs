//! GitHub remote helpers used by the release pipeline.
//!
//! Extracted from `core::workflow` to keep the orchestrator focused on the
//! release flow rather than GitHub plumbing. Re-exported through
//! `core::workflow::*` for compatibility with existing call sites.

use anyhow::Result;

use crate::core::git::repository::Repository;

pub struct GitHubRemoteInfo {
    pub owner: String,
    pub repo: String,
}

pub fn extract_github_remote(repo: &Repository) -> Option<GitHubRemoteInfo> {
    let upstream_url = repo.upstream_url().ok()?;
    let parsed = git_url_parse::GitUrl::parse(&upstream_url).ok()?;
    let provider: git_url_parse::types::provider::GenericProvider = parsed.provider_info().ok()?;

    Some(GitHubRemoteInfo {
        owner: provider.owner().to_string(),
        repo: provider.repo().to_string(),
    })
}

pub fn load_github_token() -> Option<crate::core::api::StoredToken> {
    crate::core::auth::token::load_token()
        .ok()
        .flatten()
        .filter(|t| !t.is_expired())
}

pub(super) fn parse_github_url(url: &str) -> Result<(String, String)> {
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

    anyhow::bail!("Could not parse GitHub URL: {}", url)
}
