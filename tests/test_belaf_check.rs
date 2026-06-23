//! F8 — `belaf check` commit-label validation.

mod common;

use common::TestRepo;

fn seed_crate(repo: &TestRepo) {
    repo.write_file(
        "Cargo.toml",
        "[package]\nname = \"mylib\"\nversion = \"1.0.0\"\nedition = \"2021\"\n",
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
    repo.commit("Initial commit");
    let out =
        repo.run_belaf_command_with_env(&["init", "--ci", "--force"], &[("BELAF_NO_KEYRING", "1")]);
    assert!(out.status.success(), "init must succeed");
    repo.commit("chore: bootstrap belaf");
}

#[test]
fn check_message_mode_flags_unknown_scope() {
    let repo = TestRepo::new();
    seed_crate(&repo);

    // Unknown scope → violation → hard-fail under --ci.
    let bad = repo.run_belaf_command_with_env(
        &["check", "--ci", "--message", "feat(ghost): nope"],
        &[("BELAF_NO_KEYRING", "1")],
    );
    assert!(
        !bad.status.success(),
        "unknown scope must fail under --ci; stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&bad.stdout),
        String::from_utf8_lossy(&bad.stderr),
    );

    // Known scope → OK.
    let good = repo.run_belaf_command_with_env(
        &["check", "--ci", "--message", "feat(mylib): yep"],
        &[("BELAF_NO_KEYRING", "1")],
    );
    assert!(
        good.status.success(),
        "known scope must pass; stderr:\n{}",
        String::from_utf8_lossy(&good.stderr),
    );

    // No scope → nothing to validate → OK.
    let none = repo.run_belaf_command_with_env(
        &["check", "--ci", "--message", "feat: no scope here"],
        &[("BELAF_NO_KEYRING", "1")],
    );
    assert!(none.status.success(), "a scopeless commit must pass");
}

#[test]
fn check_without_ci_warns_but_succeeds() {
    let repo = TestRepo::new();
    seed_crate(&repo);

    // Same unknown scope, but without --ci → warning only, exit 0.
    let out = repo.run_belaf_command_with_env(
        &["check", "--message", "feat(ghost): nope"],
        &[("BELAF_NO_KEYRING", "1")],
    );
    assert!(
        out.status.success(),
        "without --ci, violations are warnings (exit 0); stderr:\n{}",
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn check_range_validates_commit_scope() {
    let repo = TestRepo::new();
    seed_crate(&repo);

    // HEAD is a correctly-scoped commit touching the crate.
    repo.write_file("src/good.rs", "pub fn good() {}\n");
    repo.commit("feat(mylib): add good");
    let ok = repo.run_belaf_command_with_env(
        &["check", "--ci", "--range", "HEAD"],
        &[("BELAF_NO_KEYRING", "1")],
    );
    assert!(
        ok.status.success(),
        "correctly-scoped HEAD must pass; stderr:\n{}",
        String::from_utf8_lossy(&ok.stderr),
    );

    // A mislabeled HEAD fails.
    repo.write_file("src/more.rs", "pub fn more() {}\n");
    repo.commit("feat(ghost): mislabeled");
    let bad = repo.run_belaf_command_with_env(
        &["check", "--ci", "--range", "HEAD"],
        &[("BELAF_NO_KEYRING", "1")],
    );
    assert!(
        !bad.status.success(),
        "mislabeled HEAD must fail under --ci"
    );
}
