//! Unit tests for [`crate::core::git::path_matcher`].
//!
//! Moved here from `repository_tests.rs` when `PathMatcher` and
//! `is_binary_affecting` moved into their own module, so the tests sit next
//! to the code they cover.

use crate::core::git::path_matcher::{is_binary_affecting, PathMatcher};
use crate::core::git::repository::{RepoPath, RepoPathBuf};

#[test]
fn test_path_matcher_new_include() {
    let matcher = PathMatcher::new_include(RepoPathBuf::new(b"src"));
    let path = RepoPath::new(b"src/main.rs");
    assert!(matcher.repo_path_matches(path));
}

#[test]
fn test_path_matcher_no_match() {
    let matcher = PathMatcher::new_include(RepoPathBuf::new(b"src"));
    let path = RepoPath::new(b"test/main.rs");
    assert!(!matcher.repo_path_matches(path));
}

#[test]
fn test_path_matcher_exact_match() {
    let matcher = PathMatcher::new_include(RepoPathBuf::new(b"src/main.rs"));
    let path = RepoPath::new(b"src/main.rs");
    assert!(matcher.repo_path_matches(path));
}

#[test]
fn test_path_matcher_prefix_mismatch() {
    let matcher = PathMatcher::new_include(RepoPathBuf::new(b"src"));
    let path = RepoPath::new(b"source/file.rs");
    assert!(!matcher.repo_path_matches(path));
}

#[test]
fn test_path_matcher_make_disjoint() {
    let mut matcher1 = PathMatcher::new_include(RepoPathBuf::new(b""));
    let matcher2 = PathMatcher::new_include(RepoPathBuf::new(b"src"));
    matcher1.make_disjoint(&matcher2);

    assert!(!matcher1.repo_path_matches(RepoPath::new(b"src/main.rs")));
    assert!(matcher1.repo_path_matches(RepoPath::new(b"test/main.rs")));
}

#[test]
fn test_path_matcher_make_disjoint_non_overlapping() {
    let mut matcher1 = PathMatcher::new_include(RepoPathBuf::new(b"test"));
    let matcher2 = PathMatcher::new_include(RepoPathBuf::new(b"src"));
    matcher1.make_disjoint(&matcher2);

    assert!(matcher1.repo_path_matches(RepoPath::new(b"test/file.rs")));
    assert!(!matcher1.repo_path_matches(RepoPath::new(b"src/file.rs")));
}

#[test]
fn is_binary_affecting_excludes_segments_suffixes_names() {
    let cfg = crate::core::config::syntax::BinaryAffectingConfiguration {
        exclude_segments: vec!["tests".into(), "docs".into(), "examples".into()],
        exclude_suffixes: vec![".md".into()],
        exclude_names: vec!["CHANGELOG.md".into()],
    };
    // Affecting:
    assert!(is_binary_affecting(b"src/lib.rs", &cfg));
    assert!(is_binary_affecting(b"Cargo.toml", &cfg));
    // `examples` only matches a *whole segment*, not a substring:
    assert!(is_binary_affecting(b"src/examples_helper.rs", &cfg));
    // Not affecting:
    assert!(!is_binary_affecting(b"tests/it.rs", &cfg));
    assert!(!is_binary_affecting(b"crate/docs/guide.rs", &cfg));
    assert!(!is_binary_affecting(b"README.md", &cfg)); // .md suffix
    assert!(!is_binary_affecting(b"CHANGELOG.md", &cfg)); // exact name
    assert!(!is_binary_affecting(b"examples/demo.rs", &cfg));
}

#[test]
fn path_matcher_globs_match_additively() {
    let mut m = PathMatcher::new_globs_only();
    m.add_glob("**/*.sql").unwrap();
    assert!(m.has_globs());
    assert!(m.repo_path_matches(RepoPath::new(b"db/migrations/001.sql")));
    assert!(!m.repo_path_matches(RepoPath::new(b"src/lib.rs")));

    // Prefix + glob coexist: a path matches if EITHER hits.
    let mut m2 = PathMatcher::new_include(RepoPathBuf::new(b"src/"));
    m2.add_glob("**/*.sql").unwrap();
    assert!(m2.repo_path_matches(RepoPath::new(b"src/lib.rs"))); // prefix
    assert!(m2.repo_path_matches(RepoPath::new(b"other/x.sql"))); // glob
    assert!(!m2.repo_path_matches(RepoPath::new(b"other/x.rs")));
}
