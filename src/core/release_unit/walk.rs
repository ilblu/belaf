//! Filesystem walk + manifest sniffing helpers shared by every
//! detector in this module. Kept private to the detector module
//! family; callers outside should use the higher-level scan
//! functions.

use std::path::{Path, PathBuf};

use crate::core::git::repository::{RepoPathBuf, Repository};

pub(in crate::core::release_unit) fn workdir(repo: &Repository) -> Option<PathBuf> {
    let p = repo.resolve_workdir(&RepoPathBuf::new(b""));
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

pub(in crate::core::release_unit) fn relative_repopath(
    workdir: &Path,
    abs: &Path,
) -> Option<RepoPathBuf> {
    let rel = abs.strip_prefix(workdir).ok()?;
    let s = rel.to_string_lossy().to_string();
    if s.is_empty() {
        return None;
    }
    Some(RepoPathBuf::new(s.as_bytes()))
}

pub(in crate::core::release_unit) fn walk_capped<F: FnMut(&Path)>(
    workdir: &Path,
    max_depth: usize,
    mut f: F,
) {
    fn skip_dir(name: &str) -> bool {
        matches!(
            name,
            "node_modules"
                | "target"
                | ".git"
                | ".idea"
                | ".vscode"
                | "build"
                | "dist"
                | ".next"
                | "out"
                | "vendor"
                | "third_party"
                | "Pods"
                | "DerivedData"
        )
    }

    fn rec<F: FnMut(&Path)>(p: &Path, depth_left: usize, f: &mut F) {
        if depth_left == 0 {
            return;
        }
        f(p);
        let entries = match std::fs::read_dir(p) {
            Ok(e) => e,
            Err(_) => return,
        };
        // Sorted, because `read_dir` yields entries in filesystem order:
        // hash order on ext4, something else again on APFS. Detection
        // order therefore differed between a developer's machine and CI
        // for the same tree, which made `init --auto-detect` emit the
        // same blocks in a different sequence and the drift report list
        // the same paths in a different order. Cheap to make stable, and
        // stable is what a diffable config and a snapshot test both need.
        let mut dirs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|path| path.is_dir())
            .filter(|path| {
                path.file_name()
                    .and_then(|n| n.to_str())
                    .is_none_or(|name| !skip_dir(name))
            })
            .collect();
        dirs.sort();
        for path in dirs {
            rec(&path, depth_left - 1, f);
        }
    }

    rec(workdir, max_depth, &mut f);
}

pub(in crate::core::release_unit) fn find_dirs_with_subdir_pattern(
    workdir: &Path,
    name: &str,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_capped(workdir, 5, |p| {
        let candidate = p.join(name);
        if candidate.is_dir() {
            out.push(candidate);
        }
    });
    out
}

pub(in crate::core::release_unit) fn find_dirs_with_files_set(
    workdir: &Path,
    files: &[&str],
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_capped(workdir, 5, |p| {
        if files.iter().all(|f| p.join(f).exists()) {
            out.push(p.to_path_buf());
        }
    });
    out
}

pub(in crate::core::release_unit) fn list_subdirs_with_file(
    dir: &Path,
    file_name: &str,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join(file_name).exists() {
                out.push(path);
            }
        }
    }
    // Same reason as `walk_capped`: filesystem order is not an order.
    out.sort();
    out
}

pub(in crate::core::release_unit) fn cargo_toml_has_package_section(path: &Path) -> bool {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let doc: toml_edit::DocumentMut = match content.parse() {
        Ok(d) => d,
        Err(_) => return false,
    };
    doc.get("package").and_then(|p| p.as_table()).is_some()
}

pub(in crate::core::release_unit) fn cargo_toml_has_workspace_section(path: &Path) -> bool {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let doc: toml_edit::DocumentMut = match content.parse() {
        Ok(d) => d,
        Err(_) => return false,
    };
    doc.get("workspace").and_then(|p| p.as_table()).is_some()
}

pub(in crate::core::release_unit) fn file_contains_line(path: &Path, prefix: &str) -> bool {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    content.lines().any(|l| l.trim_start().starts_with(prefix))
}

pub(in crate::core::release_unit) fn file_contains_pattern(path: &Path, pattern: &str) -> bool {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let re = match regex::Regex::new(pattern) {
        Ok(r) => r,
        Err(_) => return false,
    };
    re.is_match(&content)
}
