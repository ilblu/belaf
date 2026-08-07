//! The pre-flight tag fetch that `prepare` runs before deciding anything.
//!
//! It exists because release tags are created server-side by the belaf GitHub
//! App, so a local clone goes stale after every merged release. But it is a
//! *refresh*, not a prerequisite — an unreachable or unauthenticated remote
//! must not kill a run that has perfectly good tags on disk.
//!
//! Note these tests deliberately use `run_belaf_command_with_fetch`: the rest
//! of the suite sets `BELAF_NO_FETCH=1`, which is exactly why the fetch path
//! went unexercised long enough to ship broken.

mod common;

use common::TestRepo;

fn init_repo_with_release(repo: &TestRepo) {
    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "fetch-test"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
    repo.commit("Initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(output.status.success(), "init should succeed");

    // A released state: the clone already knows where the last release was.
    repo.tag("fetch-test-v1.0.0");

    repo.write_file("src/feature.rs", "pub fn feature() {}\n");
    repo.commit("feat: add feature");
}

/// The remote is a placeholder URL that resolves to nothing, so the fetch
/// fails. With tags on disk the run must continue on them and only warn.
#[test]
fn unreachable_remote_does_not_kill_a_run_that_has_local_tags() {
    let repo = TestRepo::new();
    init_repo_with_release(&repo);

    let output = repo.run_belaf_command_with_fetch(&["prepare", "--ci"]);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        !combined.contains("failed to fetch upstream tags before release prep"),
        "the fetch must not be fatal when local tags exist. output:\n{combined}"
    );

    // It got far enough to do real work: either it emitted a manifest, or it
    // reached the push/PR stage and failed there (no network, no auth).
    let emitted_manifest = !repo.list_files_in_dir("belaf/releases").is_empty();
    assert!(
        emitted_manifest,
        "the run should have progressed past tag analysis and written a manifest. \
         output:\n{combined}"
    );
}

/// The warning has to be visible — silently working from a stale tag view is
/// how a release gets computed against the wrong baseline.
#[test]
fn a_failed_fetch_is_reported() {
    let repo = TestRepo::new();
    init_repo_with_release(&repo);

    let output = repo.run_belaf_command_with_fetch(&["prepare", "--ci"]);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        combined.contains("could not refresh tags from the upstream"),
        "a failed refresh must be surfaced, not swallowed. output:\n{combined}"
    );
}

/// Without any tags on disk there is nothing to fall back to: every unit would
/// look brand-new and the bumps would be wrong. That case stays fatal.
#[test]
fn a_failed_fetch_is_fatal_when_the_clone_has_no_tags() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "fetch-test-untagged"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
    repo.commit("Initial commit");
    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(output.status.success(), "init should succeed");

    repo.write_file("src/feature.rs", "pub fn feature() {}\n");
    repo.commit("feat: add feature");

    let output = repo.run_belaf_command_with_fetch(&["prepare", "--ci"]);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        !output.status.success(),
        "with no tags to fall back on the run must fail. output:\n{combined}"
    );
    assert!(
        combined.contains("no version tags to fall back on"),
        "the error should say why it is fatal here. output:\n{combined}"
    );
}
