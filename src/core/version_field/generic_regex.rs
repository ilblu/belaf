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

fn read_file(path: &Path) -> Result<String> {
    fs::read_to_string(path).map_err(|e| VersionFieldError::Io {
        path: path.display().to_string(),
        source: e,
    })
}

/// Capture group 1 of the first match, or of every match in file order
/// when `all` is set. A file that states its version more than once (an
/// Xcode project writes `MARKETING_VERSION` once per build configuration)
/// has one entry per statement.
fn captures(path: &Path, content: &str, re: &Regex, all: bool) -> Result<Vec<String>> {
    let values: Vec<String> = re
        .captures_iter(content)
        .take(if all { usize::MAX } else { 1 })
        .filter_map(|caps| caps.get(1).map(|m| m.as_str().to_string()))
        .collect();
    if values.is_empty() {
        return Err(VersionFieldError::VersionFieldMissing {
            path: path.display().to_string(),
            looked_for: "regex pattern (custom)",
        });
    }
    Ok(values)
}

/// The version the file states. With `all`, every match must state the
/// same one: a file that says two different things has no single current
/// version, and picking the first would hide the drift.
pub fn read(path: &Path, pattern: &str, all: bool) -> Result<String> {
    let content = read_file(path)?;
    let re = compile(pattern)?;
    let mut values = captures(path, &content, &re, all)?;
    values.sort();
    values.dedup();
    if values.len() > 1 {
        return Err(VersionFieldError::InconsistentVersions {
            path: path.display().to_string(),
            values: values.join(", "),
        });
    }
    Ok(values.remove(0))
}

/// Replace the first match, or every match when `all` is set, with
/// `replace_template`, `{version}` substituted. A file already at
/// `new_version` is left untouched.
pub fn write(
    path: &Path,
    pattern: &str,
    replace_template: &str,
    all: bool,
    new_version: &str,
) -> Result<()> {
    let content = read_file(path)?;
    let re = compile(pattern)?;

    let values = captures(path, &content, &re, all)?;
    if values.iter().all(|v| v == new_version) {
        return Ok(());
    }

    let replacement = replace_template.replace("{version}", new_version);
    let new_content = if all {
        re.replace_all(&content, replacement.as_str()).into_owned()
    } else {
        re.replace(&content, replacement.as_str()).into_owned()
    };

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
        let r = read(f.path(), r"(?m)^VERSION=(\d+\.\d+\.\d+)$", false).unwrap();
        assert_eq!(r, "1.2.3");
    }

    #[test]
    fn writes_idempotent() {
        let f = write_temp("VERSION=1.0.0\n");
        write(
            f.path(),
            r"(?m)^VERSION=(\d+\.\d+\.\d+)$",
            "VERSION={version}",
            false,
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
            false,
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
            read(f.path(), r"(?m)^VERSION=(.+)$", false).unwrap_err(),
            VersionFieldError::VersionFieldMissing { .. }
        ));
    }

    const PBXPROJ: &str = "\t\t\t\tMARKETING_VERSION = 0.3.1;\n\t\t\t\tPRODUCT_NAME = clikd;\n\t\t\t\tMARKETING_VERSION = 0.3.1;\n\t\t\t\tMARKETING_VERSION = 0.3.1;\n";
    const PBX_PATTERN: &str = r"MARKETING_VERSION = ([0-9][^;]*);";
    const PBX_REPLACE: &str = "MARKETING_VERSION = {version};";

    #[test]
    fn writes_every_match() {
        let f = write_temp(PBXPROJ);
        write(f.path(), PBX_PATTERN, PBX_REPLACE, true, "0.4.0").unwrap();
        let after = std::fs::read_to_string(f.path()).unwrap();
        assert_eq!(after.matches("MARKETING_VERSION = 0.4.0;").count(), 3);
        assert!(!after.contains("0.3.1"));
        assert!(after.contains("PRODUCT_NAME = clikd;"));
        assert_eq!(read(f.path(), PBX_PATTERN, true).unwrap(), "0.4.0");
    }

    #[test]
    fn read_rejects_matches_that_disagree() {
        let f = write_temp(&PBXPROJ.replacen("0.3.1", "0.2.0", 1));
        assert!(matches!(
            read(f.path(), PBX_PATTERN, true).unwrap_err(),
            VersionFieldError::InconsistentVersions { .. }
        ));
    }

    #[test]
    fn write_heals_matches_that_disagree() {
        let f = write_temp(&PBXPROJ.replacen("0.3.1", "0.4.0", 1));
        write(f.path(), PBX_PATTERN, PBX_REPLACE, true, "0.4.0").unwrap();
        let after = std::fs::read_to_string(f.path()).unwrap();
        assert_eq!(after.matches("MARKETING_VERSION = 0.4.0;").count(), 3);
    }

    #[test]
    fn first_match_only_by_default() {
        let f = write_temp(PBXPROJ);
        write(f.path(), PBX_PATTERN, PBX_REPLACE, false, "0.4.0").unwrap();
        let after = std::fs::read_to_string(f.path()).unwrap();
        assert_eq!(after.matches("MARKETING_VERSION = 0.4.0;").count(), 1);
        assert_eq!(after.matches("MARKETING_VERSION = 0.3.1;").count(), 2);
        assert_eq!(read(f.path(), PBX_PATTERN, false).unwrap(), "0.4.0");
    }

    #[test]
    fn replace_template_substitutes_version_placeholder() {
        let f = write_temp("# pinned\nversion: 1.0.0\nother: stuff\n");
        write(
            f.path(),
            r"(?m)^version: (.+)$",
            "version: {version}",
            false,
            "2.0.0",
        )
        .unwrap();
        let after = std::fs::read_to_string(f.path()).unwrap();
        assert!(after.contains("version: 2.0.0"));
        assert!(after.contains("# pinned"));
        assert!(after.contains("other: stuff"));
    }
}
