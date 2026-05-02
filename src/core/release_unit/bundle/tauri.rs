//! Tauri bundle — `package.json` + `src-tauri/Cargo.toml` +
//! `src-tauri/tauri.conf.json` triplet.
//!
//! Two emission shapes:
//!
//! - **single-source**: `tauri.conf.json` references the version in a
//!   sibling JSON via `"version": "../package.json"` (or simply omits
//!   the version). One manifest in the release_unit, the rest follow.
//! - **legacy multi-file**: all three files carry an inline version
//!   that must be bumped in lockstep. Three manifests in the
//!   release_unit, kept in sync by the rewriter.

use std::path::Path;
use std::sync::LazyLock;

use super::super::shape::{BundleKind, DetectedShape, DetectorMatch};
use super::super::walk::{find_dirs_with_files_set, relative_repopath};

use crate::cmd::init::auto_detect::DetectionCounters;
use crate::cmd::init::toml_util::toml_quote;

pub fn detect(workdir: &Path) -> Vec<DetectorMatch> {
    let mut out = Vec::new();
    for triplet_root in find_dirs_with_files_set(
        workdir,
        &[
            "package.json",
            "src-tauri/Cargo.toml",
            "src-tauri/tauri.conf.json",
        ],
    ) {
        let conf_path = triplet_root.join("src-tauri/tauri.conf.json");
        let single_source = is_tauri_single_source(&conf_path);
        let repopath = match relative_repopath(workdir, &triplet_root) {
            Some(r) => r,
            None => continue,
        };
        out.push(DetectorMatch {
            shape: DetectedShape::Bundle(BundleKind::Tauri { single_source }),
            path: repopath,
            note: Some(if single_source {
                "single-source (version derived from package.json)".to_string()
            } else {
                "legacy multi-file (3 files hand-managed)".to_string()
            }),
        });
    }
    out
}

/// Emit blocks for every Tauri match in the slice. Filters out
/// non-Tauri matches; safe to call with an unfiltered slice (the
/// dispatch in `bundle::emit_all` passes only Bundle matches).
pub fn emit_all(
    matches: &[&DetectorMatch],
    snippet: &mut String,
    counters: &mut DetectionCounters,
) {
    for m in matches {
        if matches!(m.shape, DetectedShape::Bundle(BundleKind::Tauri { .. })) {
            emit_block(m, snippet, counters);
        }
    }
}

fn emit_block(m: &DetectorMatch, snippet: &mut String, counters: &mut DetectionCounters) {
    let DetectedShape::Bundle(BundleKind::Tauri { single_source }) = m.shape else {
        return;
    };
    let path = m.path.escaped();
    let name_raw = path.rsplit('/').next().unwrap_or("desktop");
    let name_q = toml_quote(name_raw);
    let satellites_q = toml_quote(&path);
    if single_source {
        counters.tauri_single_source += 1;
        let manifest_q = toml_quote(&format!("{path}/package.json"));
        snippet.push_str(&format!(
            "\n[[release_unit]]\nname = {name_q}\necosystem = \"tauri\"\nsatellites = [{satellites_q}]\n[[release_unit.source.manifests]]\npath = {manifest_q}\nversion_field = \"npm_package_json\"\n",
        ));
    } else {
        counters.tauri_legacy += 1;
        let pkg_q = toml_quote(&format!("{path}/package.json"));
        let cargo_q = toml_quote(&format!("{path}/src-tauri/Cargo.toml"));
        let conf_q = toml_quote(&format!("{path}/src-tauri/tauri.conf.json"));
        snippet.push_str(&format!(
            "\n# Tauri legacy multi-file (3 manifests in lockstep)\n[[release_unit]]\nname = {name_q}\necosystem = \"tauri\"\nsatellites = [{satellites_q}]\n[[release_unit.source.manifests]]\npath = {pkg_q}\nversion_field = \"npm_package_json\"\n[[release_unit.source.manifests]]\npath = {cargo_q}\nversion_field = \"cargo_toml\"\n[[release_unit.source.manifests]]\npath = {conf_q}\nversion_field = \"tauri_conf_json\"\n",
        ));
    }
}

static TAURI_PATH_REF_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r#""version"\s*:\s*"\.\./[^"]+\.json""#).expect("static regex must compile")
});
static TAURI_ANY_VERSION_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r#""version"\s*:\s*"[^"]+""#).expect("static regex must compile")
});

fn is_tauri_single_source(conf_path: &Path) -> bool {
    let content = match std::fs::read_to_string(conf_path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    if TAURI_PATH_REF_RE.is_match(&content) {
        return true;
    }
    !TAURI_ANY_VERSION_RE.is_match(&content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(p: &Path, content: &str) {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, content).unwrap();
    }

    #[test]
    fn single_source_via_path_ref() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("apps/desktop/package.json"),
            r#"{"version":"0.1.0"}"#,
        );
        write(
            &root.join("apps/desktop/src-tauri/Cargo.toml"),
            "[package]\nname = \"desktop\"\nversion = \"0.0.0\"\n",
        );
        write(
            &root.join("apps/desktop/src-tauri/tauri.conf.json"),
            r#"{"productName":"desktop","version":"../package.json"}"#,
        );
        let matches = detect(root);
        assert_eq!(matches.len(), 1);
        match &matches[0].shape {
            DetectedShape::Bundle(BundleKind::Tauri { single_source }) => assert!(*single_source),
            _ => panic!("expected Tauri bundle"),
        }
    }

    #[test]
    fn legacy_multi_file() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("apps/desktop/package.json"),
            r#"{"version":"0.1.0"}"#,
        );
        write(
            &root.join("apps/desktop/src-tauri/Cargo.toml"),
            "[package]\nname = \"desktop\"\nversion = \"0.1.0\"\n",
        );
        write(
            &root.join("apps/desktop/src-tauri/tauri.conf.json"),
            r#"{"productName":"desktop","version":"0.1.0"}"#,
        );
        let matches = detect(root);
        assert_eq!(matches.len(), 1);
        match &matches[0].shape {
            DetectedShape::Bundle(BundleKind::Tauri { single_source }) => assert!(!*single_source),
            _ => panic!("expected Tauri bundle"),
        }
    }
}
