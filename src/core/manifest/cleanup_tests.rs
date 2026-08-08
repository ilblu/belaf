//! The rule under test is one-directional: a manifest is deleted only when
//! every tag it names exists. Everything ambiguous — an unreadable file, a
//! newer schema, a partially-released set — has to leave the file alone.

use super::*;
use crate::core::git::repository::Repository;
use std::process::Command;
use tempfile::TempDir;

/// A git repo with a manifest directory and no tags.
fn scaffold() -> (TempDir, Repository) {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path();
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "t@e.com"],
        vec!["config", "user.name", "T"],
    ] {
        Command::new("git")
            .args(&args)
            .current_dir(root)
            .output()
            .expect("git");
    }
    std::fs::create_dir_all(root.join(MANIFEST_DIR)).expect("manifest dir");
    std::fs::write(root.join("README.md"), "x\n").expect("seed file");
    Command::new("git")
        .args(["add", "-A"])
        .current_dir(root)
        .output()
        .expect("git add");
    Command::new("git")
        .args(["commit", "-qm", "seed"])
        .current_dir(root)
        .output()
        .expect("git commit");

    let repo = Repository::open(root).expect("open repo");
    (dir, repo)
}

fn write_manifest(root: &std::path::Path, id: &str, tags: &[&str]) -> RepoPathBuf {
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
    let json = format!(
        r#"{{"schema_version":"1","manifest_id":"{id}","base_branch":"main",
           "created_at":"2026-08-08T08:47:51Z","created_by":"t",
           "groups":[],"releases":[{}],"x":{{}}}}"#,
        releases.join(",")
    );
    std::fs::write(root.join(MANIFEST_DIR).join(format!("{id}.json")), json)
        .expect("write manifest");
    RepoPathBuf::new(format!("{MANIFEST_DIR}/{id}.json").as_bytes())
}

fn tag(root: &std::path::Path, name: &str) {
    Command::new("git")
        .args(["tag", name])
        .current_dir(root)
        .output()
        .expect("git tag");
}

const ID_A: &str = "019fe15e-1072-7df1-884a-f6461b3f4fa7";
const ID_B: &str = "019fd3de-cc7f-7fc0-ba0c-9ffbd65ba54c";

#[test]
fn a_manifest_whose_tags_all_exist_is_processed() {
    let (dir, repo) = scaffold();
    let path = write_manifest(dir.path(), ID_A, &["gate-v0.1.1", "aura-v0.1.1"]);
    tag(dir.path(), "gate-v0.1.1");
    tag(dir.path(), "aura-v0.1.1");

    let found = find_processed_manifests(&repo, None);

    assert_eq!(found.len(), 1, "expected the released manifest");
    assert_eq!(found[0].path, path);
}

#[test]
fn a_partially_released_manifest_is_left_alone() {
    // The decisive safety property: one missing tag means the release did not
    // fully happen, so the request stands.
    let (dir, repo) = scaffold();
    write_manifest(dir.path(), ID_A, &["gate-v0.1.1", "aura-v0.1.1"]);
    tag(dir.path(), "gate-v0.1.1");

    assert!(find_processed_manifests(&repo, None).is_empty());
}

#[test]
fn no_tags_at_all_means_nothing_is_deleted() {
    // What a failed or incomplete tag fetch looks like. Fewer visible tags must
    // only ever mean "not done yet".
    let (dir, repo) = scaffold();
    write_manifest(dir.path(), ID_A, &["gate-v0.1.1"]);

    assert!(find_processed_manifests(&repo, None).is_empty());
}

#[test]
fn an_unparseable_manifest_is_left_alone() {
    // Could be a newer schema this build does not understand. Deleting it would
    // discard a release nobody performed.
    let (dir, repo) = scaffold();
    std::fs::write(
        dir.path().join(MANIFEST_DIR).join("broken.json"),
        "{ not json",
    )
    .expect("write");

    assert!(find_processed_manifests(&repo, None).is_empty());
}

#[test]
fn a_manifest_with_no_releases_is_left_alone() {
    // "Every tag exists" is vacuously true for an empty set; that must not be
    // read as permission to delete.
    let (dir, repo) = scaffold();
    write_manifest(dir.path(), ID_A, &[]);

    assert!(find_processed_manifests(&repo, None).is_empty());
}

#[test]
fn the_manifest_this_run_just_wrote_is_excluded() {
    let (dir, repo) = scaffold();
    let fresh = write_manifest(dir.path(), ID_A, &["gate-v0.1.1"]);
    tag(dir.path(), "gate-v0.1.1");

    assert!(find_processed_manifests(&repo, Some(&fresh)).is_empty());
}

#[test]
fn only_the_released_manifest_of_several_is_picked() {
    let (dir, repo) = scaffold();
    let done = write_manifest(dir.path(), ID_A, &["gate-v0.1.1"]);
    write_manifest(dir.path(), ID_B, &["writ-v0.9.9"]);
    tag(dir.path(), "gate-v0.1.1");

    let found = find_processed_manifests(&repo, None);

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].path, done);
    assert_eq!(found[0].tags, vec!["gate-v0.1.1".to_string()]);
}

#[test]
fn a_missing_manifest_directory_is_not_an_error() {
    let (dir, repo) = scaffold();
    std::fs::remove_dir_all(dir.path().join(MANIFEST_DIR)).expect("rm");

    assert!(find_processed_manifests(&repo, None).is_empty());
}
