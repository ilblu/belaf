use anyhow::{bail, Result};
use git_url_parse::types::provider::GenericProvider;
use git_url_parse::GitUrl;

pub fn parse_github_url(url: &str) -> Result<(String, String)> {
    if let Ok(parsed) = GitUrl::parse(url) {
        if let Ok(provider) = parsed.provider_info::<GenericProvider>() {
            let owner = provider.owner();
            let repo = provider.repo();

            if !owner.is_empty() && !repo.is_empty() {
                validate_path_component(owner, "owner")?;
                validate_path_component(repo, "repository name")?;
                return Ok((owner.to_string(), repo.to_string()));
            }
        }
    }

    parse_github_url_fallback(url)
}

fn validate_path_component(component: &str, name: &str) -> Result<()> {
    if component == "." || component == ".." {
        bail!("Invalid {}: path traversal not allowed", name);
    }

    if component.contains("..") {
        bail!("Invalid {}: path traversal sequence '..' not allowed", name);
    }

    if component.starts_with('.')
        && component.len() > 1
        && !component
            .chars()
            .nth(1)
            .map(|c| c.is_alphanumeric())
            .unwrap_or(false)
    {
        bail!("Invalid {}: suspicious path component", name);
    }

    if component.contains('\0') {
        bail!("Invalid {}: null bytes not allowed", name);
    }

    if component.contains('/') || component.contains('\\') {
        bail!("Invalid {}: path separators not allowed", name);
    }

    Ok(())
}

fn parse_github_url_fallback(url: &str) -> Result<(String, String)> {
    if let Some(rest) = url.strip_prefix("git@github.com:") {
        let repo = rest.trim_end_matches(".git");
        let mut parts = repo.split('/');
        if let (Some(owner), Some(name)) = (parts.next(), parts.next()) {
            if !owner.is_empty() && !name.is_empty() && parts.next().is_none() {
                validate_path_component(owner, "owner")?;
                validate_path_component(name, "repository name")?;
                return Ok((owner.to_string(), name.to_string()));
            }
        }
    }

    if let Some(rest) = url.strip_prefix("https://github.com/") {
        let repo = rest.trim_end_matches(".git");
        let mut parts = repo.split('/');
        if let (Some(owner), Some(name)) = (parts.next(), parts.next()) {
            if !owner.is_empty() && !name.is_empty() {
                validate_path_component(owner, "owner")?;
                validate_path_component(name, "repository name")?;
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

    #[test]
    fn test_path_traversal_dotdot_rejected() {
        assert!(parse_github_url("https://github.com/../repo.git").is_err());
        assert!(parse_github_url("https://github.com/owner/..").is_err());
        assert!(parse_github_url("git@github.com:../repo.git").is_err());
    }

    #[test]
    fn test_path_traversal_dot_rejected() {
        assert!(parse_github_url("https://github.com/./repo.git").is_err());
        assert!(parse_github_url("https://github.com/owner/.").is_err());
    }

    #[test]
    fn test_null_byte_rejected() {
        assert!(parse_github_url("https://github.com/owner\0/repo.git").is_err());
        assert!(parse_github_url("https://github.com/owner/repo\0.git").is_err());
    }

    #[test]
    fn test_valid_dotfile_repo_accepted() {
        let result = parse_github_url("https://github.com/owner/.dotfiles.git");
        assert!(result.is_ok());
        let (owner, repo) = result.unwrap();
        assert_eq!(owner, "owner");
        assert_eq!(repo, ".dotfiles");
    }
}
