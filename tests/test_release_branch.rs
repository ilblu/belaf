//! The release branch `prepare` pushes to.
//!
//! It is stable by design — re-running `prepare` targets the same branch and
//! force-updates it, which is what keeps one open release PR up to date
//! instead of opening a new one per run.
//!
//! These runs cannot reach the network, so `prepare` fails at the push step.
//! Everything asserted here happens before that: the branch is created,
//! named, and carries the release commit by the time the push is attempted.

mod common;

use common::TestRepo;

const ENV: &[(&str, &str)] = &[("BELAF_NO_KEYRING", "1")];

fn init_repo(repo: &TestRepo) {
    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "branch-test"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
    repo.commit("Initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(output.status.success(), "init should succeed");
}

/// Every local branch, one per line.
fn branches(repo: &TestRepo) -> Vec<String> {
    repo.git(&["branch", "--format=%(refname:short)"])
        .lines()
        .map(str::to_string)
        .collect()
}

fn release_branches(repo: &TestRepo) -> Vec<String> {
    branches(repo)
        .into_iter()
        .filter(|b| b.starts_with("belaf/release"))
        .collect()
}

#[test]
fn release_branch_is_named_after_the_base_branch() {
    let repo = TestRepo::new();
    init_repo(&repo);

    let base = repo.git(&["rev-parse", "--abbrev-ref", "HEAD"]);

    repo.write_file("src/feature.rs", "pub fn feature() {}\n");
    repo.commit("feat: add feature");

    let _ = repo.run_belaf_command_with_env(&["prepare", "--ci"], ENV);

    assert_eq!(
        release_branches(&repo),
        vec![format!("belaf/release--{base}")],
        "one release branch, named after the base branch"
    );
}

/// The whole point of the feature: a second run reuses the branch instead of
/// erroring with "Reference already exists" or piling up a new one.
///
/// Each run starts from the base branch, the way CI does — a fresh checkout
/// per run.
#[test]
fn rerunning_prepare_reuses_the_same_release_branch() {
    let repo = TestRepo::new();
    init_repo(&repo);

    let base = repo.git(&["rev-parse", "--abbrev-ref", "HEAD"]);

    repo.write_file("src/feature.rs", "pub fn feature() {}\n");
    repo.commit("feat: add feature");

    let first = repo.run_belaf_command_with_env(&["prepare", "--ci"], ENV);
    let after_first = release_branches(&repo);

    repo.git(&["checkout", "--force", &base]);
    repo.write_file("src/second.rs", "pub fn second() {}\n");
    repo.commit("feat: add another feature");

    let second = repo.run_belaf_command_with_env(&["prepare", "--ci"], ENV);
    let after_second = release_branches(&repo);

    assert_eq!(
        after_first, after_second,
        "the second run must reuse the branch, not add one"
    );
    assert_eq!(after_second.len(), 1, "exactly one release branch");

    for (label, output) in [("first", &first), ("second", &second)] {
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !stderr.contains("already exists"),
            "{label} run must not fail on an existing branch. stderr: {stderr}"
        );
    }
}

/// A run that fails at the push leaves HEAD on the release branch. Starting
/// again from there must be refused: the release branch name is derived from
/// the branch the run starts on, so it would compound into
/// `belaf/release--belaf-release--main` and base the PR on a release branch.
#[test]
fn prepare_refuses_to_start_from_a_release_branch() {
    let repo = TestRepo::new();
    init_repo(&repo);

    let base = repo.git(&["rev-parse", "--abbrev-ref", "HEAD"]);

    repo.write_file("src/feature.rs", "pub fn feature() {}\n");
    repo.commit("feat: add feature");

    // Fails at the push: the remote is a placeholder URL.
    let _ = repo.run_belaf_command_with_env(&["prepare", "--ci"], ENV);
    assert_eq!(
        repo.git(&["rev-parse", "--abbrev-ref", "HEAD"]),
        format!("belaf/release--{base}"),
        "a failed push leaves HEAD on the release branch"
    );

    // The failed run leaves a regenerated Cargo.lock behind; commit it so the
    // dirty-tree check does not mask the branch guard we are testing.
    repo.git(&["add", "-A"]);
    repo.commit("chore: settle working tree");

    let output = repo.run_belaf_command_with_env(&["prepare", "--ci"], ENV);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "starting from a release branch must fail, not compound the name"
    );
    assert!(
        stderr.contains("release branch") && stderr.contains("git switch"),
        "the error should name the problem and the way out. stderr: {stderr}"
    );
    assert_eq!(
        release_branches(&repo),
        vec![format!("belaf/release--{base}")],
        "no second, compounded release branch may appear"
    );
}

/// The branch is pushed exactly once, already carrying the release commit.
/// If it were ever pushed while equal to its base, GitHub would close the
/// open release PR.
#[test]
fn release_branch_is_ahead_of_its_base() {
    let repo = TestRepo::new();
    init_repo(&repo);

    let base = repo.git(&["rev-parse", "--abbrev-ref", "HEAD"]);

    repo.write_file("src/feature.rs", "pub fn feature() {}\n");
    repo.commit("feat: add feature");

    let _ = repo.run_belaf_command_with_env(&["prepare", "--ci"], ENV);

    let release_branch = format!("belaf/release--{base}");
    let ahead = repo.git(&["rev-list", "--count", &format!("{base}..{release_branch}")]);

    assert_eq!(
        ahead, "1",
        "the release branch must carry exactly the release commit"
    );
}

#[test]
fn release_branch_template_is_configurable() {
    let repo = TestRepo::new();
    init_repo(&repo);

    let config = repo.read_file("belaf/config.toml");
    repo.write_file(
        "belaf/config.toml",
        &config.replace(
            "[repo]",
            "[repo]\nrelease_branch = \"releases/from-{base}\"",
        ),
    );
    repo.commit("chore: set a release branch template");

    let base = repo.git(&["rev-parse", "--abbrev-ref", "HEAD"]);

    repo.write_file("src/feature.rs", "pub fn feature() {}\n");
    repo.commit("feat: add feature");

    let _ = repo.run_belaf_command_with_env(&["prepare", "--ci"], ENV);

    let all = branches(&repo);
    assert!(
        all.contains(&format!("releases/from-{base}")),
        "the configured template should decide the name. branches: {all:?}"
    );
    assert!(
        release_branches(&repo).is_empty(),
        "the default template must not also be used. branches: {all:?}"
    );
}
