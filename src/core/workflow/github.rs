//! GitHub remote helpers used by the release pipeline.
//!
//! Extracted from `core::workflow` to keep the orchestrator focused on the
//! release flow rather than GitHub plumbing. Re-exported through
//! `core::workflow::*` for compatibility with existing call sites.

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
