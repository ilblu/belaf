//! The four ways a JavaScript monorepo says "these are my packages", and the
//! two dependency protocols belaf must not touch.
//!
//! npm, yarn and bun all put a `workspaces` array in the root `package.json`.
//! pnpm does not: it uses a sibling `pnpm-workspace.yaml`, and the root
//! `package.json` in a pnpm repo often has no `workspaces` key at all. Reading
//! only `package.json` therefore finds *nothing* in a pnpm repo, which is a
//! quieter failure than finding the wrong thing.
//!
//! Bun and pnpm additionally share two specs that look like versions and are
//! not: `workspace:*` and `catalog:`. Both name a place to resolve from, and
//! overwriting either with a number sends the install to a registry that, for
//! a `"private": true` package, has nothing to give it.

mod common;

use common::TestRepo;

/// A member listing, whatever the manager, should produce the same units.
fn assert_units(repo: &TestRepo, expected: &[&str]) {
    let out = repo.run_belaf_command(&["status", "--ci"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "status failed:\nstdout: {stdout}\nstderr: {stderr}"
    );
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("status is not JSON ({e}): {stdout}"));
    let mut names: Vec<String> = parsed["projects"]
        .as_array()
        .expect("projects array")
        .iter()
        .filter_map(|p| p["name"].as_str().map(str::to_owned))
        .collect();
    names.sort();
    let mut want: Vec<String> = expected.iter().map(|s| (*s).to_owned()).collect();
    want.sort();
    assert_eq!(names, want, "full status was: {stdout}");
}

/// `belaf init --auto-detect --ci`, failing the test loudly if it did not.
/// A silently failing init makes every later assertion vacuous.
fn init(repo: &TestRepo) {
    let out = repo.run_belaf_command(&["init", "--auto-detect", "--ci"]);
    assert!(
        out.status.success(),
        "init failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn seed_members(repo: &TestRepo) {
    repo.write_file(
        "packages/ui/package.json",
        r#"{ "name": "@acme/ui", "version": "1.0.0", "private": true }"#,
    );
    repo.write_file(
        "packages/api/package.json",
        r#"{
  "name": "@acme/api",
  "version": "1.0.0",
  "private": true,
  "dependencies": { "@acme/ui": "workspace:*" }
}"#,
    );
}

#[test]
fn npm_and_bun_declare_members_in_package_json() {
    let repo = TestRepo::new();
    repo.write_file(
        "package.json",
        r#"{ "name": "root", "private": true, "workspaces": ["packages/*"] }"#,
    );
    seed_members(&repo);
    repo.commit("feat: seed");
    init(&repo);

    assert_units(&repo, &["@acme/ui", "@acme/api"]);
}

/// The gap this test exists for: before `pnpm-workspace.yaml` was read, this
/// repo produced no members at all.
#[test]
fn pnpm_declares_members_in_a_sibling_yaml() {
    let repo = TestRepo::new();
    // Note: no `workspaces` key. This is what pnpm repos actually look like.
    repo.write_file("package.json", r#"{ "name": "root", "private": true }"#);
    repo.write_file(
        "pnpm-workspace.yaml",
        "packages:\n  - 'packages/*'\n\ncatalog:\n  react: ^19.0.0\n",
    );
    seed_members(&repo);
    repo.commit("feat: seed");
    init(&repo);

    assert_units(&repo, &["@acme/ui", "@acme/api"]);
}

/// Bun and npm both accept `!`-prefixed patterns in `workspaces`.
///
/// Read as a positive glob, `!packages/fixtures/**` turns into the literal
/// directory prefix `!packages/fixtures` and matches nothing — so the pattern
/// was silently inert and the directory stayed a member. It now excludes, and
/// the excluded directory is no longer wired into the workspace: no member
/// entry, and no internal dependency edge to or from it.
///
/// What it does *not* do — deliberately, and this test pins it so the
/// boundary cannot move by accident — is make the directory disappear. A
/// `package.json` outside the workspace is still a package, and belaf still
/// discovers it as a standalone unit. Whether an excluded path should instead
/// be treated like `[ignore_paths]` is a policy question, not a parsing one.
#[test]
fn negative_patterns_exclude_from_the_workspace() {
    let repo = TestRepo::new();
    repo.write_file(
        "package.json",
        r#"{ "name": "root", "private": true, "workspaces": ["packages/*", "!packages/fixtures/**"] }"#,
    );
    seed_members(&repo);
    repo.write_file(
        "packages/fixtures/package.json",
        r#"{
  "name": "@acme/fixtures",
  "version": "1.0.0",
  "private": true,
  "dependencies": { "@acme/ui": "workspace:*" }
}"#,
    );
    repo.commit("feat: seed");
    init(&repo);

    // Still discovered as a package in its own right.
    assert_units(&repo, &["@acme/ui", "@acme/api", "@acme/fixtures"]);

    // But not as a workspace member: belaf does not record an internal
    // dependency edge for it, which is what membership buys.
    let after = repo.read_file("packages/fixtures/package.json");
    assert!(
        !after.contains("internalDepVersions"),
        "an excluded directory is not a workspace member and gets no edge bookkeeping:\n{after}"
    );
    // The members that *are* in the workspace still do.
    let member = repo.read_file("packages/api/package.json");
    assert!(
        member.contains("internalDepVersions"),
        "a real member keeps its edge bookkeeping:\n{member}"
    );
}

