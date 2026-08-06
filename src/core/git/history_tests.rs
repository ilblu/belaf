//! Unit tests for [`crate::core::git::history`].
//!
//! Moved here from `repository_tests.rs` when `RepoHistory` moved into its own
//! module, so the tests sit next to the code they cover.

use crate::core::git::history::{HistoryBoundary, RepoHistory};
use crate::core::git::repository::CommitId;

#[test]
fn test_repo_history_n_commits() {
    let history = RepoHistory {
        commits: vec![CommitId(git2::Oid::zero()), CommitId(git2::Oid::zero())],
        boundary: None,
        provenance: Default::default(),
        input_hits: Default::default(),
    };
    assert_eq!(history.n_commits(), 2);
}

#[test]
fn test_repo_history_n_commits_empty() {
    let history = RepoHistory {
        commits: vec![],
        boundary: None,
        provenance: Default::default(),
        input_hits: Default::default(),
    };
    assert_eq!(history.n_commits(), 0);
}

#[test]
fn test_repo_history_with_release_tag() {
    let history = RepoHistory {
        commits: vec![],
        boundary: Some(HistoryBoundary::ReleaseTag {
            commit: CommitId(git2::Oid::zero()),
            tag_name: "test-v1.0.0".to_string(),
            version: semver::Version::new(1, 0, 0),
        }),
        provenance: Default::default(),
        input_hits: Default::default(),
    };
    assert!(history.has_release_tag());
    assert!(history.boundary_commit().is_some());
    assert_eq!(
        history.release_version().unwrap(),
        &semver::Version::new(1, 0, 0)
    );
}

#[test]
fn test_repo_history_with_baseline() {
    let history = RepoHistory {
        commits: vec![],
        boundary: Some(HistoryBoundary::Baseline {
            commit: CommitId(git2::Oid::zero()),
        }),
        provenance: Default::default(),
        input_hits: Default::default(),
    };
    assert!(!history.has_release_tag());
    assert!(history.boundary_commit().is_some());
    assert!(history.release_version().is_none());
}

#[test]
fn test_repo_history_no_boundary() {
    let history = RepoHistory {
        commits: vec![],
        boundary: None,
        provenance: Default::default(),
        input_hits: Default::default(),
    };
    assert!(!history.has_release_tag());
    assert!(history.boundary_commit().is_none());
    assert!(history.release_version().is_none());
}

#[test]
fn test_history_boundary_release_tag() {
    let boundary = HistoryBoundary::ReleaseTag {
        commit: CommitId(git2::Oid::zero()),
        tag_name: "my-package-v1.2.3".to_string(),
        version: semver::Version::new(1, 2, 3),
    };
    match boundary {
        HistoryBoundary::ReleaseTag {
            tag_name, version, ..
        } => {
            assert_eq!(tag_name, "my-package-v1.2.3");
            assert_eq!(version, semver::Version::new(1, 2, 3));
        }
        _ => panic!("Expected ReleaseTag variant"),
    }
}
