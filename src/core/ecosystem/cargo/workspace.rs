//! Reading the *shape* of a cargo workspace from its manifests alone.
//!
//! Split out of `cargo.rs` because two very different callers need the same
//! judgement and only one of them can afford `cargo metadata`:
//!
//! - the cargo loader, which already has a [`cargo_metadata::Metadata`] and
//!   wants to know whether a workspace releases as one unit or as many;
//! - the Tauri detector in `release_unit::bundle::tauri`, which runs during
//!   `belaf init --auto-detect` over a directory tree, has no resolved
//!   metadata, and must not shell out to cargo for every candidate it sees.
//!
//! Everything here is therefore pure manifest reading: `toml_edit` over a
//! path, no subprocess, no lockfile.

use std::path::{Path, PathBuf};

use toml_edit::DocumentMut;

use crate::utils::file_io::read_config_file;

/// Whether a member manifest takes its version from `[workspace.package]`
/// (`version.workspace = true`) rather than pinning its own.
///
/// A manifest that is unreadable, unparseable, or carries no `version` at all
/// counts as *not* inheriting: Cargo defaults such a package to `0.0.0`
/// independently of the workspace, so folding it into a shared release unit
/// would be a guess.
pub fn member_inherits_workspace_version(manifest_path: &Path) -> bool {
    let Ok(content) = read_config_file(manifest_path) else {
        return false;
    };
    let Ok(doc) = content.parse::<DocumentMut>() else {
        return false;
    };
    member_doc_inherits_workspace_version(&doc)
}

