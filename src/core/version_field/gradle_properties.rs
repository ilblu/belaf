//! `gradle.properties` `^version=...` reader/writer.
//!
//! Behaviour by match count (per `BELAF_MASTER_PLAN.md` Part VI):
//! - 0 matches → edge case 18, hard error with hint "add version=0.1.0"
//! - 1 match → bump it
//! - N>1 matches → replace all (idempotent — all become same value),
//!   warn-log with line numbers (edge case 21)
//!
//! Preserves file ordering and comments — every line outside the
//! matched ones is left byte-identical.

use std::fs;
use std::path::Path;

use regex::Regex;
use tracing::warn;

use super::{Result, VersionFieldError};

fn version_re() -> Result<Regex> {
    let pat = r"(?m)^version=(.+)$";
    Regex::new(pat).map_err(|e| VersionFieldError::RegexCompile {
        pattern: pat.to_string(),
        source: e,
    })
}

pub fn read(path: &Path) -> Result<String> {
    let content = fs::read_to_string(path).map_err(|e| VersionFieldError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    let re = version_re()?;
    let caps = re
        .captures(&content)
        .ok_or_else(|| VersionFieldError::VersionFieldMissing {
            path: path.display().to_string(),
            looked_for: r"^version=(.+)$",
        })?;
    let value = caps
        .get(1)
        .ok_or_else(|| VersionFieldError::VersionFieldMissing {
            path: path.display().to_string(),
            looked_for: r"^version=(.+)$ (capture group 1 absent)",
        })?;
    Ok(value.as_str().trim().to_string())
}

pub fn write(path: &Path, new_version: &str) -> Result<()> {
    let content = fs::read_to_string(path).map_err(|e| VersionFieldError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    let re = version_re()?;

    let matches: Vec<_> = re.find_iter(&content).collect();
    if matches.is_empty() {
        return Err(VersionFieldError::VersionFieldMissing {
            path: path.display().to_string(),
            looked_for: r"^version=(.+)$",
        });
    }

    if matches.len() > 1 {
        let line_numbers: Vec<usize> = matches
            .iter()
            .map(|m| content[..m.start()].matches('\n').count() + 1)
            .collect();
        warn!(
            "{}: gradle.properties has {} `version=` lines (lines {:?}); replacing all",
            path.display(),
            matches.len(),
            line_numbers
        );
    }

    // Idempotent — if every match's captured value already equals
    // new_version, no-op. A capture without group 1 means the regex
    // matched but didn't capture, which contradicts `version_re()`'s
    // shape — treat that as "needs rewrite" rather than panicking.
    let all_idempotent = re.captures_iter(&content).all(|c| {
        c.get(1)
            .map(|m| m.as_str().trim() == new_version)
            .unwrap_or(false)
    });
    if all_idempotent {
        return Ok(());
    }

    let new_content = re
        .replace_all(&content, format!("version={new_version}").as_str())
        .into_owned();

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

    #[test]
    fn reads_version() {
        let f = write_temp("group=com.example\nversion=1.2.3\nname=foo\n");
        assert_eq!(read(f.path()).unwrap(), "1.2.3");
    }

    #[test]
    fn read_no_version_errors() {
        let f = write_temp("group=com.example\nname=foo\n");
        assert!(matches!(
            read(f.path()).unwrap_err(),
            VersionFieldError::VersionFieldMissing { .. }
        ));
    }

    #[test]
    fn write_updates_idempotent() {
        let f = write_temp("group=com.example\nversion=1.0.0\nname=foo\n");
        write(f.path(), "1.1.0").unwrap();
        let after = std::fs::read_to_string(f.path()).unwrap();
        assert_eq!(after, "group=com.example\nversion=1.1.0\nname=foo\n");

        // Second write at same value: no-op
        write(f.path(), "1.1.0").unwrap();
        let after2 = std::fs::read_to_string(f.path()).unwrap();
        assert_eq!(after, after2);
    }

    #[test]
    fn write_preserves_comments_and_ordering() {
        let f = write_temp(
            "# Top comment\ngroup=com.example\n# Why this version\nversion=1.0.0\nname=foo\n",
        );
        write(f.path(), "2.0.0").unwrap();
        let after = std::fs::read_to_string(f.path()).unwrap();
        assert!(after.contains("# Top comment\n"));
        assert!(after.contains("# Why this version\n"));
        assert!(after.contains("version=2.0.0\n"));
        assert!(after.contains("group=com.example\n"));
        assert!(after.contains("name=foo\n"));
    }

    #[test]
    fn write_multi_match_replaces_all() {
        // Edge case 21
        let f = write_temp("version=0.1.0\nname=foo\nversion=0.0.5\n");
        write(f.path(), "1.0.0").unwrap();
        let after = std::fs::read_to_string(f.path()).unwrap();
        // Both replaced
        assert_eq!(after.matches("version=1.0.0").count(), 2);
        assert_eq!(after.matches("version=0.").count(), 0);
    }

    #[test]
    fn write_no_version_line_errors() {
        let f = write_temp("group=com.example\nname=foo\n");
        assert!(matches!(
            write(f.path(), "1.0.0").unwrap_err(),
            VersionFieldError::VersionFieldMissing { .. }
        ));
    }
}
