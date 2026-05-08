//! `pyproject.toml` PEP 621 `[project].version` reader/writer using
//! `toml_edit` so comments + key ordering survive the round-trip.
//!
//! Mirrors `cargo_toml.rs`. The parallel auto-detect path in
//! `core::ecosystem::pypa::PyProjectVersionRewriter` uses the same
//! crate; this module exposes the read/write as a stand-alone
//! `VersionFieldSpec::Pep621` handler so explicit `[release_unit]`
//! blocks (and partial overrides) can target pyproject.toml without
//! resorting to `generic_regex`.

use std::fs;
use std::path::Path;

use toml_edit::DocumentMut;

use super::{Result, VersionFieldError};

const KIND: &str = "TOML";
const LOOKED_FOR: &str = "[project].version";

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

pub fn read(path: &Path) -> Result<String> {
    let doc = parse_doc(path)?;
    if let Some(v) = doc
        .get("project")
        .and_then(|p| p.as_table())
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
    {
        return Ok(v.to_string());
    }
    Err(VersionFieldError::VersionFieldMissing {
        path: path.display().to_string(),
        looked_for: LOOKED_FOR,
    })
}

pub fn write(path: &Path, new_version: &str) -> Result<()> {
    let mut doc = parse_doc(path)?;

    let project_tbl = doc
        .get_mut("project")
        .and_then(|p| p.as_table_mut())
        .ok_or_else(|| VersionFieldError::VersionFieldMissing {
            path: path.display().to_string(),
            looked_for: LOOKED_FOR,
        })?;

    if let Some(existing) = project_tbl.get("version").and_then(|v| v.as_str()) {
        if existing == new_version {
            return Ok(());
        }
    } else {
        return Err(VersionFieldError::VersionFieldMissing {
            path: path.display().to_string(),
            looked_for: LOOKED_FOR,
        });
    }

    project_tbl["version"] = toml_edit::value(new_version);

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
    fn reads_project_version() {
        let f = write_temp("[project]\nname = \"x\"\nversion = \"1.2.3\"\n");
        assert_eq!(read(f.path()).unwrap(), "1.2.3");
    }

    #[test]
    fn read_missing_version_errors() {
        let f = write_temp("[project]\nname = \"x\"\n");
        let err = read(f.path()).unwrap_err();
        assert!(matches!(err, VersionFieldError::VersionFieldMissing { .. }));
    }

    #[test]
    fn read_missing_project_table_errors() {
        let f = write_temp("[build-system]\nrequires = []\n");
        let err = read(f.path()).unwrap_err();
        assert!(matches!(err, VersionFieldError::VersionFieldMissing { .. }));
    }

    #[test]
    fn write_updates_project_version_idempotent() {
        let f = write_temp("[project]\nname = \"x\"\nversion = \"1.0.0\"\n");
        write(f.path(), "1.1.0").unwrap();
        let after = std::fs::read_to_string(f.path()).unwrap();
        assert!(after.contains("version = \"1.1.0\""));
        assert!(after.contains("name = \"x\""), "name preserved");

        let mtime_before = std::fs::metadata(f.path()).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        write(f.path(), "1.1.0").unwrap();
        let mtime_after = std::fs::metadata(f.path()).unwrap().modified().unwrap();
        assert_eq!(
            mtime_before, mtime_after,
            "no-op write should not touch the file"
        );
    }

    #[test]
    fn write_preserves_comments_and_ordering() {
        let f = write_temp(
            "# project metadata\n[project]\nname = \"x\"\n# bump this on release\nversion = \"0.1.0\"\nauthors = []\n",
        );
        write(f.path(), "0.2.0").unwrap();
        let after = std::fs::read_to_string(f.path()).unwrap();
        assert!(after.contains("# project metadata"));
        assert!(after.contains("# bump this on release"));
        assert!(after.contains("version = \"0.2.0\""));
        let name_pos = after.find("name").unwrap();
        let version_pos = after.find("version").unwrap();
        let authors_pos = after.find("authors").unwrap();
        assert!(name_pos < version_pos && version_pos < authors_pos);
    }
}
