//! Escape-hatch reader/writer for arbitrary single-capture-group
//! regex patterns plus a `{version}`-substituting replace template.
//!
//! The resolver validates "exactly one capture group" at config-load
//! time, so by the time we get here the pattern is well-formed.

use std::fs;
use std::path::Path;

use regex::Regex;

use super::{Result, VersionFieldError};

fn compile(pattern: &str) -> Result<Regex> {
    Regex::new(pattern).map_err(|e| VersionFieldError::RegexCompile {
        pattern: pattern.to_string(),
        source: e,
    })
}

pub fn read(path: &Path, pattern: &str) -> Result<String> {
    let content = fs::read_to_string(path).map_err(|e| VersionFieldError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    let re = compile(pattern)?;
    let caps = re
        .captures(&content)
        .ok_or_else(|| VersionFieldError::VersionFieldMissing {
            path: path.display().to_string(),
            looked_for: "regex pattern (custom)",
        })?;
    Ok(caps.get(1).unwrap().as_str().to_string())
}

pub fn write(path: &Path, pattern: &str, replace_template: &str, new_version: &str) -> Result<()> {
    let content = fs::read_to_string(path).map_err(|e| VersionFieldError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    let re = compile(pattern)?;

    let caps = re
        .captures(&content)
        .ok_or_else(|| VersionFieldError::VersionFieldMissing {
            path: path.display().to_string(),
            looked_for: "regex pattern (custom)",
        })?;
    let current = caps.get(1).unwrap().as_str();
    if current == new_version {
        return Ok(()); // idempotent
    }

    let replacement = replace_template.replace("{version}", new_version);
    let new_content = re.replace(&content, replacement.as_str()).into_owned();

    fs::write(path, new_content).map_err(|e| VersionFieldError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn write_temp(content: &str) -> NamedTempFile {
        let f = NamedTempFile::new().unwrap();
        std::fs::write(f.path(), content).unwrap();
        f
    }

    // Patterns that anchor with `^...$` need the `(?m)` flag in
    // Rust's `regex` crate so they treat line boundaries as match
    // boundaries. The user's regex is taken verbatim — we don't
    // rewrite it — so tests use the same flag when needed.

    #[test]
    fn reads_via_capture_group() {
        let f = write_temp("VERSION=1.2.3\n");
        let r = read(f.path(), r"(?m)^VERSION=(\d+\.\d+\.\d+)$").unwrap();
        assert_eq!(r, "1.2.3");
    }

    #[test]
    fn writes_idempotent() {
        let f = write_temp("VERSION=1.0.0\n");
        write(
            f.path(),
            r"(?m)^VERSION=(\d+\.\d+\.\d+)$",
            "VERSION={version}",
            "1.1.0",
        )
        .unwrap();
        let after = std::fs::read_to_string(f.path()).unwrap();
        assert_eq!(after, "VERSION=1.1.0\n");

        // Second write at same value: no-op
        write(
            f.path(),
            r"(?m)^VERSION=(\d+\.\d+\.\d+)$",
            "VERSION={version}",
            "1.1.0",
        )
        .unwrap();
        let after2 = std::fs::read_to_string(f.path()).unwrap();
        assert_eq!(after, after2);
    }

    #[test]
    fn read_no_match_errors() {
        let f = write_temp("nothing useful here\n");
        assert!(matches!(
            read(f.path(), r"(?m)^VERSION=(.+)$").unwrap_err(),
            VersionFieldError::VersionFieldMissing { .. }
        ));
    }

    #[test]
    fn replace_template_substitutes_version_placeholder() {
        let f = write_temp("# pinned\nversion: 1.0.0\nother: stuff\n");
        write(
            f.path(),
            r"(?m)^version: (.+)$",
            "version: {version}",
            "2.0.0",
        )
        .unwrap();
        let after = std::fs::read_to_string(f.path()).unwrap();
        assert!(after.contains("version: 2.0.0"));
        assert!(after.contains("# pinned"));
        assert!(after.contains("other: stuff"));
    }
}
