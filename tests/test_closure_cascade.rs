//! F2/F4 — dependency-closure cascade end-to-end.
//!
//! A change to an `internal` library crate must bump the `deploy` services
//! whose dependency closure contains it, while the internal crate itself is
//! never versioned/tagged/released. A `[codegen_edges]` source behaves the
//! same way for crates that consume generated code.

mod common;

use common::TestRepo;

fn manifest_json(repo: &TestRepo) -> serde_json::Value {
    let dir = repo.path.join("belaf/releases");
    let entry = std::fs::read_dir(&dir)
        .unwrap_or_else(|_| panic!("no belaf/releases dir at {}", dir.display()))
        .filter_map(|e| e.ok())
        .find(|e| e.path().extension().is_some_and(|x| x == "json"))
        .expect("a release manifest json must exist");
    let body = std::fs::read_to_string(entry.path()).unwrap();
    serde_json::from_str(&body).expect("manifest must be valid json")
}

fn released_names(manifest: &serde_json::Value) -> Vec<String> {
    manifest["releases"]
        .as_array()
        .map(|rs| {
            rs.iter()
                .filter_map(|r| r["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Workspace: deploy bin `hata` depends (cargo path-dep) on internal lib
/// `clikd-migrate`. `clikd-migrate` is marked `kind = "internal"`.
fn scaffold_service_and_internal_lib(repo: &TestRepo) {
    repo.write_file(
        "Cargo.toml",
        "[workspace]\nmembers = [\"packages/migrate\", \"apps/services/hata\"]\nresolver = \"2\"\n",
    );
    repo.write_file(
        "packages/migrate/Cargo.toml",
        "[package]\nname = \"clikd-migrate\"\nversion = \"1.0.0\"\nedition = \"2021\"\n",
    );
    repo.write_file("packages/migrate/src/lib.rs", "pub fn migrate() {}\n");
    repo.write_file(
        "apps/services/hata/Cargo.toml",
        "[package]\nname = \"hata\"\nversion = \"1.0.0\"\nedition = \"2021\"\n\n[dependencies]\nclikd-migrate = { path = \"../../../packages/migrate\" }\n",
    );
    repo.write_file(
        "apps/services/hata/src/main.rs",
        "fn main() { clikd_migrate::migrate(); }\n",
    );
    // Mark the library crate as an internal cascade node (no version/tag/release).
    repo.write_file(
        "belaf/config.toml",
        "[release_unit.clikd-migrate]\nkind = \"internal\"\n",
    );
    repo.commit("Initial commit");
    // Generate + commit Cargo.lock so a later `prepare` (which runs
    // `cargo metadata`) sees a clean working tree.
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
    // Real release tag so prepare's window excludes the seed commits.
    repo.tag("hata-v1.0.0");
}

#[test]
fn internal_lib_change_bumps_dependent_service_not_itself() {
    let repo = TestRepo::new();
    scaffold_service_and_internal_lib(&repo);

    // A fix landing only in the internal library crate.
    repo.write_file("packages/migrate/src/fixed.rs", "pub fn fixed() {}\n");
    repo.commit("fix(clikd-migrate): correct a migration");

    // The manifest is written before the (auth-gated) PR step; like the other
    // prepare tests, we ignore the exit status and assert on the manifest file.
    let _ = repo.run_belaf_command_with_env(&["prepare", "--ci"], &[("BELAF_NO_KEYRING", "1")]);

    let manifest = manifest_json(&repo);
    let names = released_names(&manifest);

    // F2 — the deploy service bumps because the changed internal crate is in
    // its dependency closure.
    assert!(
        names.iter().any(|n| n == "hata"),
        "hata must bump when its internal dep changed; released: {names:?}"
    );
    // F1 — the internal crate is a pure cascade node: never released.
    assert!(
        !names.iter().any(|n| n == "clikd-migrate"),
        "internal crate must NOT be released; released: {names:?}"
    );

    // F7 — the propagated commit is marked with its origin crate in hata's
    // changelog (`via clikd-migrate — …`).
    let hata = manifest["releases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "hata")
        .expect("hata release");
    let changelog = hata["changelog"].as_str().unwrap_or("");
    assert!(
        changelog.contains("via clikd-migrate"),
        "hata changelog must attribute the closure-propagated commit to its origin crate; got:\n{changelog}"
    );
}

#[test]
fn scope_does_not_drive_bump_only_paths_do() {
    // F-decouple (Risk 7, HIGH) — a commit whose conventional SCOPE names unit
    // `api` but whose binary-affecting PATHS touch only `web` must bump `web`,
    // NOT `api`. Under the old model scope drove WHETHER and would have bumped
    // `api`; this is the silent-bug case that passes tests checking only
    // path-based units.
    let repo = TestRepo::new();
    repo.write_file(
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/*\"]\nresolver = \"2\"\n",
    );
    repo.write_file(
        "crates/api/Cargo.toml",
        "[package]\nname = \"api\"\nversion = \"1.0.0\"\nedition = \"2021\"\n",
    );
    repo.write_file("crates/api/src/lib.rs", "pub fn api() {}\n");
    repo.write_file(
        "crates/web/Cargo.toml",
        "[package]\nname = \"web\"\nversion = \"1.0.0\"\nedition = \"2021\"\n",
    );
    repo.write_file("crates/web/src/lib.rs", "pub fn web() {}\n");
    repo.commit("Initial commit");
    let lock = std::process::Command::new("cargo")
        .args(["generate-lockfile"])
        .current_dir(&repo.path)
        .output()
        .expect("cargo generate-lockfile");
    assert!(lock.status.success());
    repo.commit("chore: lockfile");
    repo.tag("api-v1.0.0");
    repo.tag("web-v1.0.0");

    // Scope says `api`, but the change touches only web's source.
    repo.write_file("crates/web/src/feature.rs", "pub fn feature() {}\n");
    repo.commit("feat(api): scoped api but only touches web");

    let _ = repo.run_belaf_command_with_env(&["prepare", "--ci"], &[("BELAF_NO_KEYRING", "1")]);

    let names = released_names(&manifest_json(&repo));
    assert!(
        names.iter().any(|n| n == "web"),
        "web (the path-touched unit) must bump; released: {names:?}"
    );
    assert!(
        !names.iter().any(|n| n == "api"),
        "api (scope-only, no binary-affecting path change) must NOT bump — scope does \
         not drive WHETHER; released: {names:?}"
    );
}
