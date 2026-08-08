//! Removing manifests whose releases have already happened.
//!
//! A manifest is a request: "please tag and release these versions". Once the
//! github-app has done so, the file has served its purpose — but nothing was
//! reliably removing it.
//!
//! The app does try, with a direct commit to the base branch. On a repository
//! with branch protection it cannot: the app is not a bypass actor, and making
//! it one would defeat the protection to solve a housekeeping problem. So the
//! files accumulate, and — worse — a leftover manifest becomes
//! indistinguishable from one that was never processed. The directory stops
//! telling you anything.
//!
//! `prepare` is the natural place to do it instead. It already opens a pull
//! request and already has write access to its own branch, so the deletion
//! travels the same reviewed path as everything else it writes, and branch
//! protection is satisfied rather than circumvented.
//!
//! **A manifest is done when every tag it names exists.** Tags are created by
//! the app on a successful release, so an existing tag *is* the evidence. That
//! makes the check purely local — no API call, no credentials — and it fails in
//! the safe direction: if the tag fetch was incomplete (it is deliberately
//! fail-soft), fewer tags are visible, the manifest looks unfinished, and it
//! stays. The failure mode is always "leaves it lying", never "deletes
//! something unreleased".

use std::path::Path;

use crate::core::git::repository::{RepoPathBuf, Repository};
use crate::core::manifest::{ReleaseManifest, MANIFEST_DIR};

/// A manifest on disk whose releases are all tagged.
#[derive(Debug, Clone)]
pub struct ProcessedManifest {
    /// Repo-relative path, ready for the release commit.
    pub path: RepoPathBuf,
    /// Tags the manifest asked for; all of them exist.
    pub tags: Vec<String>,
}

/// Find every manifest in `belaf/releases/` that has already been released.
///
/// `exclude` is the manifest this run just wrote — it names tags that do not
/// exist yet, so it would never match, but skipping it explicitly keeps the
/// intent obvious.
pub fn find_processed_manifests(
    repo: &Repository,
    exclude: Option<&RepoPathBuf>,
) -> Vec<ProcessedManifest> {
    let dir = repo.resolve_workdir(RepoPathBuf::new(MANIFEST_DIR.as_bytes()).as_ref());
    let Ok(entries) = std::fs::read_dir(&dir) else {
        // No manifest directory: nothing was ever released from this repo.
        return Vec::new();
    };

    let mut processed = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let Some(repo_path) = to_repo_path(&path) else {
            continue;
        };
        if exclude.is_some_and(|e| *e == repo_path) {
            continue;
        }

        let Some(tags) = released_tags(repo, &path) else {
            continue;
        };
        processed.push(ProcessedManifest {
            path: repo_path,
            tags,
        });
    }

    // Stable order so the release commit is reproducible.
    processed.sort_by(|a, b| a.path.escaped().cmp(&b.path.escaped()));
    processed
}

/// The tags a manifest names, if **all** of them exist. `None` when the file is
/// unreadable, unparseable, names no releases, or still has work outstanding.
fn released_tags(repo: &Repository, path: &Path) -> Option<Vec<String>> {
    let raw = std::fs::read_to_string(path).ok()?;
    // A manifest we cannot parse is not a manifest we may delete. It could be
    // a newer schema this build does not understand yet, and guessing would
    // throw away a release nobody has performed.
    let manifest = ReleaseManifest::from_json(&raw).ok()?;

    let tags: Vec<String> = manifest
        .releases
        .iter()
        .map(|r| r.tag_name.clone())
        .collect();

    // An empty manifest would vacuously satisfy "every tag exists". Nothing
    // legitimately produces one, so treat it as unfinished rather than invent
    // a reason to delete a file we do not understand.
    if tags.is_empty() {
        return None;
    }

    tags.iter().all(|t| repo.tag_exists(t)).then_some(tags)
}

/// Convert an absolute path under the manifest directory back to a
/// repo-relative one. `MANIFEST_DIR` is a fixed, ASCII, forward-slash prefix,
/// so this stays correct on Windows where `read_dir` hands back backslashes.
fn to_repo_path(abs: &Path) -> Option<RepoPathBuf> {
    let name = abs.file_name()?.to_str()?;
    Some(RepoPathBuf::new(
        format!("{MANIFEST_DIR}/{name}").as_bytes(),
    ))
}

#[cfg(test)]
#[path = "cleanup_tests.rs"]
mod cleanup_tests;
