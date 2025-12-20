use super::*;

#[test]
fn test_repo_path_empty() {
    let path = RepoPath::new(b"");
    assert!(path.is_empty());
    assert_eq!(path.len(), 0);
}

#[test]
fn test_repo_path_split_basename_no_separator() {
    let path = RepoPath::new(b"file.txt");
    let (dirname, basename) = path.split_basename();
    assert!(dirname.is_empty());
    assert_eq!(basename.as_ref(), b"file.txt");
}

#[test]
fn test_repo_path_split_basename_with_separator() {
    let path = RepoPath::new(b"dir/file.txt");
    let (dirname, basename) = path.split_basename();
    assert_eq!(dirname.as_ref(), b"dir/");
    assert_eq!(basename.as_ref(), b"file.txt");
}

#[test]
fn test_repo_path_split_basename_nested() {
    let path = RepoPath::new(b"a/b/c/file.txt");
    let (dirname, basename) = path.split_basename();
    assert_eq!(dirname.as_ref(), b"a/b/c/");
    assert_eq!(basename.as_ref(), b"file.txt");
}

#[test]
fn test_repo_path_split_basename_trailing_slash() {
    let path = RepoPath::new(b"dir/");
    let (dirname, basename) = path.split_basename();
    assert_eq!(dirname.as_ref(), b"dir/");
    assert_eq!(basename.as_ref(), b"");
}

#[test]
fn test_repo_path_pop_sep_with_trailing() {
    let path = RepoPath::new(b"dir/");
    let result = path.pop_sep();
    assert_eq!(result.as_ref(), b"dir");
}

#[test]
fn test_repo_path_pop_sep_without_trailing() {
    let path = RepoPath::new(b"dir");
    let result = path.pop_sep();
    assert_eq!(result.as_ref(), b"dir");
}

#[test]
fn test_repo_path_pop_sep_empty() {
    let path = RepoPath::new(b"");
    let result = path.pop_sep();
    assert!(result.is_empty());
}

#[test]
fn test_repo_path_starts_with_true() {
    let path = RepoPath::new(b"src/main.rs");
    assert!(path.starts_with(b"src"));
    assert!(path.starts_with(b"src/"));
    assert!(path.starts_with(b"src/main"));
}

#[test]
fn test_repo_path_starts_with_false() {
    let path = RepoPath::new(b"src/main.rs");
    assert!(!path.starts_with(b"test"));
    assert!(!path.starts_with(b"lib"));
    assert!(!path.starts_with(b"src/main.rs.bak"));
}

#[test]
fn test_repo_path_starts_with_empty() {
    let path = RepoPath::new(b"src/main.rs");
    assert!(path.starts_with(b""));
}

#[test]
fn test_repo_path_ends_with_true() {
    let path = RepoPath::new(b"src/main.rs");
    assert!(path.ends_with(b".rs"));
    assert!(path.ends_with(b"main.rs"));
    assert!(path.ends_with(b"src/main.rs"));
}

#[test]
fn test_repo_path_ends_with_false() {
    let path = RepoPath::new(b"src/main.rs");
    assert!(!path.ends_with(b".txt"));
    assert!(!path.ends_with(b"test.rs"));
}

#[test]
fn test_repo_path_ends_with_empty() {
    let path = RepoPath::new(b"src/main.rs");
    assert!(path.ends_with(b""));
}

#[test]
fn test_repo_path_buf_new() {
    let path = RepoPathBuf::new(b"test/path");
    let bytes: &[u8] = path.as_ref();
    assert_eq!(bytes, b"test/path");
}

#[test]
fn test_repo_path_buf_push_to_empty() {
    let mut path = RepoPathBuf::new(b"");
    path.push(b"file.txt");
    let bytes: &[u8] = path.as_ref();
    assert_eq!(bytes, b"file.txt");
}

#[test]
fn test_repo_path_buf_push_without_trailing_sep() {
    let mut path = RepoPathBuf::new(b"dir");
    path.push(b"file.txt");
    let bytes: &[u8] = path.as_ref();
    assert_eq!(bytes, b"dir/file.txt");
}

#[test]
fn test_repo_path_buf_push_with_trailing_sep() {
    let mut path = RepoPathBuf::new(b"dir/");
    path.push(b"file.txt");
    let bytes: &[u8] = path.as_ref();
    assert_eq!(bytes, b"dir/file.txt");
}

#[test]
fn test_repo_path_buf_push_multiple() {
    let mut path = RepoPathBuf::new(b"a");
    path.push(b"b");
    path.push(b"c");
    let bytes: &[u8] = path.as_ref();
    assert_eq!(bytes, b"a/b/c");
}

