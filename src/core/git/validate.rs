use anyhow::{bail, Result};

pub fn validate_remote_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("remote name cannot be empty");
    }

    if name.len() > 255 {
        bail!("remote name cannot exceed 255 characters");
    }

    for (idx, ch) in name.chars().enumerate() {
        if ch.is_whitespace() {
            bail!("remote name cannot contain whitespace");
        }

        if ch.is_control() {
            bail!("remote name cannot contain control characters");
        }

        if "~^:?*[\\".contains(ch) {
            bail!("remote name cannot contain special characters: ~ ^ : ? * [ \\");
        }

        if idx == 0 && (ch == '.' || ch == '/') {
            bail!("remote name cannot start with '.' or '/'");
        }
    }

    if name.ends_with('/') {
        bail!("remote name cannot end with '/'");
    }

    if name.ends_with(".lock") {
        bail!("remote name cannot end with '.lock'");
    }

    if name.contains("..") {
        bail!("remote name cannot contain '..'");
    }

    if name.contains("@{") {
        bail!("remote name cannot contain '@{{'");
    }

    if name.contains("//") {
        bail!("remote name cannot contain consecutive slashes");
    }

    Ok(())
}

pub fn validate_branch_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("branch name cannot be empty");
    }

    if name.len() > 255 {
        bail!("branch name cannot exceed 255 characters");
    }

    for (idx, ch) in name.chars().enumerate() {
        if ch.is_whitespace() {
            bail!("branch name cannot contain whitespace");
        }

        if ch.is_control() {
            bail!("branch name cannot contain control characters");
        }

        if "~^:?*[\\".contains(ch) {
            bail!("branch name cannot contain special characters: ~ ^ : ? * [ \\");
        }

        if idx == 0 && (ch == '.' || ch == '/') {
            bail!("branch name cannot start with '.' or '/'");
        }
    }

    if name.ends_with('/') {
        bail!("branch name cannot end with '/'");
    }

    if name.ends_with(".lock") {
        bail!("branch name cannot end with '.lock'");
    }

    if name.contains("..") {
        bail!("branch name cannot contain '..'");
    }

    if name.contains("@{") {
        bail!("branch name cannot contain '@{{'");
    }

    if name.contains("//") {
        bail!("branch name cannot contain consecutive slashes");
    }

    Ok(())
}

pub fn validate_tag_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("tag name cannot be empty");
    }

    if name.len() > 255 {
        bail!("tag name cannot exceed 255 characters");
    }

    for (idx, ch) in name.chars().enumerate() {
        if ch.is_whitespace() {
            bail!("tag name cannot contain whitespace");
        }

        if ch.is_control() {
            bail!("tag name cannot contain control characters");
        }

        if "~^:?*[\\".contains(ch) {
            bail!("tag name cannot contain special characters: ~ ^ : ? * [ \\");
        }

        if idx == 0 && (ch == '.' || ch == '/') {
            bail!("tag name cannot start with '.' or '/'");
        }
    }

    if name.ends_with('/') {
        bail!("tag name cannot end with '/'");
    }

    if name.ends_with(".lock") {
        bail!("tag name cannot end with '.lock'");
    }

    if name.contains("..") {
        bail!("tag name cannot contain '..'");
    }

    if name.contains("@{") {
        bail!("tag name cannot contain '@{{'");
    }

    if name.contains("//") {
        bail!("tag name cannot contain consecutive slashes");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_remote_name() {
        assert!(validate_remote_name("origin").is_ok());
        assert!(validate_remote_name("upstream").is_ok());
        assert!(validate_remote_name("my-remote").is_ok());

        assert!(validate_remote_name("").is_err());
        assert!(validate_remote_name("remote with spaces").is_err());
        assert!(validate_remote_name("../etc/passwd").is_err());
        assert!(validate_remote_name(".hidden").is_err());
        assert!(validate_remote_name("remote/").is_err());
        assert!(validate_remote_name("remote.lock").is_err());
        assert!(validate_remote_name("remote@{").is_err());
        assert!(validate_remote_name("remote//path").is_err());
    }

    #[test]
    fn test_validate_branch_name() {
        assert!(validate_branch_name("main").is_ok());
        assert!(validate_branch_name("feature/new-feature").is_ok());
        assert!(validate_branch_name("release-1.0").is_ok());

        assert!(validate_branch_name("").is_err());
        assert!(validate_branch_name("branch with spaces").is_err());
        assert!(validate_branch_name("../etc/passwd").is_err());
    }

    #[test]
    fn test_validate_tag_name() {
        assert!(validate_tag_name("v1.0.0").is_ok());
        assert!(validate_tag_name("release-2024-01-01").is_ok());

        assert!(validate_tag_name("").is_err());
        assert!(validate_tag_name("tag with spaces").is_err());
    }
}
