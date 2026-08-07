//! When is a cargo workspace **one** release unit?
//!
//! `[workspace.package].version` on the root does not answer that. It declares
//! a value members *may* inherit, and Cargo lets each member decide:
//! `version.workspace = true` takes it, `version = "1.2.3"` opts out. Nearly
//! every modern workspace sets the key, so reading its mere presence as "this
//! workspace is a single project" collapses ordinary multi-crate repos into one
//! unit — a unit that owns no `[package]`, and whose name can only come from
//! the directory-name fallback, because `name` is not an inheritable field and
//! so can never appear under `[workspace.package]`.

mod common;

use common::TestRepo;

fn unit_names(repo: &TestRepo) -> Vec<String> {
    let out = repo.run_belaf_command_with_env(&["status", "--ci"], &[("BELAF_NO_KEYRING", "1")]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "`belaf status --ci` failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    // Tracing warnings share stdout with the `--ci` JSON payload, which starts
    // at the first line that is a bare `{`.
    let body_start = stdout
        .lines()
        .position(|l| l.trim() == "{")
        .unwrap_or_else(|| panic!("no JSON object in status output:\n{stdout}"));
    let body: String = stdout
        .lines()
        .skip(body_start)
        .collect::<Vec<_>>()
        .join("\n");
    let json: serde_json::Value =
        serde_json::from_str(&body).unwrap_or_else(|e| panic!("status JSON: {e}\n{body}"));
    let mut names: Vec<String> = json["projects"]
        .as_array()
        .expect("projects array")
        .iter()
        .filter_map(|p| p["name"].as_str().map(str::to_string))
        .collect();
    names.sort();
    names
}

fn commit_with_lockfile(repo: &TestRepo) {
    repo.commit("Initial commit");
    let lock = std::process::Command::new("cargo")
        .args(["generate-lockfile"])
        .current_dir(&repo.path)
        .output()
        .expect("cargo generate-lockfile");
    assert!(
        lock.status.success(),
        "generate-lockfile failed:\n{}",
        String::from_utf8_lossy(&lock.stderr)
    );
    repo.commit("chore: lockfile");
}

#[test]
fn members_that_pin_their_own_version_stay_separate_units() {
    // The clikd shape: the root offers an inheritable version, satellite crates
    // take it, but the released crates pin their own. These release
    // independently, so the workspace is not one project — and no unit named
    // after the repo directory may be invented.
    let repo = TestRepo::new();
    repo.write_file(
        "Cargo.toml",
        "[workspace.package]\nversion = \"0.2.0\"\nedition = \"2021\"\n\n\
         [workspace]\nresolver = \"2\"\nmembers = [\"crates/alpha\", \"crates/beta\"]\n",
    );
    repo.write_file(
        "crates/alpha/Cargo.toml",
        "[package]\nname = \"alpha\"\nversion = \"0.1.7\"\nedition.workspace = true\n",
    );
    repo.write_file("crates/alpha/src/lib.rs", "pub fn a() {}\n");
    repo.write_file(
        "crates/beta/Cargo.toml",
        "[package]\nname = \"beta\"\nversion = \"2.4.0\"\nedition.workspace = true\n",
    );
    repo.write_file("crates/beta/src/lib.rs", "pub fn b() {}\n");
    commit_with_lockfile(&repo);

    let names = unit_names(&repo);
    assert!(
        names.iter().any(|n| n == "alpha") && names.iter().any(|n| n == "beta"),
        "both members pin their own version and must be separate units; got {names:?}"
    );

    // The phantom: a unit named after the repo directory, backed by no
    // `[package]`, that nothing releases but that still trips the
    // untagged-unit guard.
    let repo_dir = repo
        .path
        .file_name()
        .and_then(|n| n.to_str())
        .expect("temp dir name")
        .to_string();
    assert!(
        !names.iter().any(|n| n == &repo_dir),
        "no unit may be invented from the workspace directory name `{repo_dir}`; got {names:?}"
    );
}

#[test]
fn a_workspace_whose_members_all_inherit_is_one_unit() {
    // The true positive: every member takes `version.workspace = true`, so they
    // can only ever carry one version. Emitting one unit per member would
    // produce N releases that are required to agree.
    let repo = TestRepo::new();
    repo.write_file(
        "Cargo.toml",
        "[workspace.package]\nversion = \"1.4.0\"\nedition = \"2021\"\n\n\
         [workspace]\nresolver = \"2\"\nmembers = [\"crates/alpha\", \"crates/beta\"]\n",
    );
    repo.write_file(
        "crates/alpha/Cargo.toml",
        "[package]\nname = \"alpha\"\nversion.workspace = true\nedition.workspace = true\n",
    );
    repo.write_file("crates/alpha/src/lib.rs", "pub fn a() {}\n");
    repo.write_file(
        "crates/beta/Cargo.toml",
        "[package]\nname = \"beta\"\nversion.workspace = true\nedition.workspace = true\n",
    );
    repo.write_file("crates/beta/src/lib.rs", "pub fn b() {}\n");
    commit_with_lockfile(&repo);

    let names = unit_names(&repo);
    assert_eq!(
        names.len(),
        1,
        "a fully-inheriting workspace is one release unit; got {names:?}"
    );
    assert!(
        !names.iter().any(|n| n == "alpha" || n == "beta"),
        "members of a single-project workspace are not units of their own; got {names:?}"
    );
}

#[test]
fn one_pinned_member_is_enough_to_split_the_workspace() {
    // The boundary: all-but-one inherit. That one member releases on its own
    // schedule, so the workspace cannot be modelled as a single version.
    let repo = TestRepo::new();
    repo.write_file(
        "Cargo.toml",
        "[workspace.package]\nversion = \"1.4.0\"\nedition = \"2021\"\n\n\
         [workspace]\nresolver = \"2\"\nmembers = [\"crates/alpha\", \"crates/beta\"]\n",
    );
    repo.write_file(
        "crates/alpha/Cargo.toml",
        "[package]\nname = \"alpha\"\nversion.workspace = true\nedition.workspace = true\n",
    );
    repo.write_file("crates/alpha/src/lib.rs", "pub fn a() {}\n");
    repo.write_file(
        "crates/beta/Cargo.toml",
        "[package]\nname = \"beta\"\nversion = \"9.9.9\"\nedition.workspace = true\n",
    );
    repo.write_file("crates/beta/src/lib.rs", "pub fn b() {}\n");
    commit_with_lockfile(&repo);

    let names = unit_names(&repo);
    assert!(
        names.iter().any(|n| n == "alpha") && names.iter().any(|n| n == "beta"),
        "a single pinned member splits the workspace into per-member units; got {names:?}"
    );
}