#[test]
fn test_repo_path_buf_truncate() {
    let mut path = RepoPathBuf::new(b"src/main.rs");
    path.truncate(3);
    let bytes: &[u8] = path.as_ref();
    assert_eq!(bytes, b"src");
}

#[test]
fn test_repo_path_buf_truncate_zero() {
    let mut path = RepoPathBuf::new(b"src/main.rs");
    path.truncate(0);
    assert!(path.is_empty());
}

#[test]
fn test_escape_pathlike_valid_utf8() {
    let result = escape_pathlike(b"test/path.txt");
    assert_eq!(result, "test/path.txt");
}

#[test]
fn test_escape_pathlike_invalid_utf8() {
    let invalid_bytes: &[u8] = &[0xFF, 0xFE];
    let result = escape_pathlike(invalid_bytes);
    assert!(result.starts_with('"'));
    assert!(result.ends_with('"'));
    assert!(result.contains("\\xff"));
}

#[test]
fn test_escape_pathlike_empty() {
    let result = escape_pathlike(b"");
    assert_eq!(result, "");
}

#[test]
fn test_escape_pathlike_null_byte() {
    let bytes_with_null: &[u8] = &[0x00];
    let result = escape_pathlike(bytes_with_null);
    assert!(result.starts_with('"'));
    assert!(result.ends_with('"'));
}

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
fn test_parse_history_ref_id_valid() {
    let repo = match Repository::open_from_env() {
        Ok(r) => r,
        Err(_) => return,
    };

    let valid_oid = "0000000000000000000000000000000000000000";
    let result = repo.parse_history_ref(valid_oid);
    assert!(result.is_ok());
    if let Ok(ParsedHistoryRef::Id(_)) = result {
    } else {
        panic!("Expected ParsedHistoryRef::Id");
    }
}

#[test]
fn test_parse_history_ref_thiscommit() {
    let repo = match Repository::open_from_env() {
        Ok(r) => r,
        Err(_) => return,
    };

    let result = repo.parse_history_ref("thiscommit:abc123");
    assert!(result.is_ok());
    if let Ok(ParsedHistoryRef::ThisCommit { salt }) = result {
        assert_eq!(salt, "abc123");
    } else {
        panic!("Expected ParsedHistoryRef::ThisCommit");
    }
}

#[test]
fn test_parse_history_ref_thiscommit_empty_salt() {
    let repo = match Repository::open_from_env() {
        Ok(r) => r,
        Err(_) => return,
    };

    let result = repo.parse_history_ref("thiscommit:");
    assert!(result.is_ok());
    if let Ok(ParsedHistoryRef::ThisCommit { salt }) = result {
        assert_eq!(salt, "");
    } else {
        panic!("Expected ParsedHistoryRef::ThisCommit");
    }
}

#[test]
fn test_parse_history_ref_manual() {
    let repo = match Repository::open_from_env() {
        Ok(r) => r,
        Err(_) => return,
    };

    let result = repo.parse_history_ref("manual:1.0.0");
    assert!(result.is_ok());
    if let Ok(ParsedHistoryRef::Manual(text)) = result {
        assert_eq!(text, "1.0.0");
    } else {
        panic!("Expected ParsedHistoryRef::Manual");
    }
}

#[test]
fn test_parse_history_ref_manual_empty() {
    let repo = match Repository::open_from_env() {
        Ok(r) => r,
        Err(_) => return,
    };

    let result = repo.parse_history_ref("manual:");
    assert!(result.is_ok());
    if let Ok(ParsedHistoryRef::Manual(text)) = result {
        assert_eq!(text, "");
    } else {
        panic!("Expected ParsedHistoryRef::Manual");
    }
}

#[test]
fn test_parse_history_ref_invalid() {
    let repo = match Repository::open_from_env() {
        Ok(r) => r,
        Err(_) => return,
    };

    let result = repo.parse_history_ref("invalid-ref-format");
    assert!(result.is_err());
}

#[test]
fn test_parse_history_ref_invalid_oid() {
    let repo = match Repository::open_from_env() {
        Ok(r) => r,
        Err(_) => return,
    };

    let result = repo.parse_history_ref("not-a-valid-oid");
    assert!(result.is_err());
}

#[test]
fn test_change_list_default() {
    let changes = ChangeList::default();
    assert_eq!(changes.paths().count(), 0);
}

#[test]
fn test_change_list_add_path() {
    let mut changes = ChangeList::default();
    changes.add_path(RepoPath::new(b"file1.txt"));
    changes.add_path(RepoPath::new(b"file2.txt"));
    assert_eq!(changes.paths().count(), 2);
}

