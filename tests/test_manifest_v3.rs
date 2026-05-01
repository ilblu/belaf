//! Schema-level integration tests for manifest 3.0.
//!
//! Unit tests in `core::wire::domain` already cover serde round-trips on
//! the domain types. These tests verify the *schema invariants* the
//! github-app relies on when consuming the manifest — that the actual
//! files belaf writes to disk match the contract documented in
//! `schemas/manifest.v3.0.schema.json`.

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

fn init_simple_cargo_project(repo: &TestRepo) -> String {
    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "schema-test"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn x() {}\n");
    repo.commit("init");

    let init = repo.run_belaf_command(&["init", "--force"]);
    assert!(
        init.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    repo.write_file("src/feat.rs", "pub fn feat() {}\n");
    repo.commit("feat: drift");

    let _ = repo.run_belaf_command(&["prepare", "--ci"]);

    repo.list_files_in_dir("belaf/releases")
        .into_iter()
        .find(|f| f.ends_with(".json"))
        .expect("manifest file written")
}

#[test]
fn manifest_filename_is_uuid_v7_dot_json() {
    let repo = TestRepo::new();
    let manifest_filename = init_simple_cargo_project(&repo);
    assert!(
        manifest_filename.ends_with(".json"),
        "filename must end in `.json`, got {manifest_filename}"
    );
    let stem = manifest_filename.trim_end_matches(".json");
    let parts: Vec<&str> = stem.split('-').collect();
    assert_eq!(parts.len(), 5, "UUID must be 5 dash-separated groups");
    assert_eq!(parts[0].len(), 8);
    assert_eq!(parts[1].len(), 4);
    assert_eq!(parts[2].len(), 4);
    assert_eq!(parts[3].len(), 4);
    assert_eq!(parts[4].len(), 12);
    assert!(
        parts[2].starts_with('7'),
        "UUID v7 third group must start with `7`, got {}",
        parts[2]
    );
}

#[test]
fn manifest_has_v3_schema_version() {
    let repo = TestRepo::new();
    init_simple_cargo_project(&repo);
    let m = read_manifest_json(&repo);
    assert_eq!(m["schema_version"], "3.0");
}

#[test]
fn manifest_top_level_has_required_keys() {
    let repo = TestRepo::new();
    init_simple_cargo_project(&repo);
    let m = read_manifest_json(&repo);
    for key in &[
        "schema_version",
        "manifest_id",
        "created_at",
        "created_by",
        "base_branch",
        "releases",
    ] {
        assert!(!m[key].is_null(), "manifest missing required key `{key}`");
    }
    // `created_at` must look like RFC 3339 (presence of `T` and either Z
    // or a +/- offset).
    let created_at = m["created_at"].as_str().expect("created_at is string");
    assert!(created_at.contains('T'), "RFC 3339 needs a `T`");
    assert!(
        created_at.contains('Z') || created_at.contains('+') || created_at.contains('-'),
        "RFC 3339 needs a Z or offset, got: {created_at}"
    );
}

#[test]
fn manifest_release_has_v2_required_fields() {
    let repo = TestRepo::new();
    init_simple_cargo_project(&repo);
    let m = read_manifest_json(&repo);
    let r = &m["releases"][0];
    for key in &[
        "name",
        "ecosystem",
        "previous_version",
        "new_version",
        "bump_type",
        "tag_name",
    ] {
        assert!(
            !r[key].is_null(),
            "release object missing required v2 field `{key}`"
        );
    }
    assert_eq!(
        r["ecosystem"], "cargo",
        "ecosystem must be the wire string, not display_name"
    );
}

#[test]
fn manifest_release_has_x_extension_namespace() {
    let repo = TestRepo::new();
    init_simple_cargo_project(&repo);
    let m = read_manifest_json(&repo);
    // `x` may be omitted when empty (typify skips empty Map), but if
    // present it must be an object — never a non-object scalar.
    if !m["x"].is_null() {
        assert!(m["x"].is_object(), "top-level `x` must be an object");
    }
    let r = &m["releases"][0];
    if !r["x"].is_null() {
        assert!(r["x"].is_object(), "release `x` must be an object");
    }
}

#[test]
fn manifest_does_not_emit_v1_only_fields() {
    // The v1 schema had `prerelease` (boolean, deprecated alongside the
    // canonical `is_prerelease`). v2 only has `is_prerelease`. Make sure
    // we don't accidentally re-emit the v1-era field.
    let repo = TestRepo::new();
    init_simple_cargo_project(&repo);
    let m = read_manifest_json(&repo);
    let r = &m["releases"][0];
    assert!(
        r.get("prerelease").is_none() || r["prerelease"].is_null(),
        "deprecated v1 `prerelease` field must not appear in v2 output, got: {r}"
    );
    assert!(
        r.get("prefix").is_none() || r["prefix"].is_null(),
        "v1 `prefix` field is removed in v2 (CLI owns full tag_name now)"
    );
}
