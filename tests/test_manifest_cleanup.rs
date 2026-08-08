//! `prepare` carries out the manifest cleanup the github-app cannot.
//!
//! The app removes a processed manifest with a direct commit to the base
//! branch. On a protected branch it simply cannot — it is not a bypass actor —
//! so the files pile up, and a leftover manifest becomes indistinguishable from
//! one that was never processed.
//!
//! `prepare` already opens a pull request, so the deletion travels the reviewed
//! path instead of circumventing the protection.

mod common;

use common::TestRepo;

/// Write a manifest naming `tags`, as the app would have received it.
fn write_manifest(repo: &TestRepo, id: &str, tags: &[&str]) -> String {
    let releases: Vec<String> = tags
        .iter()
        .map(|t| {
            format!(
                r#"{{"name":"{t}","ecosystem":"cargo","previous_version":"0.1.0",
                   "new_version":"0.1.1","bump_type":"patch","tag_name":"{t}",
                   "is_prerelease":false,"changelog":"","contributors":[],
                   "first_time_contributors":[],"bundle_manifests":[],
                   "satellites":[],"cascade_inputs":[]}}"#
            )
        })
        .collect();
    let path = format!("belaf/releases/{id}.json");
    repo.write_file(
        &path,
        &format!(
            r#"{{"schema_version":"1","manifest_id":"{id}","base_branch":"main",
               "created_at":"2026-08-08T08:47:51Z","created_by":"t",
               "groups":[],"releases":[{}],"x":{{}}}}"#,
            releases.join(",")
        ),
    );
    path
}

const RELEASED: &str = "019fd3de-cc7f-7fc0-ba0c-9ffbd65ba54c";
const PENDING: &str = "019fdbdd-0387-7373-a4d8-9477388549f7";

fn scaffold(repo: &TestRepo) {
    repo.write_file(
        "Cargo.toml",
        "[package]\nname = \"alpha\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    repo.write_file("src/lib.rs", "pub fn a() {}\n");
    repo.write_file("belaf/config.toml", "");
    repo.commit("Initial commit");
    let gen = std::process::Command::new("cargo")
        .args(["generate-lockfile"])
        .current_dir(&repo.path)
        .output()
        .expect("cargo generate-lockfile");
    assert!(gen.status.success());
    repo.commit("chore: lockfile");
    repo.tag("alpha-v0.1.0");
}

#[test]
fn a_released_manifest_is_removed_in_the_release_commit() {
    let repo = TestRepo::new();
    scaffold(&repo);

    // One manifest whose release happened (its tag exists), one still waiting.
    let released = write_manifest(&repo, RELEASED, &["alpha-v0.1.0"]);
    let pending = write_manifest(&repo, PENDING, &["beta-v9.9.9"]);
    repo.commit("chore: leftover manifests");

    repo.write_file("src/feature.rs", "pub fn f() {}\n");
    repo.commit("feat: something to release");

    let out = repo.run_belaf_command_with_env(&["prepare", "--ci"], &[("BELAF_NO_KEYRING", "1")]);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        !repo.file_exists(&released),
        "the manifest whose tag exists must be deleted\nstderr:\n{stderr}"
    );
    assert!(
        repo.file_exists(&pending),
        "a manifest whose release has not happened must survive"
    );

    // Deleting it on disk is not enough — it has to be staged, or the pull
    // request carries the old file and nothing changes for the next run.
    let committed = repo.git(&["show", "--name-status", "--format=", "HEAD"]);
    assert!(
        committed
            .lines()
            .any(|l| l.starts_with('D') && l.contains(&released)),
        "the deletion must be part of the release commit; commit contained:\n{committed}"
    );
    assert!(
        repo.git(&["status", "--porcelain"]).is_empty(),
        "`prepare` must leave no uncommitted changes; got:\n{}",
        repo.git(&["status", "--porcelain"])
    );
}

#[test]
fn nothing_is_removed_when_no_tags_are_visible() {
    // What an incomplete tag fetch looks like. Fewer visible tags must only
    // ever mean "not released yet" — never a licence to delete.
    let repo = TestRepo::new();
    scaffold(&repo);

    let untagged = write_manifest(&repo, RELEASED, &["never-tagged-v1.0.0"]);
    repo.commit("chore: leftover manifest");

    repo.write_file("src/feature.rs", "pub fn f() {}\n");
    repo.commit("feat: something to release");

    let _ = repo.run_belaf_command_with_env(&["prepare", "--ci"], &[("BELAF_NO_KEYRING", "1")]);

    assert!(
        repo.file_exists(&untagged),
        "a manifest with no matching tag must be left in place"
    );
}
