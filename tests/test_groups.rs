//! `[[group]]` config-table integration tests.
//!
//! Verifies the wire contract end-to-end: a config with `[[group]]` entries
//! produces a manifest with both `groups[]` (containing the group's
//! user-facing member names) and `releases[].group_id` set on each member.
//! That contract is what the github-app reads to drive atomic group
//! releases (plan §7).

mod common;

use common::TestRepo;

fn read_manifest_json(repo: &TestRepo) -> serde_json::Value {
    let files = repo.list_files_in_dir("belaf/releases");
    let manifest_file = files
        .iter()
        .find(|f| f.ends_with(".json"))
        .expect("a manifest .json should have been written");
    let content = repo.read_file(&format!("belaf/releases/{manifest_file}"));
    serde_json::from_str(&content).expect("manifest must be valid JSON")
}

#[test]
fn group_id_propagates_to_manifest_releases() {
    let repo = TestRepo::new();

    repo.write_file(
        "packages/npm/package.json",
        r#"{
  "name": "@org/schema",
  "version": "0.1.0",
  "main": "index.js"
}
"#,
    );
    repo.write_file("packages/npm/index.js", "module.exports = {};\n");
    repo.write_file(
        "packages/cargo/Cargo.toml",
        r#"[package]
name = "schema-rs"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write_file("packages/cargo/src/lib.rs", "pub fn schema() {}\n");
    repo.commit("init");

    let init_out = repo.run_belaf_command(&["init", "--force"]);
    assert!(
        init_out.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&init_out.stderr)
    );

    // Append a [[group]] section binding both projects together.
    let cfg = repo.read_file("belaf/config.toml");
    let cfg_with_group =
        format!("{cfg}\n[[group]]\nid = \"schema\"\nmembers = [\"@org/schema\", \"schema-rs\"]\n");
    repo.write_file("belaf/config.toml", &cfg_with_group);
    repo.commit("chore: add schema group");

    repo.write_file(
        "packages/npm/feature.js",
        "module.exports.next = () => null;\n",
    );
    repo.write_file("packages/cargo/src/feature.rs", "pub fn next() {}\n");
    repo.commit("feat: shared schema feature");

    // Discard exit status: PR push (which needs auth) happens after manifest
    // emission. We only care that the manifest was written correctly.
    let _ = repo.run_belaf_command(&["prepare", "--ci"]);

    let manifest = read_manifest_json(&repo);

    let groups = manifest["groups"]
        .as_array()
        .expect("manifest must have groups[] (possibly empty)");
    assert_eq!(groups.len(), 1, "expected one emitted group");
    assert_eq!(groups[0]["id"], "schema");
    let members: Vec<&str> = groups[0]["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m.as_str().unwrap())
        .collect();
    assert!(members.contains(&"@org/schema"));
    assert!(members.contains(&"schema-rs"));

    let releases = manifest["releases"].as_array().unwrap();
    assert_eq!(releases.len(), 2);
    for r in releases {
        assert_eq!(
            r["group_id"], "schema",
            "every group member release must carry group_id"
        );
    }
}

#[test]
fn no_groups_yields_empty_groups_array() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "solo"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn solo() {}\n");
    repo.commit("init");

    let init_out = repo.run_belaf_command(&["init", "--force"]);
    assert!(init_out.status.success());

    repo.write_file("src/two.rs", "pub fn two() {}\n");
    repo.commit("feat: add two");

    let _ = repo.run_belaf_command(&["prepare", "--ci"]);

    let manifest = read_manifest_json(&repo);
    // `groups` may be omitted entirely (typify skips empty `Vec`) or present-
    // but-empty. Either is fine — both convey "no groups configured".
    if let Some(groups) = manifest["groups"].as_array() {
        assert!(
            groups.is_empty(),
            "no [[group]] entries should yield an empty groups array, got {groups:?}"
        );
    }

    let releases = manifest["releases"].as_array().unwrap();
    for r in releases {
        assert!(
            r["group_id"].is_null(),
            "release outside any group must have group_id == null"
        );
    }
}

