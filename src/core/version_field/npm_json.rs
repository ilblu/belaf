//! `package.json` `$.version` reader/writer using `serde_json` with
//! indent-detection so 2-space vs 4-space style survives the round
//! trip.

use std::fs;
use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use super::{Result, VersionFieldError};

const KIND: &str = "JSON";

fn parse_json(path: &Path) -> Result<(String, Value)> {
    let content = fs::read_to_string(path).map_err(|e| VersionFieldError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    let value: Value =
        serde_json::from_str(&content).map_err(|e| VersionFieldError::ParseError {
            path: path.display().to_string(),
            kind: KIND,
            reason: e.to_string(),
        })?;
    Ok((content, value))
}

/// Detects the leading indentation of the first non-trivial nested
/// line in the original content. Falls back to 2 spaces.
fn detect_indent(original: &str) -> String {
    for line in original.lines() {
        let leading: String = line.chars().take_while(|c| c.is_whitespace()).collect();
        if !leading.is_empty() && leading != "\t" {
            return leading;
        }
        if leading == "\t" {
            return "\t".to_string();
        }
    }
    "  ".to_string()
}

pub fn read(path: &Path) -> Result<String> {
    let (_, value) = parse_json(path)?;
    let v = value
        .get("version")
        .and_then(|v| v.as_str())
        .ok_or_else(|| VersionFieldError::VersionFieldMissing {
            path: path.display().to_string(),
            looked_for: "$.version",
        })?;
    Ok(v.to_string())
}

pub fn write(path: &Path, new_version: &str) -> Result<()> {
    let (original, mut value) = parse_json(path)?;

    // Idempotent — no-op if unchanged
    if value
        .get("version")
        .and_then(|v| v.as_str())
        .map(|s| s == new_version)
        .unwrap_or(false)
    {
        return Ok(());
    }

    let obj = value
        .as_object_mut()
        .ok_or_else(|| VersionFieldError::ParseError {
            path: path.display().to_string(),
            kind: KIND,
            reason: "package.json root is not an object".to_string(),
        })?;
    obj.insert(
        "version".to_string(),
        Value::String(new_version.to_string()),
    );

    let indent = detect_indent(&original);
    let buf = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes());
    let mut ser = serde_json::Serializer::with_formatter(buf, formatter);
    value
        .serialize(&mut ser)
        .map_err(|e| VersionFieldError::ParseError {
            path: path.display().to_string(),
            kind: KIND,
            reason: e.to_string(),
        })?;
    let mut out = String::from_utf8(ser.into_inner()).expect("serde_json output is utf-8");

    // Preserve a trailing newline if the original had one.
    if original.ends_with('\n') && !out.ends_with('\n') {
        out.push('\n');
    }

    fs::write(path, out).map_err(|e| VersionFieldError::Io {
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
        let f = write_temp(r#"{"name":"x","version":"1.2.3"}"#);
        assert_eq!(read(f.path()).unwrap(), "1.2.3");
    }

    #[test]
    fn read_missing_version_errors() {
        let f = write_temp(r#"{"name":"x"}"#);
        assert!(matches!(
            read(f.path()).unwrap_err(),
            VersionFieldError::VersionFieldMissing { .. }
        ));
    }

    #[test]
    fn write_updates_idempotent() {
        let f = write_temp("{\n  \"name\": \"x\",\n  \"version\": \"1.0.0\"\n}\n");
        write(f.path(), "1.1.0").unwrap();
        let after = std::fs::read_to_string(f.path()).unwrap();
        assert!(after.contains("\"version\": \"1.1.0\""));

        // Second write at same version: no-op
        let len_before = after.len();
        write(f.path(), "1.1.0").unwrap();
        let after2 = std::fs::read_to_string(f.path()).unwrap();
        assert_eq!(len_before, after2.len());
    }

    #[test]
    fn write_preserves_2space_indent() {
        let f = write_temp("{\n  \"name\": \"x\",\n  \"version\": \"1.0.0\"\n}\n");
        write(f.path(), "1.1.0").unwrap();
        let after = std::fs::read_to_string(f.path()).unwrap();
        // 2-space-indented version line should still be 2-spaced
        assert!(after.contains("  \"version\""));
    }

    #[test]
    fn write_preserves_4space_indent() {
        let f = write_temp("{\n    \"name\": \"x\",\n    \"version\": \"1.0.0\"\n}\n");
        write(f.path(), "1.1.0").unwrap();
        let after = std::fs::read_to_string(f.path()).unwrap();
        assert!(after.contains("    \"version\""));
    }
}
