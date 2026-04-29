//! `tauri.conf.json` `$.version` reader/writer using a regex-based
//! field replace that's JSON5-tolerant. **Not** serde-roundtrip —
//! Tauri 2.0's config is JSON5 with optional comments + unquoted keys
//! that serde_json's roundtrip would drop.

use std::fs;
use std::path::Path;

use regex::Regex;

use super::{Result, VersionFieldError};

/// Matches `"version": "X.Y.Z"` with optional whitespace around the
/// colon. Accepts but does not require the leading `"version"` to be
/// quoted (JSON5 allows unquoted keys; we still emit them quoted on
/// write to be safe in both JSON5 and strict JSON contexts).
fn version_re() -> Result<Regex> {
    let pat = r#"("version"|version)\s*:\s*"([^"]*)""#;
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
            looked_for: r#""version": "..." (top-level)"#,
        })?;
    Ok(caps.get(2).unwrap().as_str().to_string())
}

pub fn write(path: &Path, new_version: &str) -> Result<()> {
    let content = fs::read_to_string(path).map_err(|e| VersionFieldError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    let re = version_re()?;
    let caps = re
        .captures(&content)
        .ok_or_else(|| VersionFieldError::VersionFieldMissing {
            path: path.display().to_string(),
            looked_for: r#""version": "..." (top-level)"#,
        })?;

    if caps.get(2).unwrap().as_str() == new_version {
        return Ok(()); // idempotent
    }

    let new_content = re
        .replace(&content, format!(r#""version": "{new_version}""#).as_str())
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
    fn reads_version_strict_json() {
        let f = write_temp(r#"{"productName": "x", "version": "1.2.3"}"#);
        assert_eq!(read(f.path()).unwrap(), "1.2.3");
    }

    #[test]
    fn reads_version_with_comments_json5() {
        let f = write_temp(
            r#"// header comment
{
    "productName": "x",
    // current release
    "version": "1.2.3"
}"#,
        );
        assert_eq!(read(f.path()).unwrap(), "1.2.3");
    }

    #[test]
    fn writes_preserves_comments() {
        let f = write_temp(
            r#"// header comment
{
    "productName": "x",
    // current release
    "version": "1.0.0"
}"#,
        );
        write(f.path(), "1.1.0").unwrap();
        let after = std::fs::read_to_string(f.path()).unwrap();
        assert!(after.contains("// header comment"));
        assert!(after.contains("// current release"));
        assert!(after.contains(r#""version": "1.1.0""#));
        assert!(after.contains(r#""productName": "x""#));
    }

    #[test]
    fn writes_idempotent() {
        let f = write_temp(r#"{"version": "1.0.0"}"#);
        write(f.path(), "1.0.0").unwrap();
        let after = std::fs::read_to_string(f.path()).unwrap();
        assert_eq!(after, r#"{"version": "1.0.0"}"#);
    }

    #[test]
    fn read_no_version_errors() {
        let f = write_temp(r#"{"productName": "x"}"#);
        assert!(matches!(
            read(f.path()).unwrap_err(),
            VersionFieldError::VersionFieldMissing { .. }
        ));
    }
}
