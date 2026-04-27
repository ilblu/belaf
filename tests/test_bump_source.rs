//! Integration tests for `--bump-source` / `--bump-source-cmd` /
//! `[[bump_source]]` config-driven bump decisions.
//!
//! Unit tests for the JSON parser + subprocess runner live in
//! `core::bump_source`. These run end-to-end against a temp repo so the
//! precedence chain (config → CLI flag → `--project` override →
//! conventional commits) is exercised through real `belaf prepare --ci`
//! invocations.

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

fn init_simple_cargo_project(repo: &TestRepo) {
    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "my-crate"
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
}

#[test]
fn bump_source_file_overrides_conventional_commits() {
    let repo = TestRepo::new();
    init_simple_cargo_project(&repo);

    // Decision file forces a major bump despite the `fix:` commit.
    repo.write_file(
        "belaf/decisions.json",
        r#"{
  "version": 1,
  "decisions": [
    { "project": "my-crate", "bump": "major",
      "reason": "external schema break detected", "source": "test" }
  ]
}
"#,
    );
    repo.write_file("src/feat.rs", "pub fn feat() {}\n");
    repo.commit("fix: would normally bump patch");

    let _ = repo.run_belaf_command(&["prepare", "--ci", "--bump-source", "belaf/decisions.json"]);

    let manifest = read_manifest_json(&repo);
    let releases = manifest["releases"].as_array().unwrap();
    assert_eq!(releases.len(), 1);
    assert_eq!(
        releases[0]["bump_type"], "major",
        "bump-source decision must override conventional-commit inference"
    );
    assert_eq!(releases[0]["new_version"], "2.0.0");
}

#[test]
fn project_override_beats_bump_source() {
    let repo = TestRepo::new();
    init_simple_cargo_project(&repo);

    repo.write_file(
        "belaf/decisions.json",
        r#"{
  "version": 1,
  "decisions": [{ "project": "my-crate", "bump": "major" }]
}
"#,
    );
    repo.write_file("src/feat.rs", "pub fn feat() {}\n");
    repo.commit("fix: bug");

    // CLI --project overrides the decision file.
    let _ = repo.run_belaf_command(&[
        "prepare",
        "--ci",
        "--bump-source",
        "belaf/decisions.json",
        "--project",
        "my-crate:minor",
    ]);

    let manifest = read_manifest_json(&repo);
    let releases = manifest["releases"].as_array().unwrap();
    assert_eq!(
        releases[0]["bump_type"], "minor",
        "--project override must beat --bump-source"
    );
}

#[test]
fn bump_source_cmd_executes_subprocess() {
    let repo = TestRepo::new();
    init_simple_cargo_project(&repo);

    repo.write_file("src/feat.rs", "pub fn feat() {}\n");
    repo.commit("chore: noop");

    // Inline command emits a v1 envelope on stdout.
    let cmd = r#"printf '{"version":1,"decisions":[{"project":"my-crate","bump":"minor"}]}'"#;

    let _ = repo.run_belaf_command(&["prepare", "--ci", "--bump-source-cmd", cmd]);

    let manifest = read_manifest_json(&repo);
    let releases = manifest["releases"].as_array().unwrap();
    assert_eq!(releases[0]["bump_type"], "minor");
}

#[test]
fn malformed_bump_source_is_hard_error() {
    let repo = TestRepo::new();
    init_simple_cargo_project(&repo);

    repo.write_file("belaf/bad.json", "{ this is not valid json");
    repo.write_file("src/feat.rs", "pub fn feat() {}\n");
    repo.commit("fix: bug");

    let out = repo.run_belaf_command(&["prepare", "--ci", "--bump-source", "belaf/bad.json"]);
    assert!(
        !out.status.success(),
        "prepare must fail when --bump-source JSON is malformed"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("malformed") || stderr.contains("invalid"),
        "stderr must explain the malformed payload; got:\n{stderr}"
    );
}

#[test]
fn unsupported_version_field_is_hard_error() {
    let repo = TestRepo::new();
    init_simple_cargo_project(&repo);

    repo.write_file("belaf/v2.json", r#"{ "version": 2, "decisions": [] }"#);
    repo.write_file("src/feat.rs", "pub fn feat() {}\n");
    repo.commit("fix: bug");

    let out = repo.run_belaf_command(&["prepare", "--ci", "--bump-source", "belaf/v2.json"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("version 2") || stderr.contains("version"),
        "stderr must mention the unsupported version; got:\n{stderr}"
    );
}

#[test]
fn unknown_project_in_decision_is_hard_error() {
    let repo = TestRepo::new();
    init_simple_cargo_project(&repo);

    repo.write_file(
        "belaf/d.json",
        r#"{ "version": 1,
             "decisions": [{ "project": "ghost-pkg", "bump": "patch" }] }"#,
    );
    repo.write_file("src/feat.rs", "pub fn feat() {}\n");
    repo.commit("fix: bug");

    let out = repo.run_belaf_command(&["prepare", "--ci", "--bump-source", "belaf/d.json"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("ghost-pkg"),
        "stderr must name the unknown project; got:\n{stderr}"
    );
}

#[test]
fn config_bump_source_runs_by_default() {
    let repo = TestRepo::new();
    init_simple_cargo_project(&repo);

    let cfg = repo.read_file("belaf/config.toml");
    let cfg_with_source = format!(
        "{cfg}\n[[bump_source]]\ncmd = \"printf '{{\\\"version\\\":1,\\\"decisions\\\":[{{\\\"project\\\":\\\"my-crate\\\",\\\"bump\\\":\\\"major\\\"}}]}}'\"\ntimeout_sec = 10\n"
    );
    repo.write_file("belaf/config.toml", &cfg_with_source);
    repo.write_file("src/feat.rs", "pub fn feat() {}\n");
    repo.commit("chore: bump_source + change");

    let _ = repo.run_belaf_command(&["prepare", "--ci"]);

    let manifest = read_manifest_json(&repo);
    let releases = manifest["releases"].as_array().unwrap();
    assert_eq!(
        releases[0]["bump_type"], "major",
        "[[bump_source]] in config must drive the bump without explicit --bump-source flag"
    );
}