/// The [`DocumentMut`] half of [`member_inherits_workspace_version`], so a
/// caller that already parsed the manifest does not parse it twice.
pub fn member_doc_inherits_workspace_version(doc: &DocumentMut) -> bool {
    let Some(pkg) = doc.get("package").and_then(|v| v.as_table()) else {
        return false;
    };
    let Some(version) = pkg.get("version") else {
        return false;
    };
    // `version.workspace = true` parses as a (possibly inline) table with a
    // single `workspace` key; `version = "1.2.3"` parses as a plain value.
    version
        .as_table_like()
        .and_then(|t| t.get("workspace"))
        .and_then(|w| w.as_value())
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Whether a root manifest declares an inheritable `[workspace.package].version`.
///
/// On its own this says nothing about how many units the workspace releases —
/// see the note on [`is_single_version_workspace`].
pub fn declares_inheritable_version(root_doc: &DocumentMut) -> bool {
    root_doc
        .get("workspace")
        .and_then(|ws| ws.as_table())
        .and_then(|ws_table| ws_table.get("package"))
        .and_then(|pkg| pkg.as_table())
        .and_then(|pkg_table| pkg_table.get("version"))
        .is_some()
}

/// The literal member paths listed under `[workspace].members`.
///
/// Returned verbatim, including globs like `crates/*`. Cargo resolves those
/// against the workspace root; callers that only need to test one known member
/// should use [`workspace_claims_member`], which handles both forms.
pub fn declared_members(root_doc: &DocumentMut) -> Vec<String> {
    root_doc
        .get("workspace")
        .and_then(|ws| ws.as_table())
        .and_then(|ws_table| ws_table.get("members"))
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Whether `[workspace].members` claims `rel` (a member directory relative to
/// the workspace root, slash-separated, no trailing slash).
///
/// Handles the two forms cargo accepts: a literal path, and a single-level
/// `prefix/*` glob. `**` is accepted as a prefix match. `exclude` is honoured.
pub fn workspace_claims_member(root_doc: &DocumentMut, rel: &str) -> bool {
    let excluded = root_doc
        .get("workspace")
        .and_then(|ws| ws.as_table())
        .and_then(|ws_table| ws_table.get("exclude"))
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .any(|e| path_matches(e, rel))
        })
        .unwrap_or(false);
    if excluded {
        return false;
    }

    declared_members(root_doc)
        .iter()
        .any(|pattern| path_matches(pattern, rel))
}

fn path_matches(pattern: &str, rel: &str) -> bool {
    let pattern = pattern.trim_end_matches('/');
    if pattern == rel {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix("/**") {
        return rel.starts_with(&format!("{prefix}/"));
    }
    if let Some(prefix) = pattern.strip_suffix("/*") {
        // Single level only: `crates/*` claims `crates/core`, not
        // `crates/core/macros`.
        return rel
            .strip_prefix(&format!("{prefix}/"))
            .is_some_and(|rest| !rest.contains('/'));
    }
    false
}

/// A workspace that can only ever carry one version, judged from the manifests
/// on disk.
///
/// The root must declare `[workspace.package].version` **and** every listed
/// member must actually inherit it. The second half is the load-bearing one:
/// practically every modern workspace sets the root key, so reading it alone
/// as "this is one project" collapses ordinary multi-crate repos into a single
/// unit. If even one member pins its own version, the members release
/// independently and this returns `false`.
///
/// Glob members (`crates/*`) are resolved against the filesystem. A member
/// directory that is listed but absent is ignored rather than treated as a
/// non-inheriting member, so a stale entry does not silently flip the verdict.
pub fn is_single_version_workspace(root_manifest: &Path) -> bool {
    let Ok(content) = read_config_file(root_manifest) else {
        return false;
    };
    let Ok(doc) = content.parse::<DocumentMut>() else {
        return false;
    };
    if !declares_inheritable_version(&doc) {
        return false;
    }

    let Some(root_dir) = root_manifest.parent() else {
        return false;
    };

    let members = resolve_member_dirs(&doc, root_dir);
    if members.is_empty() {
        // Nothing to collapse into one unit.
        return false;
    }

    members
        .iter()
        .all(|dir| member_inherits_workspace_version(&dir.join("Cargo.toml")))
}

/// Expand `[workspace].members` to the member directories that exist on disk.
fn resolve_member_dirs(root_doc: &DocumentMut, root_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for pattern in declared_members(root_doc) {
        let pattern = pattern.trim_end_matches('/');
        if let Some(prefix) = pattern.strip_suffix("/*").or(pattern.strip_suffix("/**")) {
            let base = root_dir.join(prefix);
            let Ok(entries) = std::fs::read_dir(&base) else {
                continue;
            };
            let mut dirs: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.join("Cargo.toml").is_file())
                .collect();
            // `read_dir` order is the filesystem's; sort so a config diff is
            // the same on every machine.
            dirs.sort();
            out.extend(dirs);
        } else {
            let dir = root_dir.join(pattern);
            if dir.join("Cargo.toml").is_file() {
                out.push(dir);
            }
        }
    }
    out
}

/// Walk up from a member manifest looking for the workspace that owns it.
///
/// Stops at `ceiling` (the repo root) so the search never escapes the
/// repository into the user's home directory. Returns the directory holding
/// the owning root `Cargo.toml`, or `None` if the crate is standalone.
///
/// A crate is considered owned when the ancestor manifest has a `[workspace]`
/// table that claims it — either explicitly through `members`, or implicitly
/// because the crate sets `version.workspace = true` and therefore cannot
/// resolve without one.
pub fn enclosing_workspace_root(member_manifest: &Path, ceiling: &Path) -> Option<PathBuf> {
    let member_dir = member_manifest.parent()?;
    let inherits = member_inherits_workspace_version(member_manifest);

    let mut cursor = member_dir.parent();
    while let Some(dir) = cursor {
        let candidate = dir.join("Cargo.toml");
        if candidate.is_file() {
            let parsed = read_config_file(&candidate)
                .ok()
                .and_then(|content| content.parse::<DocumentMut>().ok())
                .filter(|doc| doc.get("workspace").is_some());
            if let Some(doc) = parsed {
                let rel = member_dir
                    .strip_prefix(dir)
                    .ok()
                    .and_then(|p| p.to_str())
                    .map(|s| s.replace('\\', "/"));
                if let Some(rel) = rel {
                    if workspace_claims_member(&doc, &rel) || inherits {
                        return Some(dir.to_path_buf());
                    }
                }
            }
        }

        if dir == ceiling {
            break;
        }
        cursor = dir.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write(dir: &Path, rel: &str, body: &str) {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().expect("BUG: path should have a parent"))
            .expect("BUG: should create dirs");
        fs::write(path, body).expect("BUG: should write file");
    }

    /// The shape belaf kept getting wrong: a Tauri app whose `src-tauri` crate
    /// is one member of a repo-root workspace that versions everything
    /// together.
    fn rakete_shaped(root: &Path) {
        write(
            root,
            "Cargo.toml",
            r#"
[workspace]
resolver = "2"
members = ["apps/rakete/src-tauri", "apps/server", "crates/*"]

[workspace.package]
version = "0.1.0"
publish = false
"#,
        );
        for member in [
            "apps/rakete/src-tauri",
            "apps/server",
            "crates/core",
            "crates/pdf",
        ] {
            write(
                root,
                &format!("{member}/Cargo.toml"),
                "[package]\nname = \"x\"\nversion.workspace = true\n",
            );
        }
    }

    #[test]
    fn every_member_inheriting_is_one_version() {
        let tmp = TempDir::new().expect("BUG: tempdir");
        rakete_shaped(tmp.path());
        assert!(is_single_version_workspace(&tmp.path().join("Cargo.toml")));
    }

    #[test]
    fn one_member_pinning_its_own_version_is_not() {
        let tmp = TempDir::new().expect("BUG: tempdir");
        rakete_shaped(tmp.path());
        // This is the case the doc comment warns about: the root key is set,
        // but the members do not all take it.
        write(
            tmp.path(),
            "crates/pdf/Cargo.toml",
            "[package]\nname = \"pdf\"\nversion = \"0.3.0\"\n",
        );
        assert!(!is_single_version_workspace(&tmp.path().join("Cargo.toml")));
    }

    #[test]
    fn a_root_without_an_inheritable_version_is_not() {
        let tmp = TempDir::new().expect("BUG: tempdir");
        write(
            tmp.path(),
            "Cargo.toml",
            "[workspace]\nmembers = [\"crates/*\"]\n",
        );
        write(
            tmp.path(),
            "crates/core/Cargo.toml",
            "[package]\nname = \"core\"\nversion = \"1.0.0\"\n",
        );
        assert!(!is_single_version_workspace(&tmp.path().join("Cargo.toml")));
    }

    #[test]
    fn a_workspace_with_no_members_on_disk_is_not() {
        let tmp = TempDir::new().expect("BUG: tempdir");
        write(
            tmp.path(),
            "Cargo.toml",
            "[workspace]\nmembers = [\"crates/*\"]\n\n[workspace.package]\nversion = \"0.1.0\"\n",
        );
        assert!(!is_single_version_workspace(&tmp.path().join("Cargo.toml")));
    }

    #[test]
    fn src_tauri_finds_the_repo_root_workspace() {
        let tmp = TempDir::new().expect("BUG: tempdir");
        rakete_shaped(tmp.path());
        let found = enclosing_workspace_root(
            &tmp.path().join("apps/rakete/src-tauri/Cargo.toml"),
            tmp.path(),
        );
        assert_eq!(found.as_deref(), Some(tmp.path()));
    }

    #[test]
    fn a_standalone_crate_has_no_enclosing_workspace() {
        let tmp = TempDir::new().expect("BUG: tempdir");
        write(
            tmp.path(),
            "apps/desktop/src-tauri/Cargo.toml",
            "[package]\nname = \"desktop\"\nversion = \"0.1.0\"\n",
        );
        assert_eq!(
            enclosing_workspace_root(
                &tmp.path().join("apps/desktop/src-tauri/Cargo.toml"),
                tmp.path()
            ),
            None
        );
    }

    #[test]
    fn the_walk_stops_at_the_ceiling() {
        let tmp = TempDir::new().expect("BUG: tempdir");
        // A workspace *above* the repo root must not be picked up.
        write(
            tmp.path(),
            "Cargo.toml",
            "[workspace]\nmembers = [\"repo/apps/desktop/src-tauri\"]\n\n[workspace.package]\nversion = \"9.9.9\"\n",
        );
        let repo_root = tmp.path().join("repo");
        write(
            &repo_root,
            "apps/desktop/src-tauri/Cargo.toml",
            "[package]\nname = \"desktop\"\nversion = \"0.1.0\"\n",
        );
        assert_eq!(
            enclosing_workspace_root(
                &repo_root.join("apps/desktop/src-tauri/Cargo.toml"),
                &repo_root
            ),
            None
        );
    }

    #[test]
    fn members_globs_and_excludes_are_understood() {
        let doc: DocumentMut =
            "[workspace]\nmembers = [\"crates/*\", \"apps/**\"]\nexclude = [\"crates/legacy\"]\n"
                .parse()
                .expect("BUG: should parse");
        assert!(workspace_claims_member(&doc, "crates/core"));
        assert!(workspace_claims_member(&doc, "apps/rakete/src-tauri"));
        assert!(!workspace_claims_member(&doc, "crates/legacy"));
        // `crates/*` is one level only.
        assert!(!workspace_claims_member(&doc, "crates/core/macros"));
    }
}
