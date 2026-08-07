//! "Explicit wins": a directory the user declared in its own
//! `[release_unit.<name>]` block must not also be claimed — or complained
//! about — by a glob that happens to match it.
//!
//! The shadow check existed but ran too late and looked at too little: it
//! only saw manifest and satellite paths, so a `paths = [...]` unit (the only
//! shape `kind = "ignore"` can take) shadowed nothing, and a soft-skipped
//! glob match was warned about before the check ran at all.

//! These assert on resolver *diagnostics*, not on the command's exit code: a
//! glob-form unit over auto-discovered cargo crates currently aborts later
//! with "multiple projects with same name", a separate pre-existing bug. The
//! resolve step — and therefore the behaviour under test — runs first.

mod common;

use common::TestRepo;

/// A directory the user declared explicitly must not be re-reported by a glob
/// that happens to match it too.
///
/// Reported as a cosmetic warning, but the cause was a real coverage hole:
/// `unit_paths` collected only manifest and satellite paths, so a
/// `paths = [...]` unit — the only shape `kind = "ignore"` can take —
/// contributed nothing to the covered set and shadowed nothing at all.
#[test]
fn explicit_ignore_unit_shadows_a_glob_match_silently() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        "[workspace]\nmembers = [\"apps/services/gate\"]\nresolver = \"2\"\n",
    );
    repo.write_file(
        "apps/services/gate/Cargo.toml",
        "[package]\nname = \"gate\"\nversion = \"1.0.0\"\nedition = \"2021\"\n",
    );
    repo.write_file("apps/services/gate/src/lib.rs", "pub fn gate() {}\n");
    // A flat test crate under the same glob, carrying none of the manifests
    // the glob template expects — and explicitly declared as ignored.
    repo.write_file("apps/services/e2e/README.md", "harness\n");

    repo.write_file(
        "belaf/config.toml",
        r#"[release_unit.services]
glob = "apps/services/*"
name = "{basename}"
ecosystem = "cargo"
manifests = ["{path}/Cargo.toml"]

[release_unit.e2e]
kind = "ignore"
paths = ["apps/services/e2e"]
"#,
    );
    repo.commit("Initial commit");

    let output = repo.run_belaf_command(&["status", "--ci", "--format=json"]);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        !combined.contains("has none of the expected manifests"),
        "an explicitly declared path must not be warned about; output:\n{combined}"
    );
}

/// The warning still has to fire for a directory nobody declared — that signal
/// is how a real service silently dropping out of releases stays visible.
#[test]
fn undeclared_glob_match_without_manifests_still_warns() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        "[workspace]\nmembers = [\"apps/services/gate\"]\nresolver = \"2\"\n",
    );
    repo.write_file(
        "apps/services/gate/Cargo.toml",
        "[package]\nname = \"gate\"\nversion = \"1.0.0\"\nedition = \"2021\"\n",
    );
    repo.write_file("apps/services/gate/src/lib.rs", "pub fn gate() {}\n");
    repo.write_file("apps/services/orphan/README.md", "no manifest here\n");

    repo.write_file(
        "belaf/config.toml",
        r#"[release_unit.services]
glob = "apps/services/*"
name = "{basename}"
ecosystem = "cargo"
manifests = ["{path}/Cargo.toml"]
"#,
    );
    repo.commit("Initial commit");

    let output = repo.run_belaf_command(&["status", "--ci", "--format=json"]);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        combined.contains("has none of the expected manifests"),
        "an undeclared manifest-less match must stay visible; output:\n{combined}"
    );
}