/// `workspace:*` is a routing instruction. Rewriting it to the resolved
/// number points a `"private": true` dependency at the public registry.
#[test]
fn a_workspace_protocol_dependency_survives_a_release() {
    let repo = TestRepo::new();
    repo.write_file(
        "package.json",
        r#"{ "name": "root", "private": true, "workspaces": ["packages/*"] }"#,
    );
    seed_members(&repo);
    repo.commit("feat: seed");
    init(&repo);

    let after = repo.read_file("packages/api/package.json");
    assert!(
        after.contains(r#""@acme/ui": "workspace:*""#),
        "workspace: protocol must survive untouched, got:\n{after}"
    );
    assert!(
        !after.contains(r#""@acme/ui": "1.0.0""#),
        "a pinned version here breaks the install:\n{after}"
    );
}

/// The same rule for pnpm/bun catalogs: the version lives once, in the
/// catalog. Inlining it here silently opts the package out.
#[test]
fn a_catalog_dependency_survives_a_release() {
    let repo = TestRepo::new();
    repo.write_file(
        "package.json",
        r#"{ "name": "root", "private": true, "workspaces": ["packages/*"] }"#,
    );
    repo.write_file(
        "packages/ui/package.json",
        r#"{ "name": "@acme/ui", "version": "1.0.0", "private": true }"#,
    );
    repo.write_file(
        "packages/api/package.json",
        r#"{
  "name": "@acme/api",
  "version": "1.0.0",
  "private": true,
  "dependencies": { "@acme/ui": "catalog:" }
}"#,
    );
    repo.commit("feat: seed");
    init(&repo);

    let after = repo.read_file("packages/api/package.json");
    assert!(
        after.contains(r#""@acme/ui": "catalog:""#),
        "catalog: protocol must survive untouched, got:\n{after}"
    );
}

/// Whatever belaf does to a `package.json`, it must not reorder it. The
/// rewriters round-trip the file through `serde_json::Map`, which sorts
/// alphabetically unless `preserve_order` is on — turning a one-line version
/// bump into a whole-file diff, on manifests that are not even release units.
#[test]
fn a_rewrite_does_not_reshuffle_the_file() {
    let repo = TestRepo::new();
    repo.write_file(
        "package.json",
        r#"{ "name": "root", "private": true, "workspaces": ["packages/*"] }"#,
    );
    seed_members(&repo);
    repo.commit("feat: seed");
    init(&repo);

    let after = repo.read_file("packages/api/package.json");
    let name_at = after.find(r#""name""#).expect("name key");
    let version_at = after.find(r#""version""#).expect("version key");
    let deps_at = after.find(r#""dependencies""#).expect("dependencies key");
    assert!(
        name_at < version_at && version_at < deps_at,
        "source key order must survive, got:\n{after}"
    );
}
