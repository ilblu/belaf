//! `Cargo.toml` `[package].version` (or `[workspace.package].version`)
//! reader/writer using `toml_edit` so comments + key ordering survive
//! the round-trip.

use std::fs;
use std::path::Path;

use toml_edit::DocumentMut;

use super::{Result, VersionFieldError};

const KIND: &str = "TOML";

fn parse_doc(path: &Path) -> Result<DocumentMut> {
    let content = fs::read_to_string(path).map_err(|e| VersionFieldError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    content
        .parse::<DocumentMut>()
        .map_err(|e| VersionFieldError::ParseError {
            path: path.display().to_string(),
            kind: KIND,
            reason: e.to_string(),
        })
}

/// Extract the current `[package].version` if present, else
/// `[workspace.package].version`. If both are missing, returns
/// `VersionFieldMissing`.
pub fn read(path: &Path) -> Result<String> {
    let doc = parse_doc(path)?;
    if let Some(v) = doc
        .get("package")
        .and_then(|p| p.as_table())
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
    {
        return Ok(v.to_string());
    }
    if let Some(v) = doc
        .get("workspace")
        .and_then(|w| w.as_table())
        .and_then(|w| w.get("package"))
        .and_then(|p| p.as_table())
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
    {
        return Ok(v.to_string());
    }
    Err(VersionFieldError::VersionFieldMissing {
        path: path.display().to_string(),
        looked_for: "[package].version or [workspace.package].version",
    })
}

/// Write `new_version` to whichever field is present. Prefers
/// `[package].version` if both exist (the common case for a normal
/// crate), then falls back to `[workspace.package].version` for
/// virtual workspace roots. Idempotent.
pub fn write(path: &Path, new_version: &str) -> Result<()> {
    let mut doc = parse_doc(path)?;

    let mut wrote = false;
    if let Some(pkg_tbl) = doc.get_mut("package").and_then(|i| i.as_table_mut()) {
        if let Some(existing) = pkg_tbl.get("version").and_then(|v| v.as_str()) {
            if existing == new_version {
                return Ok(()); // idempotent — no-op
            }
        }
        pkg_tbl["version"] = toml_edit::value(new_version);
        wrote = true;
    }

    if !wrote {
        if let Some(ws_pkg_tbl) = doc
            .get_mut("workspace")
            .and_then(|w| w.as_table_mut())
            .and_then(|w| w.get_mut("package"))
            .and_then(|p| p.as_table_mut())
        {
            if let Some(existing) = ws_pkg_tbl.get("version").and_then(|v| v.as_str()) {
                if existing == new_version {
                    return Ok(());
                }
            }
            ws_pkg_tbl["version"] = toml_edit::value(new_version);
            wrote = true;
        }
    }

    if !wrote {
        return Err(VersionFieldError::VersionFieldMissing {
            path: path.display().to_string(),
            looked_for: "[package].version or [workspace.package].version",
        });
    }

    fs::write(path, doc.to_string()).map_err(|e| VersionFieldError::Io {
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
    fn reads_package_version() {
        let f = write_temp("[package]\nname = \"x\"\nversion = \"1.2.3\"\n");
        assert_eq!(read(f.path()).unwrap(), "1.2.3");
    }

    #[test]
    fn reads_workspace_package_version_fallback() {
        let f = write_temp("[workspace]\n\n[workspace.package]\nversion = \"0.5.0\"\n");
        assert_eq!(read(f.path()).unwrap(), "0.5.0");
    }

    #[test]
    fn read_missing_version_errors() {
        let f = write_temp("[package]\nname = \"x\"\n");
        let err = read(f.path()).unwrap_err();
        assert!(matches!(err, VersionFieldError::VersionFieldMissing { .. }));
    }

    #[test]
    fn write_updates_package_version_idempotent() {
        let f = write_temp("[package]\nname = \"x\"\nversion = \"1.0.0\"\n");
        write(f.path(), "1.1.0").unwrap();
        let after = std::fs::read_to_string(f.path()).unwrap();
        assert!(after.contains("version = \"1.1.0\""));
        assert!(after.contains("name = \"x\""), "name preserved");

        // Second write at same version — no change
        write(f.path(), "1.1.0").unwrap();
        let after2 = std::fs::read_to_string(f.path()).unwrap();
        assert_eq!(after, after2);
    }

    #[test]
    fn write_preserves_comments_and_ordering() {
        let original =
            "# top comment\n[package]\nname = \"x\"\nversion = \"1.0.0\"\nedition = \"2021\"\n";
        let f = write_temp(original);
        write(f.path(), "1.1.0").unwrap();
        let after = std::fs::read_to_string(f.path()).unwrap();
        assert!(after.contains("# top comment"));
        assert!(after.contains("edition = \"2021\""));
        // version field updated in place
        assert!(after.contains("version = \"1.1.0\""));
    }

    #[test]
    fn write_to_workspace_package_when_no_package_section() {
        let f =
            write_temp("[workspace]\nmembers = []\n\n[workspace.package]\nversion = \"0.0.0\"\n");
        write(f.path(), "0.1.0").unwrap();
        let after = std::fs::read_to_string(f.path()).unwrap();
        assert!(after.contains("version = \"0.1.0\""));
    }
}
