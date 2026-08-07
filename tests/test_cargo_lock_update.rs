//! `Cargo.lock` has to leave `prepare` in step with the manifests it bumped —
//! and it has to leave in the release commit.
//!
//! A commit carrying new manifest versions against an unchanged lockfile does
//! not just break reproducible builds: the next checkout regenerates the lock,
//! finds a modified working tree, and `prepare` refuses to run in a dirty tree.
//! That is a permanent stop until somebody syncs the lock by hand.
//!
//! These tests self-skip when `cargo` isn't available in the test environment
//! (CI workflows may run with a stripped toolchain).

use std::path::Path;
use std::process::Command;

mod common;
use common::TestRepo;

fn cargo_available() -> bool {
    Command::new("cargo")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn read_lockfile_version(lockfile: &Path, crate_name: &str) -> Option<String> {
    let s = std::fs::read_to_string(lockfile).ok()?;
    // Toml parsing is overkill; the format is line-based:
    //   [[package]]
    //   name = "<crate_name>"
    //   version = "0.1.0"
    //
    // Limitation: returns the FIRST matching `[[package]]` entry. If
    // a workspace had multiple versions of the same crate (e.g.
    // `serde 1.0.x` and `serde 1.0.y` pulled in by different deps)
    // this would only see the first. Sufficient for the single-crate
    // fixture below; broaden if a multi-version test ever lands.
    let mut found_name = false;
    for line in s.lines() {
        let trimmed = line.trim();
        if trimmed == "[[package]]" {
            found_name = false;
        }
        if trimmed == format!("name = \"{crate_name}\"") {
            found_name = true;
            continue;
        }
        if found_name {
            if let Some(rest) = trimmed.strip_prefix("version = \"") {
                if let Some(end) = rest.find('"') {
                    return Some(rest[..end].to_string());
                }
            }
        }
    }
    None
}

#[test]
fn update_workspace_refreshes_lockfile() {
    if !cargo_available() {
        eprintln!("cargo unavailable in this test env, skipping");
        return;
    }

    let repo = TestRepo::new();
    repo.write_file(
        "Cargo.toml",
        "[package]\nname = \"alpha\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    repo.write_file("src/lib.rs", "pub fn x() {}\n");

    // Generate the initial lockfile.
    let gen = Command::new("cargo")
        .args(["generate-lockfile"])
        .current_dir(&repo.path)
        .output()
        .expect("cargo generate-lockfile");
    if !gen.status.success() {
        eprintln!(
            "cargo generate-lockfile failed; skipping. stderr:\n{}",
            String::from_utf8_lossy(&gen.stderr)
        );
        return;
    }

    let lockfile = repo.path.join("Cargo.lock");
    assert_eq!(
        read_lockfile_version(&lockfile, "alpha"),
        Some("0.1.0".to_string()),
        "lockfile must record the initial version"
    );

    // Hand-bump the manifest the way a rewriter would.
    let bumped = "[package]\nname = \"alpha\"\nversion = \"0.2.0\"\nedition = \"2021\"\n";
    std::fs::write(repo.path.join("Cargo.toml"), bumped).expect("write Cargo.toml");

    belaf::core::cargo_lock::update_workspace(&repo.path)
        .expect("cargo_lock::update_workspace must succeed");

    assert_eq!(
        read_lockfile_version(&lockfile, "alpha"),
        Some("0.2.0".to_string()),
        "lockfile version must follow the manifest"
    );
}

/// A `Cargo.lock` the repo deliberately gitignores is not part of the release.
///
/// Published libraries commonly ignore the lockfile while still having one on
/// disk from local builds. `libgit2`'s `add_path` stages files past the ignore
/// rules, so without an explicit tracked-check the release commit would carry a
/// file the repo went out of its way not to version.
#[test]
fn an_ignored_lockfile_is_left_alone() {
    if !cargo_available() {
        eprintln!("cargo unavailable in this test env, skipping");
        return;
    }

    let repo = TestRepo::new();
    repo.write_file(
        "Cargo.toml",
        "[package]\nname = \"alpha\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    repo.write_file("src/lib.rs", "pub fn x() {}\n");
    repo.write_file(".gitignore", "/Cargo.lock\n");
    repo.write_file("belaf/config.toml", "");
    repo.commit("Initial commit");

    // A lockfile exists on disk but is untracked and ignored.
    let gen = Command::new("cargo")
        .args(["generate-lockfile"])
        .current_dir(&repo.path)
        .output()
        .expect("cargo generate-lockfile");
    if !gen.status.success() || !repo.path.join("Cargo.lock").exists() {
        eprintln!("cargo generate-lockfile failed, skipping");
        return;
    }
    repo.tag("alpha-v0.1.0");

    repo.write_file("src/feature.rs", "pub fn feature() {}\n");
    repo.commit("feat: add a feature");

    let out = repo.run_belaf_command_with_env(&["prepare", "--ci"], &[("BELAF_NO_KEYRING", "1")]);
    let stderr = String::from_utf8_lossy(&out.stderr);

    let committed = repo.git(&["show", "--name-only", "--format=", "HEAD"]);
    assert!(
        committed.lines().any(|l| l.trim() == "Cargo.toml"),
        "the release should still have happened; commit contained:\n{committed}\n\
         stderr:\n{stderr}"
    );
    assert!(
        !committed.lines().any(|l| l.trim() == "Cargo.lock"),
        "an ignored Cargo.lock must never be staged into the release commit; \
         commit contained:\n{committed}"
    );
}

/// The failure that shut clikd's releases down for days.
///
/// `prepare` bumps the manifests and refreshes `Cargo.lock` in the same run.
/// When the refresh silently failed, the commit carried new manifest versions
/// against an old lockfile — so the next CI checkout regenerated the lock, found
/// a modified working tree, and `prepare` refused to run. Every subsequent
/// release attempt died on `requires a clean working directory: Cargo.lock`.
///
/// This pins the invariant that prevents it: after `prepare`, regenerating the
/// lockfile changes nothing.
#[test]
fn prepare_leaves_the_lockfile_in_step_with_the_bumped_manifests() {
    if !cargo_available() {
        eprintln!("cargo unavailable in this test env, skipping");
        return;
    }

    let repo = TestRepo::new();
    repo.write_file(
        "Cargo.toml",
        "[workspace]\nresolver = \"2\"\nmembers = [\"packages/observability\", \
         \"apps/services/gate/crates/bin\"]\n",
    );
    // The unit is named `observability` after its directory; the crate is
    // `clikd-observability`. `cargo update -p observability` matches nothing —
    // the name has to come from the manifest, not from the unit.
    repo.write_file(
        "packages/observability/Cargo.toml",
        "[package]\nname = \"clikd-observability\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    repo.write_file("packages/observability/src/lib.rs", "pub fn init() {}\n");
    repo.write_file(
        "apps/services/gate/crates/bin/Cargo.toml",
        "[package]\nname = \"gate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
         [dependencies]\nclikd-observability = { path = \"../../../../../packages/observability\" }\n",
    );
    repo.write_file(
        "apps/services/gate/crates/bin/src/main.rs",
        "fn main() { clikd_observability::init(); }\n",
    );
    repo.write_file(
        "belaf/config.toml",
        "[release_unit.packages]\n\
         ecosystem = \"cargo\"\n\
         glob = \"packages/*\"\n\
         name = \"{basename}\"\n\
         manifests = [\"{path}/Cargo.toml\"]\n\
         \n\
         [release_unit.services]\n\
         ecosystem = \"cargo\"\n\
         glob = \"apps/services/*\"\n\
         name = \"{basename}\"\n\
         manifests = [\"{path}/crates/bin/Cargo.toml\"]\n\
         satellites = [\"{path}/crates\"]\n",
    );
    repo.commit("Initial commit");

    let gen = Command::new("cargo")
        .args(["generate-lockfile"])
        .current_dir(&repo.path)
        .output()
        .expect("cargo generate-lockfile");
    if !gen.status.success() {
        eprintln!(
            "cargo generate-lockfile failed; skipping. stderr:\n{}",
            String::from_utf8_lossy(&gen.stderr)
        );
        return;
    }
    repo.commit("chore: lockfile");
    repo.tag("observability-v0.1.0");
    repo.tag("gate-v0.1.0");

    repo.write_file(
        "packages/observability/src/otlp.rs",
        "pub fn headers() {}\n",
    );
    repo.commit("fix(observability): send the OTLP headers");

    let out = repo.run_belaf_command_with_env(&["prepare", "--ci"], &[("BELAF_NO_KEYRING", "1")]);
    let stderr = String::from_utf8_lossy(&out.stderr);

    let lockfile = repo.path.join("Cargo.lock");
    assert_eq!(
        read_lockfile_version(&lockfile, "clikd-observability"),
        Some("0.1.1".to_string()),
        "the lockfile must carry the bumped version of the crate whose unit is \
         named after its directory, not its package\nstderr:\n{stderr}"
    );

    // Refreshing the file on disk is only half of it: if the lockfile is left
    // out of the release commit, the PR still merges bumped manifests against
    // the old lock and the next run is blocked exactly as before.
    let committed = repo.git(&["show", "--name-only", "--format=", "HEAD"]);
    assert!(
        committed.lines().any(|l| l.trim() == "Cargo.lock"),
        "the refreshed Cargo.lock must be part of the release commit; commit \
         contained:\n{committed}"
    );
    assert!(
        repo.git(&["status", "--porcelain"]).is_empty(),
        "`prepare` must leave no uncommitted changes behind; got:\n{}",
        repo.git(&["status", "--porcelain"])
    );

    // The invariant that actually matters, stated the way cargo states it:
    // `--locked` fails if the lockfile would have to change to satisfy the
    // manifests. That is precisely the condition under which the next CI
    // checkout dirties the tree and `prepare` refuses to run.
    //
    // (Not `cargo generate-lockfile` — that re-resolves external dependencies
    // from scratch and churns for reasons unrelated to the release.)
    let locked = Command::new("cargo")
        .args(["metadata", "--locked", "--format-version", "1"])
        .current_dir(&repo.path)
        .output()
        .expect("cargo metadata --locked");
    assert!(
        locked.status.success(),
        "Cargo.lock does not satisfy the bumped manifests — the next checkout \
         would rewrite it and leave a dirty tree:\n{}",
        String::from_utf8_lossy(&locked.stderr)
    );
    assert!(
        repo.git(&["status", "--porcelain"]).is_empty(),
        "resolving against the lockfile must not modify it; got:\n{}",
        repo.git(&["status", "--porcelain"])
    );
}