#[test]
fn invalid_group_id_pattern_rejected_at_load_time() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "bad-cfg"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn x() {}\n");
    repo.commit("init");

    let init_out = repo.run_belaf_command(&["init", "--force"]);
    assert!(init_out.status.success());

    let cfg = repo.read_file("belaf/config.toml");
    // `Schema` is invalid: capital letter not allowed by the pattern.
    let cfg_bad = format!("{cfg}\n[[group]]\nid = \"Schema\"\nmembers = [\"bad-cfg\"]\n");
    repo.write_file("belaf/config.toml", &cfg_bad);
    repo.commit("chore: bad group id");

    let out = repo.run_belaf_command(&["prepare", "--ci"]);
    assert!(
        !out.status.success(),
        "prepare should fail when [[group]] id is invalid"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("group") || stderr.contains("Schema"),
        "error must mention the offending group id; got:\n{stderr}"
    );
}

#[test]
fn conflicting_project_overrides_within_group_rejected() {
    let repo = TestRepo::new();
    repo.write_file(
        "packages/npm/package.json",
        r#"{ "name": "@org/schema", "version": "0.1.0", "main": "index.js" }
"#,
    );
    repo.write_file("packages/npm/index.js", "module.exports = {};\n");
    repo.write_file(
        "packages/cargo/Cargo.toml",
        r#"[package]
name = "schema-rs"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write_file("packages/cargo/src/lib.rs", "pub fn schema() {}\n");
    repo.commit("init");

    let init = repo.run_belaf_command(&["init", "--force"]);
    assert!(init.status.success());

    let cfg = repo.read_file("belaf/config.toml");
    let cfg_with_group =
        format!("{cfg}\n[[group]]\nid = \"schema\"\nmembers = [\"@org/schema\", \"schema-rs\"]\n");
    repo.write_file("belaf/config.toml", &cfg_with_group);

    repo.write_file(
        "packages/npm/feat.js",
        "module.exports.next = () => null;\n",
    );
    repo.write_file("packages/cargo/src/feat.rs", "pub fn next() {}\n");
    repo.commit("feat: drift");

    let out = repo.run_belaf_command(&[
        "prepare",
        "--ci",
        "--project",
        "@org/schema:major,schema-rs:patch",
    ]);
    assert!(
        !out.status.success(),
        "prepare must fail when --project flags push group members in different directions"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("schema") && stderr.contains("must share one bump"),
        "stderr must explain the group-bump conflict; got:\n{stderr}"
    );
}

#[test]
fn intra_group_cyclic_deps_are_filtered_not_rejected() {
    // Plan §5 / Gap #6: members of one release group are atomic — they
    // share a bump and ship together — so circular deps between them
    // must NOT be flagged as a graph cycle. A schema-bundle where
    // both the npm package and the Maven artifact reference each other
    // in their dep manifests should still build cleanly.
    let repo = TestRepo::new();

    repo.write_file(
        "packages/npm/package.json",
        r#"{
  "name": "@org/schema",
  "version": "0.1.0",
  "main": "index.js",
  "dependencies": { "schema-rs": "0.1.0" }
}
"#,
    );
    repo.write_file("packages/npm/index.js", "module.exports = {};\n");

    // schema-rs in turn depends on @org/schema — circular within the
    // group. (We can't actually express an npm dep target from
    // Cargo.toml, but the cargo loader walks `[dependencies]`
    // unconditionally — we use a dep on another local crate that is
    // itself in the same group.)
    repo.write_file(
        "packages/cargo/Cargo.toml",
        r#"[package]
name = "schema-rs"
version = "0.1.0"
edition = "2021"

[dependencies]
schema-rs-helper = { path = "../cargo-helper", version = "0.1.0" }
"#,
    );
    repo.write_file("packages/cargo/src/lib.rs", "pub fn schema() {}\n");

    repo.write_file(
        "packages/cargo-helper/Cargo.toml",
        r#"[package]
name = "schema-rs-helper"
version = "0.1.0"
edition = "2021"

[dependencies]
schema-rs = { path = "../cargo", version = "0.1.0" }
"#,
    );
    repo.write_file("packages/cargo-helper/src/lib.rs", "pub fn helper() {}\n");
    repo.commit("init");

    let init = repo.run_belaf_command(&["init", "--force"]);
    assert!(
        init.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    let cfg = repo.read_file("belaf/config.toml");
    let cfg_with_group = format!(
        "{cfg}\n[[group]]\nid = \"schema\"\nmembers = [\"schema-rs\", \"schema-rs-helper\"]\n"
    );
    repo.write_file("belaf/config.toml", &cfg_with_group);

    repo.write_file("packages/cargo/src/feat.rs", "pub fn next() {}\n");
    repo.commit("feat: shared schema feature");

    let out = repo.run_belaf_command(&["prepare", "--ci"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    // The intra-group cycle between schema-rs ↔ schema-rs-helper must
    // NOT trip cycle-detection. (The release itself may still fail
    // later because no GitHub App is installed — we only assert the
    // graph build succeeded by checking we got past cycle detection.)
    assert!(
        !stderr.contains("dependency cycle"),
        "intra-group dep cycle should be filtered, not rejected; got stderr:\n{stderr}"
    );
}

#[test]
fn divergent_commit_bumps_in_group_rejected_with_diagnostic() {
    // Plan §5: every member of one group ships with one shared bump.
    // When commits naturally suggest different levels per member (one
    // feat:, another fix:), the system must NOT silently pick a side or
    // auto-promote — it hard-errors with a diagnostic naming both
    // members and their conflicting bumps, telling the user how to
    // reconcile. Complements
    // `conflicting_project_overrides_within_group_rejected` (which
    // covers the *explicit* --project conflict path) by pinning the
    // *implicit* conventional-commits path through the same gate.
    let repo = TestRepo::new();

    repo.write_file(
        "packages/npm/package.json",
        r#"{
  "name": "@org/schema",
  "version": "0.1.0",
  "main": "index.js"
}
"#,
    );
    repo.write_file("packages/npm/index.js", "module.exports = {};\n");
    repo.write_file(
        "packages/cargo/Cargo.toml",
        r#"[package]
name = "schema-rs"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write_file("packages/cargo/src/lib.rs", "pub fn schema() {}\n");
    repo.commit("init");

    let init = repo.run_belaf_command(&["init", "--force"]);
    assert!(
        init.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    let cfg = repo.read_file("belaf/config.toml");
    let cfg_with_group =
        format!("{cfg}\n[[group]]\nid = \"schema\"\nmembers = [\"@org/schema\", \"schema-rs\"]\n");
    repo.write_file("belaf/config.toml", &cfg_with_group);
    repo.commit("chore: bind schema group");

    // Member A: feat: → would be minor in isolation.
    // Member B: fix: → would be patch in isolation.
    repo.write_file(
        "packages/npm/feature.js",
        "module.exports.next = () => null;\n",
    );
    repo.commit("feat(schema): add npm feature");
    repo.write_file("packages/cargo/src/fix.rs", "pub fn fix() {}\n");
    repo.commit("fix(schema-rs): patch a typo");

    let out = repo.run_belaf_command(&["prepare", "--ci"]);
    assert!(
        !out.status.success(),
        "prepare must fail when commit-driven bumps within a group diverge"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("schema") && stderr.contains("must share one bump"),
        "stderr must explain the group-bump conflict; got:\n{stderr}"
    );
    assert!(
        stderr.contains("minor") && stderr.contains("patch"),
        "diagnostic must name both conflicting bump levels; got:\n{stderr}"
    );
    assert!(
        stderr.contains("@org/schema") && stderr.contains("schema-rs"),
        "diagnostic must name both conflicting members; got:\n{stderr}"
    );
}

#[test]
fn unknown_group_member_rejected_at_load_time() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "real-pkg"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn x() {}\n");
    repo.commit("init");

    let init_out = repo.run_belaf_command(&["init", "--force"]);
    assert!(init_out.status.success());

    let cfg = repo.read_file("belaf/config.toml");
    let cfg_bad = format!(
        "{cfg}\n[[group]]\nid = \"phantom\"\nmembers = [\"real-pkg\", \"does-not-exist\"]\n"
    );
    repo.write_file("belaf/config.toml", &cfg_bad);
    repo.commit("chore: phantom member");

    let out = repo.run_belaf_command(&["prepare", "--ci"]);
    assert!(
        !out.status.success(),
        "prepare should fail when [[group]] references an unknown project"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("does-not-exist") || stderr.contains("phantom"),
        "error must mention the offending member or group; got:\n{stderr}"
    );
}
