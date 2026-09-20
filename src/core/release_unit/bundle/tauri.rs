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

use crate::core::ecosystem::cargo::workspace::{
    enclosing_workspace_root, is_single_version_workspace,
};

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
        let shared_workspace = shared_cargo_workspace(workdir, &triplet_root);
        let note = tauri_note(single_source, shared_workspace.as_deref());
        out.push(DetectorMatch {
            shape: DetectedShape::Bundle(BundleKind::Tauri {
                single_source,
                shared_workspace,
            }),
            path: repopath,
            note: Some(note),
        });
    }
    out
}

fn tauri_note(single_source: bool, shared_workspace: Option<&str>) -> String {
    let base = if single_source {
        "single-source (version derived from package.json)"
    } else {
        "legacy multi-file (3 files hand-managed)"
    };
    match shared_workspace {
        Some(ws) => {
            let where_ = if ws.is_empty() { "repo root" } else { ws };
            format!("{base}; src-tauri shares the version of the cargo workspace at {where_}")
        }
        None => base.to_string(),
    }
}

/// The cargo workspace that owns `<app>/src-tauri`, if that workspace
/// versions all its members together.
///
/// Returns the workspace root as a repo-relative directory (empty string for
/// the repo root itself). `None` covers both "standalone crate" and "member of
/// a workspace whose crates version independently" — in the second case the
/// app's crate carries its own version and the Tauri bundle really is separate
/// from whatever else the workspace releases.
fn shared_cargo_workspace(workdir: &Path, triplet_root: &Path) -> Option<String> {
    let member_manifest = triplet_root.join("src-tauri/Cargo.toml");
    let ws_root = enclosing_workspace_root(&member_manifest, workdir)?;
    if !is_single_version_workspace(&ws_root.join("Cargo.toml")) {
        return None;
    }
    let rel = ws_root.strip_prefix(workdir).ok()?;
    Some(rel.to_string_lossy().replace('\\', "/"))
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
    let DetectedShape::Bundle(BundleKind::Tauri {
        single_source,
        ref shared_workspace,
    }) = m.shape
    else {
        return;
    };
    let path = m.path.escaped();
    let name_raw = path.rsplit('/').next().unwrap_or("desktop");

    // The app directory always belongs to the unit. When the app's Rust crate
    // is a member of a workspace that versions everything together, the
    // workspace root belongs to it too — not as a satellite, which is
    // prefix-matched and at the repo root would swallow every other unit in
    // the tree, but through its manifest, which is claimed by exact path.
    let satellites_q = toml_quote(&path);

    let mut manifests: Vec<String> = Vec::new();
    let mut workspace_manifest: Option<String> = None;
    let mut preamble = String::new();

    if let Some(ws) = shared_workspace {
        let ws_manifest = if ws.is_empty() {
            "Cargo.toml".to_string()
        } else {
            format!("{ws}/Cargo.toml")
        };
        let where_ = if ws.is_empty() { "the repo root" } else { ws };
        preamble.push_str(&format!(
            "\n# `{path}/src-tauri` is a member of the cargo workspace at {where_}, and\n\
             # every member there inherits `[workspace.package].version`. The workspace\n\
             # and this app are therefore one release: the crate reads its version from\n\
             # that one key. Listing the root manifest here is what tells the cargo\n\
             # loader the workspace is already spoken for — without it the same version\n\
             # is discovered twice and the repo grows a `cargo:{name_raw}` beside this\n\
             # `tauri:{name_raw}`.\n"
        ));
        // Deliberately *not* pushed first. The resolver anchors a bundle at
        // the directory of its first manifest, and a manifest at the repo
        // root anchors at "" — which it rejects outright as a degenerate
        // config. The app directory is the honest anchor anyway; the
        // workspace manifest is a second place the same number is written.
        workspace_manifest = Some(format!(
            "{{ path = {}, version_field = \"cargo_toml\" }}",
            toml_quote(&ws_manifest)
        ));
    }

    if single_source {
        counters.tauri_single_source += 1;
        manifests.push(format!(
            "{{ path = {}, version_field = \"npm_package_json\" }}",
            toml_quote(&format!("{path}/package.json"))
        ));
    } else {
        counters.tauri_legacy += 1;
        if preamble.is_empty() {
            preamble.push_str("\n# Tauri legacy multi-file (3 manifests in lockstep)\n");
        }
        manifests.push(format!(
            "{{ path = {}, version_field = \"npm_package_json\" }}",
            toml_quote(&format!("{path}/package.json"))
        ));
        // A shared workspace already contributes the root `Cargo.toml`; the
        // crate's own manifest only inherits from it and has no version to
        // write.
        if shared_workspace.is_none() {
            manifests.push(format!(
                "{{ path = {}, version_field = \"cargo_toml\" }}",
                toml_quote(&format!("{path}/src-tauri/Cargo.toml"))
            ));
        }
        manifests.push(format!(
            "{{ path = {}, version_field = \"tauri_conf_json\" }}",
            toml_quote(&format!("{path}/src-tauri/tauri.conf.json"))
        ));
    }

    if let Some(ws) = workspace_manifest {
        manifests.push(ws);
    }

    if preamble.is_empty() {
        preamble.push('\n');
    }

    snippet.push_str(&preamble);
    snippet.push_str(&format!(
        "[release_unit.{name_raw}]\necosystem = \"tauri\"\nsatellites = [{satellites_q}]\n"
    ));
    if manifests.len() == 1 {
        snippet.push_str(&format!("manifests = [{}]\n", manifests[0]));
    } else {
        snippet.push_str("manifests = [\n");
        for m in &manifests {
            snippet.push_str(&format!("  {m},\n"));
        }
        snippet.push_str("]\n");
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
            DetectedShape::Bundle(BundleKind::Tauri {
                single_source,
                shared_workspace: None,
            }) => assert!(*single_source),
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
            DetectedShape::Bundle(BundleKind::Tauri {
                single_source,
                shared_workspace: None,
            }) => assert!(!*single_source),
            _ => panic!("expected Tauri bundle"),
        }
    }

    /// Everything the rakete repo has: a Tauri app under `apps/`, its
    /// `src-tauri` crate listed as a workspace member at the repo root, and
    /// every member inheriting one `[workspace.package].version`.
    fn shared_workspace_repo(root: &Path) {
        write(
            &root.join("Cargo.toml"),
            r#"
[workspace]
resolver = "2"
members = ["apps/rakete/src-tauri", "crates/*"]

[workspace.package]
version = "0.1.0"
publish = false
"#,
        );
        for member in ["apps/rakete/src-tauri", "crates/core", "crates/pdf"] {
            write(
                &root.join(member).join("Cargo.toml"),
                "[package]\nname = \"x\"\nversion.workspace = true\n",
            );
        }
        write(
            &root.join("apps/rakete/package.json"),
            r#"{"name":"@rakete/app","version":"0.1.0"}"#,
        );
        write(
            &root.join("apps/rakete/src-tauri/tauri.conf.json"),
            r#"{"productName":"rakete","version":"../package.json"}"#,
        );
    }

    fn emit_for(matches: &[DetectorMatch]) -> String {
        let refs: Vec<&DetectorMatch> = matches.iter().collect();
        let mut snippet = String::new();
        let mut counters = DetectionCounters::default();
        emit_all(&refs, &mut snippet, &mut counters);
        snippet
    }

    #[test]
    fn a_src_tauri_inside_a_shared_workspace_is_recognised() {
        let t = TempDir::new().unwrap();
        shared_workspace_repo(t.path());
        let matches = detect(t.path());
        assert_eq!(matches.len(), 1);
        match &matches[0].shape {
            DetectedShape::Bundle(BundleKind::Tauri {
                shared_workspace, ..
            }) => {
                // Empty string: the workspace is the repo root.
                assert_eq!(shared_workspace.as_deref(), Some(""));
            }
            other => panic!("expected Tauri bundle, got {other:?}"),
        }
    }

    /// The regression this whole change exists for. The emitted block has to
    /// name the root `Cargo.toml`, because that exact-path claim is the only
    /// thing that stops the cargo loader minting a second unit for the same
    /// version — `cargo:rakete` next to `tauri:rakete`.
    #[test]
    fn the_emitted_block_claims_the_workspace_manifest() {
        let t = TempDir::new().unwrap();
        shared_workspace_repo(t.path());
        let snippet = emit_for(&detect(t.path()));

        assert!(
            snippet.contains(r#"{ path = "Cargo.toml", version_field = "cargo_toml" }"#),
            "root manifest must be claimed:\n{snippet}"
        );
        assert!(
            snippet.contains(
                r#"{ path = "apps/rakete/package.json", version_field = "npm_package_json" }"#
            ),
            "app manifest must still be written:\n{snippet}"
        );
        // The workspace root must NOT become a satellite: satellites are
        // prefix-matched, and "" would hand the whole repo to this unit.
        assert!(
            snippet.contains(r#"satellites = ["apps/rakete"]"#),
            "satellites stay scoped to the app:\n{snippet}"
        );
        // Order matters: the resolver anchors the bundle at the dirname of
        // the *first* manifest and rejects an anchor of "". Putting the root
        // Cargo.toml first turns the whole block into a hard resolver error.
        let app_at = snippet
            .find("apps/rakete/package.json")
            .expect("app manifest present");
        let ws_at = snippet
            .find(r#"path = "Cargo.toml""#)
            .expect("workspace manifest present");
        assert!(
            app_at < ws_at,
            "the app manifest must anchor the bundle:\n{snippet}"
        );
    }

    /// A Tauri app whose crate is standalone keeps the old, narrower block.
    #[test]
    fn a_standalone_tauri_app_is_unchanged() {
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
        let snippet = emit_for(&detect(root));
        assert!(
            !snippet.contains("cargo_toml"),
            "no cargo claim:\n{snippet}"
        );
        assert!(snippet.contains(
            r#"manifests = [{ path = "apps/desktop/package.json", version_field = "npm_package_json" }]"#
        ), "{snippet}");
    }

    /// A workspace whose members pin their own versions releases them
    /// independently, so the Tauri app is genuinely separate from it and the
    /// block must not claim the root.
    #[test]
    fn a_workspace_with_independent_members_is_not_shared() {
        let t = TempDir::new().unwrap();
        shared_workspace_repo(t.path());
        write(
            &t.path().join("crates/pdf/Cargo.toml"),
            "[package]\nname = \"pdf\"\nversion = \"0.3.0\"\n",
        );
        let matches = detect(t.path());
        match &matches[0].shape {
            DetectedShape::Bundle(BundleKind::Tauri {
                shared_workspace, ..
            }) => assert_eq!(shared_workspace.as_deref(), None),
            other => panic!("expected Tauri bundle, got {other:?}"),
        }
    }

    /// Legacy multi-file inside a shared workspace: the crate's own manifest
    /// only inherits, so there is no version in it to write.
    #[test]
    fn legacy_in_a_shared_workspace_writes_the_root_not_the_member() {
        let t = TempDir::new().unwrap();
        shared_workspace_repo(t.path());
        write(
            &t.path().join("apps/rakete/src-tauri/tauri.conf.json"),
            r#"{"productName":"rakete","version":"0.1.0"}"#,
        );
        let snippet = emit_for(&detect(t.path()));
        assert!(
            snippet.contains(r#"{ path = "Cargo.toml", version_field = "cargo_toml" }"#),
            "{snippet}"
        );
        assert!(
            !snippet.contains("apps/rakete/src-tauri/Cargo.toml"),
            "an inheriting member manifest has no version to write:\n{snippet}"
        );
        assert!(
            snippet.contains("tauri_conf_json"),
            "the conf still carries its own inline version:\n{snippet}"
        );
    }
}
