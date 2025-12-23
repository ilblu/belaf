use anyhow::{bail, Result};
use git_url_parse::types::provider::GenericProvider;
use git_url_parse::GitUrl;

pub fn parse_github_url(url: &str) -> Result<(String, String)> {
    if let Ok(parsed) = GitUrl::parse(url) {
        if let Ok(provider) = parsed.provider_info::<GenericProvider>() {
            let owner = provider.owner();
            let repo = provider.repo();

            if !owner.is_empty() && !repo.is_empty() {
                return Ok((owner.to_string(), repo.to_string()));
            }
        }
    }

    parse_github_url_fallback(url)
}

fn parse_github_url_fallback(url: &str) -> Result<(String, String)> {
    if let Some(rest) = url.strip_prefix("git@github.com:") {
        let repo = rest.trim_end_matches(".git");
        let mut parts = repo.split('/');
        if let (Some(owner), Some(name)) = (parts.next(), parts.next()) {
            if !owner.is_empty() && !name.is_empty() && parts.next().is_none() {
                return Ok((owner.to_string(), name.to_string()));
            }
        }
    }

    if let Some(rest) = url.strip_prefix("https://github.com/") {
        let repo = rest.trim_end_matches(".git");
        let mut parts = repo.split('/');
        if let (Some(owner), Some(name)) = (parts.next(), parts.next()) {
            if !owner.is_empty() && !name.is_empty() {
                return Ok((owner.to_string(), name.to_string()));
            }
        }
    }

    if let Some(rest) = url.strip_prefix("http://github.com/") {
        let repo = rest.trim_end_matches(".git");
        let mut parts = repo.split('/');
        if let (Some(owner), Some(name)) = (parts.next(), parts.next()) {
            if !owner.is_empty() && !name.is_empty() {
                return Ok((owner.to_string(), name.to_string()));
            }
        }
    }

    bail!("Could not parse GitHub remote URL: {}", url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ssh_url() {
        let (owner, repo) = parse_github_url("git@github.com:owner/repo.git").unwrap();
        assert_eq!(owner, "owner");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn test_parse_ssh_url_without_git_suffix() {
        let (owner, repo) = parse_github_url("git@github.com:owner/repo").unwrap();
        assert_eq!(owner, "owner");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn test_parse_https_url() {
        let (owner, repo) = parse_github_url("https://github.com/owner/repo.git").unwrap();
        assert_eq!(owner, "owner");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn test_parse_https_url_without_git_suffix() {
        let (owner, repo) = parse_github_url("https://github.com/owner/repo").unwrap();
        assert_eq!(owner, "owner");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn test_parse_https_url_with_username() {
        let (owner, repo) = parse_github_url("https://user@github.com/owner/repo.git").unwrap();
        assert_eq!(owner, "owner");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn test_parse_invalid_url() {
        assert!(parse_github_url("not-a-url").is_err());
    }

    #[test]
    fn test_parse_empty_owner() {
        assert!(parse_github_url("git@github.com:/repo.git").is_err());
    }

    #[test]
    fn test_parse_empty_repo() {
        assert!(parse_github_url("git@github.com:owner/.git").is_err());
    }
}