#[test]
fn test_change_list_add_duplicate_paths() {
    let mut changes = ChangeList::default();
    changes.add_path(RepoPath::new(b"file.txt"));
    changes.add_path(RepoPath::new(b"file.txt"));
    assert_eq!(changes.paths().count(), 2);
}

#[test]
fn test_repo_history_n_commits() {
    let history = RepoHistory {
        commits: vec![CommitId(git2::Oid::zero()), CommitId(git2::Oid::zero())],
        release_tag: None,
    };
    assert_eq!(history.n_commits(), 2);
}

#[test]
fn test_repo_history_n_commits_empty() {
    let history = RepoHistory {
        commits: vec![],
        release_tag: None,
    };
    assert_eq!(history.n_commits(), 0);
}

#[test]
fn test_repo_history_release_tag_some() {
    let history = RepoHistory {
        commits: vec![],
        release_tag: Some(ReleaseTagInfo {
            commit: CommitId(git2::Oid::zero()),
            tag_name: "test-v1.0.0".to_string(),
            version: semver::Version::new(1, 0, 0),
        }),
    };
    assert!(history.release_tag().is_some());
    assert_eq!(
        history.release_version().unwrap(),
        &semver::Version::new(1, 0, 0)
    );
}

#[test]
fn test_repo_history_release_tag_none() {
    let history = RepoHistory {
        commits: vec![],
        release_tag: None,
    };
    assert!(history.release_tag().is_none());
    assert!(history.release_version().is_none());
}

#[test]
fn test_commit_id_display() {
    let oid = git2::Oid::zero();
    let commit_id = CommitId(oid);
    let display_str = format!("{}", commit_id);
    assert_eq!(display_str, "0000000000000000000000000000000000000000");
}

#[test]
fn test_commit_id_equality() {
    let oid1 = git2::Oid::zero();
    let oid2 = git2::Oid::zero();
    let commit_id1 = CommitId(oid1);
    let commit_id2 = CommitId(oid2);
    assert_eq!(commit_id1, commit_id2);
}

#[test]
fn test_release_tag_info() {
    let tag_info = ReleaseTagInfo {
        commit: CommitId(git2::Oid::zero()),
        tag_name: "my-package-v1.2.3".to_string(),
        version: semver::Version::new(1, 2, 3),
    };
    assert_eq!(tag_info.tag_name, "my-package-v1.2.3");
    assert_eq!(tag_info.version, semver::Version::new(1, 2, 3));
}

#[test]
#[cfg(unix)]
fn test_path_with_null_byte_rejected() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    let bytes_with_null = b"path/to\x00/file.txt";
    let os_str = OsStr::from_bytes(bytes_with_null);
    let path = Path::new(os_str);

    let result = RepoPathBuf::from_path(path);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("null byte"));
}

#[test]
fn test_path_traversal_parent_dir_rejected() {
    use std::path::Path;

    let paths = vec![
        "../etc/passwd",
        "foo/../bar",
        "foo/../../etc/passwd",
        "..",
        "../",
        "foo/..",
        "foo/../",
    ];

    for path_str in paths {
        let path = Path::new(path_str);
        let result = RepoPathBuf::from_path(path);
        assert!(
            result.is_err(),
            "Expected path '{}' to be rejected, but it was accepted",
            path_str
        );
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("parent directory") || err.to_string().contains(".."),
            "Expected error message to mention parent directory traversal, got: {}",
            err
        );
    }
}

#[test]
fn test_path_current_dir_rejected() {
    use std::path::Path;

    let paths = vec![".", "./", "./foo", "foo/./bar"];

    for path_str in paths {
        let path = Path::new(path_str);
        let result = RepoPathBuf::from_path(path);
        assert!(
            result.is_err(),
            "Expected path '{}' to be rejected, but it was accepted",
            path_str
        );
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("current directory") || err.to_string().contains("."),
            "Expected error message to mention current directory reference, got: {}",
            err
        );
    }
}

#[test]
fn test_absolute_path_rejected() {
    use std::path::Path;

    #[cfg(unix)]
    let paths = vec!["/etc/passwd", "/tmp/foo"];

    #[cfg(windows)]
    let paths = vec!["C:\\Windows", "\\Windows", "C:/Windows"];

    for path_str in paths {
        let path = Path::new(path_str);
        let result = RepoPathBuf::from_path(path);
        assert!(
            result.is_err(),
            "Expected absolute path '{}' to be rejected, but it was accepted",
            path_str
        );
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("relative") || err.to_string().contains("root"),
            "Expected error message to mention path must be relative, got: {}",
            err
        );
    }
}

