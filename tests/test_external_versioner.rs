//! Phase E.3 — integration tests for ExternalVersionerRewriter.
//! Stub external tool: a tiny bash incantation that reads/writes a
//! version file, exercises the substitution, timeout, and re-read
//! verification logic.

mod common;

use std::collections::HashMap;

use belaf::core::git::repository::{RepoPathBuf, Repository};
use belaf::core::release_unit::ExternalVersioner;
use belaf::core::rewriters::external::{self, BumpKind};
use common::TestRepo;

fn open_repo(t: &TestRepo) -> Repository {
    Repository::open(&t.path).expect("Repository::open")
}

#[test]
fn round_trip_via_stub_shell_commands() {
    let repo = TestRepo::new();
    repo.write_file("version.txt", "1.0.0\n");
    repo.commit("seed");

    let r = open_repo(&repo);
    let ext = ExternalVersioner {
        tool: "stub".to_string(),
        // Read: cat the file, trim trailing whitespace.
        read_command: "cat version.txt".to_string(),
        // Write: replace contents with the new {version} placeholder.
        write_command: "printf '%s\\n' '{version}' > version.txt".to_string(),
        cwd: Some(RepoPathBuf::new(b"")),
        timeout_sec: 5,
        env: HashMap::new(),
    };

    // Initial read returns "1.0.0".
    let current = external::read_current(&ext, &r).expect("read");
    assert_eq!(current, "1.0.0");

    // Bump to 1.1.0.
    external::write_and_verify(&ext, &r, "stub-unit", "1.1.0", BumpKind::Minor).expect("write");

    // Verify the file actually moved.
    let after = std::fs::read_to_string(repo.path.join("version.txt")).unwrap();
    assert_eq!(after.trim(), "1.1.0");

    // Re-read via the API.
    let current2 = external::read_current(&ext, &r).expect("read post-write");
    assert_eq!(current2, "1.1.0");
}

#[test]
fn write_idempotent_when_already_at_target() {
    let repo = TestRepo::new();
    repo.write_file("version.txt", "1.1.0\n");
    repo.commit("seed");

    let r = open_repo(&repo);
    // Write command would actually CHANGE the file. Idempotency check
    // means it shouldn't even run.
    let ext = ExternalVersioner {
        tool: "stub".to_string(),
        read_command: "cat version.txt".to_string(),
        write_command: "printf '%s\\n' 'CORRUPTED' > version.txt".to_string(),
        cwd: Some(RepoPathBuf::new(b"")),
        timeout_sec: 5,
        env: HashMap::new(),
    };

    external::write_and_verify(&ext, &r, "stub-unit", "1.1.0", BumpKind::Minor)
        .expect("idempotent");

    let after = std::fs::read_to_string(repo.path.join("version.txt")).unwrap();
    assert_eq!(
        after.trim(),
        "1.1.0",
        "idempotent path must not corrupt the file"
    );
}

#[test]
fn timeout_kills_the_process() {
    let repo = TestRepo::new();
    repo.write_file("version.txt", "1.0.0\n");
    repo.commit("seed");

    let r = open_repo(&repo);
    let ext = ExternalVersioner {
        tool: "slow".to_string(),
        // sleep longer than the timeout.
        read_command: "sleep 30".to_string(),
        write_command: "true".to_string(),
        cwd: Some(RepoPathBuf::new(b"")),
        timeout_sec: 1,
        env: HashMap::new(),
    };

    let err = external::read_current(&ext, &r).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("timeout"),
        "expected timeout error, got: {msg}"
    );
}

#[test]
fn non_zero_exit_captured_with_stderr() {
    let repo = TestRepo::new();
    repo.commit("seed");

    let r = open_repo(&repo);
    let ext = ExternalVersioner {
        tool: "fail".to_string(),
        read_command: "echo 'something went wrong' 1>&2; exit 7".to_string(),
        write_command: "true".to_string(),
        cwd: Some(RepoPathBuf::new(b"")),
        timeout_sec: 5,
        env: HashMap::new(),
    };

    let err = external::read_current(&ext, &r).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("exited with status 7"),
        "expected exit code 7 in error: {msg}"
    );
    assert!(
        msg.contains("something went wrong"),
        "expected captured stderr in error: {msg}"
    );
}

#[test]
fn post_write_mismatch_surfaces_actual_vs_expected() {
    let repo = TestRepo::new();
    repo.write_file("version.txt", "1.0.0\n");
    repo.commit("seed");

    let r = open_repo(&repo);
    let ext = ExternalVersioner {
        tool: "stub".to_string(),
        read_command: "cat version.txt".to_string(),
        // Write command claims to bump but ignores {version} — file
        // stays at 1.0.0. Re-read should detect the mismatch.
        write_command: "true".to_string(),
        cwd: Some(RepoPathBuf::new(b"")),
        timeout_sec: 5,
        env: HashMap::new(),
    };

    let err =
        external::write_and_verify(&ext, &r, "stub-unit", "2.0.0", BumpKind::Major).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("re-read returned"),
        "expected post-write mismatch: {msg}"
    );
    assert!(msg.contains("expected `2.0.0`"));
}

#[test]
fn substitutions_and_env_vars_reach_command() {
    let repo = TestRepo::new();
    repo.commit("seed");

    let r = open_repo(&repo);
    let ext = ExternalVersioner {
        tool: "echo-tool".to_string(),
        // read_command gets BELAF_VERSION_NEW only on write — for
        // a pure read we just check tool name + a custom env.
        read_command: "echo \"$BELAF_EXTERNAL_TOOL/$MY_CUSTOM_ENV\"".to_string(),
        write_command: "true".to_string(),
        cwd: Some(RepoPathBuf::new(b"")),
        timeout_sec: 5,
        env: {
            let mut m = HashMap::new();
            m.insert("MY_CUSTOM_ENV".to_string(), "hello".to_string());
            m
        },
    };

    let v = external::read_current(&ext, &r).unwrap();
    assert_eq!(v, "echo-tool/hello");
}
