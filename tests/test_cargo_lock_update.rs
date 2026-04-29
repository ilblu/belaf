//! Integration test: bump a Cargo crate's version and verify the
//! workspace `Cargo.lock` got the new version recorded too. Pins
//! the wiring added in Phase J — without this, `belaf prepare`
//! would commit a `Cargo.toml` whose version doesn't match the
//! adjacent lockfile, breaking reproducible builds.
//!
//! The test gracefully self-skips when `cargo` isn't available in
//! the test environment (CI workflows may run with a stripped
//! toolchain).

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
fn cargo_update_for_crate_refreshes_lockfile() {
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
    assert!(
        lockfile.exists(),
        "Cargo.lock should exist after generate-lockfile"
    );
    assert_eq!(
        read_lockfile_version(&lockfile, "alpha"),
        Some("0.1.0".to_string()),
        "lockfile must record the initial version"
    );

    // Hand-bump the manifest the way CargoRewriter would.
    let bumped = "[package]\nname = \"alpha\"\nversion = \"0.2.0\"\nedition = \"2021\"\n";
    std::fs::write(repo.path.join("Cargo.toml"), bumped).expect("write Cargo.toml");

    // Now drive the same code path CargoRewriter::rewrite calls.
    belaf::core::cargo_lock::update_for_crate("alpha", &repo.path)
        .expect("cargo_lock::update_for_crate must succeed");

    assert_eq!(
        read_lockfile_version(&lockfile, "alpha"),
        Some("0.2.0".to_string()),
        "lockfile version must follow the manifest after update_for_crate"
    );
}

#[test]
fn cargo_update_for_crate_unknown_name_falls_back_to_workspace() {
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
    let _ = Command::new("cargo")
        .args(["generate-lockfile"])
        .current_dir(&repo.path)
        .output();

    if !repo.path.join("Cargo.lock").exists() {
        eprintln!("cargo generate-lockfile failed, skipping");
        return;
    }

    // Unknown package name → cargo update -p errors → workspace fallback succeeds.
    let r = belaf::core::cargo_lock::update_for_crate("not-a-real-crate", &repo.path);
    assert!(
        r.is_ok(),
        "workspace fallback should swallow the unknown -p error, got {r:?}"
    );
}