#[test]
#[cfg(windows)]
fn test_windows_reserved_names_rejected() {
    use std::path::Path;

    let reserved = vec![
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];

    for name in reserved {
        let path = Path::new(name);
        let result = RepoPathBuf::from_path(path);
        assert!(
            result.is_err(),
            "Expected Windows reserved name '{}' to be rejected, but it was accepted",
            name
        );

        let with_extension = format!("{}.txt", name);
        let path = Path::new(&with_extension);
        let result = RepoPathBuf::from_path(path);
        assert!(
            result.is_err(),
            "Expected Windows reserved name with extension '{}' to be rejected, but it was accepted",
            with_extension
        );

        let lowercase = name.to_lowercase();
        let path = Path::new(&lowercase);
        let result = RepoPathBuf::from_path(path);
        assert!(
            result.is_err(),
            "Expected lowercase Windows reserved name '{}' to be rejected, but it was accepted",
            lowercase
        );
    }
}

#[test]
fn test_valid_relative_paths_accepted() {
    use std::path::Path;

    let valid_paths = vec![
        "foo/bar/baz.txt",
        "src/main.rs",
        "Cargo.toml",
        "foo_bar/baz-qux.txt",
        "a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p",
    ];

    for path_str in valid_paths {
        let path = Path::new(path_str);
        let result = RepoPathBuf::from_path(path);
        assert!(
            result.is_ok(),
            "Expected valid path '{}' to be accepted, but got error: {:?}",
            path_str,
            result
        );
    }
}

#[test]
fn test_very_long_path() {
    use std::path::Path;

    let component = "a".repeat(255);
    let long_path = format!("{}/{}/{}", component, component, component);
    let path = Path::new(&long_path);
    let result = RepoPathBuf::from_path(path);
    assert!(
        result.is_ok(),
        "Very long valid path should be accepted: {:?}",
        result
    );
}

#[test]
fn test_escape_null_byte_detection() {
    let path = b"foo\x00bar";
    let escaped = escape_pathlike(path);
    assert!(escaped.contains("null-byte"));
    assert!(escaped.contains("\\0"));
}

#[test]
fn test_escape_control_characters_detailed() {
    let path = b"foo\nbar\rbaz\ttab";
    let escaped = escape_pathlike(path);
    assert!(escaped.starts_with('"'));
    assert!(escaped.ends_with('"'));
    assert!(escaped.contains("\\n"));
    assert!(escaped.contains("\\r"));
    assert!(escaped.contains("\\t"));
}

#[test]
fn test_escape_path_with_spaces_detailed() {
    let path = b"foo bar/baz qux.txt";
    let escaped = escape_pathlike(path);
    assert!(escaped.starts_with('"'));
    assert!(escaped.ends_with('"'));
    assert!(escaped.contains("foo bar"));
}

#[test]
fn test_escape_backslash_detailed() {
    let path = b"foo\\bar";
    let escaped = escape_pathlike(path);
    assert!(escaped.contains("\\\\") || escaped == "foo\\bar");
}

#[test]
fn test_escape_quotes_detailed() {
    let path = br#"foo"bar"#;
    let escaped = escape_pathlike(path);
    assert!(escaped.contains(r#"\""#));
}

#[test]
fn test_escape_all_ascii_control_chars_security() {
    for i in 0..32u8 {
        let path = vec![b'a', i, b'b'];
        let escaped = escape_pathlike(&path);
        assert!(
            escaped.starts_with('"'),
            "Control char {} should trigger quoting",
            i
        );
    }
}

#[test]
fn test_escape_multiple_null_bytes_security() {
    let path = b"foo\x00bar\x00baz";
    let escaped = escape_pathlike(path);
    assert!(escaped.contains("null-byte"));
}

#[test]
fn test_parse_version_from_tag() {
    let tests = vec![
        ("gate-v1.0.0", semver::Version::new(1, 0, 0)),
        ("rig-v0.7.0", semver::Version::new(0, 7, 0)),
        ("my-project-v2.1.3", semver::Version::new(2, 1, 3)),
        ("invalid-tag", semver::Version::new(0, 0, 0)),
    ];

    for (tag_name, expected) in tests {
        let result = super::Repository::parse_version_from_tag(tag_name);
        assert_eq!(
            result, expected,
            "Failed to parse version from tag: {}",
            tag_name
        );
    }
}

#[test]
fn test_parse_version_from_tag_edge_cases() {
    assert_eq!(
        super::Repository::parse_version_from_tag(""),
        semver::Version::new(0, 0, 0)
    );

    assert_eq!(
        super::Repository::parse_version_from_tag("v1.0.0"),
        semver::Version::new(1, 0, 0)
    );

    assert_eq!(
        super::Repository::parse_version_from_tag("project-v1.0.0-alpha"),
        semver::Version::parse("1.0.0-alpha").unwrap()
    );
}
