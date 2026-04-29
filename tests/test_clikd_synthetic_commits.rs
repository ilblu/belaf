//! M.2-M.5 — synthetic conventional commits against the clikd-shape
//! fixture. Each test seeds the fixture, makes ONE conventional commit
//! that touches a specific path, and asserts:
//!
//!   1. `git log -- <path>` includes the new commit (path filter works)
//!   2. `bump::analyze_commit_messages` on that commit's message
//!      returns the expected BumpRecommendation
//!
//! Together this models the loop the prepare pipeline runs per
//! ReleaseUnit: collect the commits below the unit's coverage path,
//! infer the bump from their conventional-commit types.

mod common;
mod fixtures;

use std::path::Path;
use std::process::Command;

use belaf::core::bump::{self, BumpRecommendation};

use common::TestRepo;
use fixtures::Seedable;

impl Seedable for TestRepo {
    fn root(&self) -> &Path {
        &self.path
    }
    fn write_file(&self, relative: &str, content: &str) {
        TestRepo::write_file(self, relative, content);
    }
    fn commit(&self, message: &str) {
        TestRepo::commit(self, message);
    }
}

fn add_and_commit(repo: &TestRepo, message: &str) {
    Command::new("git")
        .args(["add", "-A"])
        .current_dir(&repo.path)
        .output()
        .expect("git add");
    Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(&repo.path)
        .output()
        .expect("git commit");
}

fn commits_touching_path(repo: &TestRepo, path: &str) -> Vec<String> {
    let out = Command::new("git")
        .args(["log", "--format=%s", "--", path])
        .current_dir(&repo.path)
        .output()
        .expect("git log");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|s| s.to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// M.2 — aura/crates/core/ — `feat(aura): ...` should drive a MINOR bump
// ---------------------------------------------------------------------------

#[test]
fn synthetic_feat_in_aura_drives_minor_bump() {
    let repo = TestRepo::new();
    fixtures::seed_clikd_shape(&repo);

    repo.write_file(
        "apps/services/aura/crates/core/Cargo.toml",
        "[package]\nname = \"aura-core\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    repo.write_file(
        "apps/services/aura/crates/core/src/lib.rs",
        "pub fn new_feature() {}\n",
    );
    let message = "feat(aura): add core domain handler";
    add_and_commit(&repo, message);

    let commits = commits_touching_path(&repo, "apps/services/aura/crates/core/");
    assert!(
        commits.iter().any(|s| s.starts_with("feat(aura):")),
        "expected commit visible under aura/crates/core/, got: {commits:?}"
    );

    let rec = bump::recommend_bump_for_commits(&[message.to_string()]).expect("bump inference");
    assert_eq!(
        rec,
        BumpRecommendation::Minor,
        "feat(...) must drive MINOR for the aura release unit"
    );
}

// ---------------------------------------------------------------------------
// M.3 — proto/events/v1/ — `feat!: ...` should drive a MAJOR bump
//   (breaking schema change cascades into every SDK)
// ---------------------------------------------------------------------------

#[test]
fn synthetic_breaking_feat_in_proto_drives_major_bump() {
    let repo = TestRepo::new();
    fixtures::seed_clikd_shape(&repo);

    repo.write_file(
        "proto/events/v1/schema.graphql",
        "type Event { id: ID!, name: String! }\n",
    );
    let message = "feat(proto)!: rename field name → display_name";
    add_and_commit(&repo, message);

    let commits = commits_touching_path(&repo, "proto/events/v1/");
    assert!(
        commits.iter().any(|s| s.starts_with("feat(proto)!:")),
        "expected breaking commit visible under proto/events/v1/, got: {commits:?}"
    );

    let rec = bump::recommend_bump_for_commits(&[message.to_string()]).expect("bump inference");
    assert_eq!(
        rec,
        BumpRecommendation::Major,
        "`feat!:` must drive MAJOR for the schema release unit"
    );
}

// ---------------------------------------------------------------------------
// M.4 — desktop/src-tauri/ — `fix(desktop): ...` should drive a PATCH bump
//   (Tauri triplet — three manifests bump in lockstep)
// ---------------------------------------------------------------------------

#[test]
fn synthetic_fix_in_desktop_drives_patch_bump() {
    let repo = TestRepo::new();
    fixtures::seed_clikd_shape(&repo);

    repo.write_file(
        "apps/desktop/src-tauri/src/main.rs",
        "fn main() {\n  // bug fix\n}\n",
    );
    let message = "fix(desktop): close window handle on quit";
    add_and_commit(&repo, message);

    let commits = commits_touching_path(&repo, "apps/desktop/");
    assert!(
        commits.iter().any(|s| s.starts_with("fix(desktop):")),
        "expected fix commit visible under apps/desktop/, got: {commits:?}"
    );

    let rec = bump::recommend_bump_for_commits(&[message.to_string()]).expect("bump inference");
    assert_eq!(
        rec,
        BumpRecommendation::Patch,
        "fix(...) must drive PATCH for the desktop release unit"
    );
}

// ---------------------------------------------------------------------------
// M.5 — sdks/kotlin/src/main/ — `feat(sdk): ...` should drive a MINOR bump
//   (JVM library, gradle.properties version source)
// ---------------------------------------------------------------------------

#[test]
fn synthetic_feat_in_kotlin_sdk_drives_minor_bump() {
    let repo = TestRepo::new();
    fixtures::seed_clikd_shape(&repo);

    repo.write_file(
        "sdks/kotlin/src/main/kotlin/Client.kt",
        "package com.clikd\nclass Client\n",
    );
    let message = "feat(sdk): add reactive Client wrapper";
    add_and_commit(&repo, message);

    let commits = commits_touching_path(&repo, "sdks/kotlin/");
    assert!(
        commits.iter().any(|s| s.starts_with("feat(sdk):")),
        "expected feat commit visible under sdks/kotlin/, got: {commits:?}"
    );

    let rec = bump::recommend_bump_for_commits(&[message.to_string()]).expect("bump inference");
    assert_eq!(
        rec,
        BumpRecommendation::Minor,
        "feat(...) must drive MINOR for the kotlin SDK release unit"
    );
}

// ---------------------------------------------------------------------------
// Cross-cutting: a commit that touches one unit must NOT show up in
// another unit's path filter. Pins the "no spillover" contract.
// ---------------------------------------------------------------------------

#[test]
fn aura_commit_does_not_leak_into_desktop_path() {
    let repo = TestRepo::new();
    fixtures::seed_clikd_shape(&repo);

    repo.write_file(
        "apps/services/aura/crates/bin/src/main.rs",
        "fn main() { println!(\"changed\"); }\n",
    );
    add_and_commit(&repo, "feat(aura): change main message");

    let desktop_commits = commits_touching_path(&repo, "apps/desktop/");
    assert!(
        !desktop_commits.iter().any(|s| s.contains("feat(aura)")),
        "aura commit must not appear in desktop path filter, got: {desktop_commits:?}"
    );

    let aura_commits = commits_touching_path(&repo, "apps/services/aura/");
    assert!(
        aura_commits.iter().any(|s| s.contains("feat(aura)")),
        "aura commit must appear in its own path filter, got: {aura_commits:?}"
    );
}
