use crate::error::{CliError, Result};
use git2::Repository;
use std::env;
use std::fs;

pub fn init_current_branch() -> Result<()> {
    let cwd = env::current_dir().map_err(CliError::Io)?;
    let branch_file = cwd.join("belaf/.branches/_current_branch");

    if branch_file.exists() {
        return Ok(());
    }

    if let Some(parent) = branch_file.parent() {
        fs::create_dir_all(parent).map_err(CliError::Io)?;
    }

    fs::write(&branch_file, "main").map_err(CliError::Io)?;

    Ok(())
}

pub fn current() -> Result<String> {
    let repo = find_repository()?;

    let head = repo.head().map_err(CliError::Git)?;

    if head.is_branch() {
        let branch_name = head
            .shorthand()
            .ok_or_else(|| CliError::Git(git2::Error::from_str("Invalid branch name")))?;
        Ok(branch_name.to_string())
    } else {
        Err(CliError::Git(git2::Error::from_str(
            "HEAD is not pointing to a branch",
        )))
    }
}

pub fn sanitize(branch: &str) -> String {
    branch.replace(['/', '-', '.'], "_").to_lowercase()
}

fn find_repository() -> Result<Repository> {
    let current_dir = std::env::current_dir().map_err(CliError::Io)?;

    Repository::discover(&current_dir)
        .or_else(|_| Repository::open(&current_dir))
        .map_err(CliError::Git)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize() {
        assert_eq!(sanitize("feature/auth"), "feature_auth");
        assert_eq!(sanitize("fix/bug-123"), "fix_bug_123");
        assert_eq!(sanitize("main"), "main");
        assert_eq!(sanitize("feat/user.profile"), "feat_user_profile");
    }
}
