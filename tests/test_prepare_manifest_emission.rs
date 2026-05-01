//! DoD #2 — `belaf prepare --ci` against a configured ReleaseUnit
//! emits a manifest under `belaf/releases/<uuid>.json` carrying the
//! correct bump per the conventional-commit type.
//!
//! We exercise the four canonical paths from the master plan:
//!   - feat in a service crate     → MINOR
//!   - feat! anywhere               → MAJOR
//!   - fix anywhere                 → PATCH
//!   - chore (non-conventional)     → no manifest
//!
//! Each test runs the actual `belaf` binary; `prepare --ci` will
//! eventually fail at the push/PR step (no network, no auth), but
//! by then the manifest has already been emitted to disk. We assert
//! on the manifest, not on the exit code.

mod common;

use common::TestRepo;
use std::path::Path;

fn manifest_files(repo: &TestRepo) -> Vec<std::path::PathBuf> {
    let dir = repo.path.join("belaf").join("releases");
    if !dir.exists() {
        return Vec::new();
    }
    std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
                .collect()
        })
        .unwrap_or_default()
}

fn read_manifest(path: &Path) -> serde_json::Value {
    let s = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("read manifest at {}: {e}", path.display()));
    serde_json::from_str(&s)
        .unwrap_or_else(|e| panic!("parse manifest at {}: {e}\ncontent:\n{s}", path.display()))
}

fn release_bump(manifest: &serde_json::Value, name: &str) -> Option<String> {
    let releases = manifest.get("releases")?.as_array()?;
    for r in releases {
        if r.get("name").and_then(|v| v.as_str()) == Some(name) {
            return r
                .get("bump_type")
                .and_then(|v| v.as_str())
                .map(String::from);
        }
    }
    None
}

fn release_new_version(manifest: &serde_json::Value, name: &str) -> Option<String> {
    let releases = manifest.get("releases")?.as_array()?;
    for r in releases {
        if r.get("name").and_then(|v| v.as_str()) == Some(name) {
            return r
                .get("new_version")
                .and_then(|v| v.as_str())
                .map(String::from);
        }
    }
    None
}

fn seed_single_crate(repo: &TestRepo, name: &str, version: &str) {
    repo.write_file(
        "Cargo.toml",
        &format!("[package]\nname = \"{name}\"\nversion = \"{version}\"\nedition = \"2021\"\n"),
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
    repo.commit("Initial commit");

    let out =
        repo.run_belaf_command_with_env(&["init", "--ci", "--force"], &[("BELAF_NO_KEYRING", "1")]);
    assert!(
        out.status.success(),
        "init must succeed; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    repo.commit("chore: bootstrap belaf");
}

#[test]
fn fix_commit_emits_patch_manifest() {
    let repo = TestRepo::new();
    seed_single_crate(&repo, "lib-x", "1.0.0");

    repo.write_file("src/fix.rs", "pub fn fix_bug() {}\n");
    repo.commit("fix: address a regression in hello()");

    // Push/PR will fail on the bare TestRepo — but the manifest is
    // emitted before then, so we can inspect it after.
    let _ = repo.run_belaf_command_with_env(&["prepare", "--ci"], &[("BELAF_NO_KEYRING", "1")]);

    let manifests = manifest_files(&repo);
    assert!(
        !manifests.is_empty(),
        "fix commit must produce a manifest in belaf/releases/"
    );
    let manifest = read_manifest(&manifests[0]);
    assert_eq!(
        release_bump(&manifest, "lib-x").as_deref(),
        Some("patch"),
        "fix commit must drive `bump_type = \"patch\"`; manifest:\n{manifest:#}"
    );
    assert_eq!(
        release_new_version(&manifest, "lib-x").as_deref(),
        Some("1.0.1"),
        "patch bump on 1.0.0 must yield 1.0.1; manifest:\n{manifest:#}"
    );
}

#[test]
fn feat_commit_emits_minor_manifest() {
    let repo = TestRepo::new();
    seed_single_crate(&repo, "lib-y", "1.0.0");

    repo.write_file("src/feat.rs", "pub fn new_thing() {}\n");
    repo.commit("feat: add new_thing()");

    let _ = repo.run_belaf_command_with_env(&["prepare", "--ci"], &[("BELAF_NO_KEYRING", "1")]);

    let manifests = manifest_files(&repo);
    assert!(!manifests.is_empty(), "feat commit must emit a manifest");
    let manifest = read_manifest(&manifests[0]);
    assert_eq!(
        release_bump(&manifest, "lib-y").as_deref(),
        Some("minor"),
        "feat must drive minor bump; manifest:\n{manifest:#}"
    );
    assert_eq!(
        release_new_version(&manifest, "lib-y").as_deref(),
        Some("1.1.0"),
    );
}

#[test]
fn breaking_commit_emits_major_manifest() {
    let repo = TestRepo::new();
    seed_single_crate(&repo, "lib-z", "1.0.0");

    repo.write_file("src/break.rs", "pub fn changed_signature() {}\n");
    repo.commit("feat!: rename hello() → greet()");

    let _ = repo.run_belaf_command_with_env(&["prepare", "--ci"], &[("BELAF_NO_KEYRING", "1")]);

    let manifests = manifest_files(&repo);
    assert!(
        !manifests.is_empty(),
        "breaking commit must emit a manifest"
    );
    let manifest = read_manifest(&manifests[0]);
    assert_eq!(
        release_bump(&manifest, "lib-z").as_deref(),
        Some("major"),
        "feat! must drive major bump; manifest:\n{manifest:#}"
    );
    assert_eq!(
        release_new_version(&manifest, "lib-z").as_deref(),
        Some("2.0.0"),
    );
}

#[test]
fn chore_commit_emits_no_manifest() {
    let repo = TestRepo::new();
    seed_single_crate(&repo, "lib-w", "1.0.0");

    // chore is conventional but not a release-driving type
    repo.write_file("src/chore.rs", "// comment only\n");
    repo.commit("chore: clean up");

    let _ = repo.run_belaf_command_with_env(&["prepare", "--ci"], &[("BELAF_NO_KEYRING", "1")]);

    let manifests = manifest_files(&repo);
    assert!(
        manifests.is_empty(),
        "chore commit must NOT produce a manifest; got: {manifests:?}"
    );
}
