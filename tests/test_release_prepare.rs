mod common;

use common::TestRepo;

#[test]
fn test_release_prepare_patch_bump() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "my-crate"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
    repo.commit("Initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(
        output.status.success(),
        "Init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    repo.write_file("src/fix.rs", "pub fn fix_bug() {}\n");
    repo.commit("fix: resolve critical bug");

    let output = repo.run_belaf_command(&["prepare", "--no-tui", "auto"]);
    assert!(
        output.status.success(),
        "Prepare failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let cargo_toml = repo.read_file("Cargo.toml");
    assert!(
        cargo_toml.contains("version = \"1.0.1\""),
        "Version should be bumped to 1.0.1 for fix commit. Got: {cargo_toml}"
    );
}

#[test]
fn test_release_prepare_minor_bump() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "feature-crate"
version = "2.0.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
    repo.commit("Initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(output.status.success());

    repo.write_file("src/feature.rs", "pub fn new_feature() {}\n");
    repo.commit("feat: add amazing new feature");

    let output = repo.run_belaf_command(&["prepare", "--no-tui", "auto"]);
    assert!(
        output.status.success(),
        "Prepare failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let cargo_toml = repo.read_file("Cargo.toml");
    assert!(
        cargo_toml.contains("version = \"2.1.0\""),
        "Version should be bumped to 2.1.0 for feat commit. Got: {cargo_toml}"
    );
}

#[test]
fn test_release_prepare_major_bump() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "breaking-crate"
version = "1.5.3"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
    repo.commit("Initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(output.status.success());

    repo.write_file("src/breaking.rs", "pub fn breaking_change() {}\n");
    repo.commit("feat!: breaking API change");

    let output = repo.run_belaf_command(&["prepare", "--no-tui", "auto"]);
    assert!(
        output.status.success(),
        "Prepare failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let cargo_toml = repo.read_file("Cargo.toml");
    assert!(
        cargo_toml.contains("version = \"2.0.0\""),
        "Version should be bumped to 2.0.0 for breaking change. Got: {cargo_toml}"
    );
}

#[test]
fn test_release_prepare_updates_version_file_only() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "changelog-test"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
    repo.commit("Initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(output.status.success());

    repo.write_file("src/new.rs", "pub fn something_new() {}\n");
    repo.commit("feat: add something new");

    let output = repo.run_belaf_command(&["prepare", "--no-tui", "auto"]);
    assert!(
        output.status.success(),
        "Prepare failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let cargo_toml = repo.read_file("Cargo.toml");
    assert!(
        cargo_toml.contains("version = \"0.2.0\""),
        "Version should be bumped to 0.2.0 for feat commit. Got: {cargo_toml}"
    );
}

#[test]
fn test_release_prepare_npm_package() {
    let repo = TestRepo::new();

    repo.write_file(
        "package.json",
        r#"{
  "name": "my-npm-package",
  "version": "3.0.0",
  "description": "Test package"
}
"#,
    );
    repo.write_file("index.js", "module.exports = {};\n");
    repo.commit("Initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(output.status.success());

    repo.write_file("feature.js", "module.exports.feature = () => {};\n");
    repo.commit("feat: add new feature");

    let output = repo.run_belaf_command(&["prepare", "--no-tui", "auto"]);
    assert!(
        output.status.success(),
        "Prepare failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let package_json = repo.read_file("package.json");
    assert!(
        package_json.contains("\"version\": \"3.1.0\""),
        "package.json version should be bumped to 3.1.0. Got: {package_json}"
    );
}

#[test]
fn test_release_prepare_python_package() {
    let repo = TestRepo::new();

    repo.write_file(
        "setup.cfg",
        r"[metadata]
name = my-python-pkg
version = 1.0.0
description = Test package
",
    );
    repo.write_file(
        "setup.py",
        r#"from setuptools import setup

__version__ = "1.0.0"  # belaf project-version

setup(version=__version__)
"#,
    );
    repo.write_file("src/__init__.py", "");
    repo.commit("Initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(
        output.status.success(),
        "Init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    repo.write_file("src/feature.py", "def new_feature(): pass\n");
    repo.commit("fix: resolve issue");

    let output = repo.run_belaf_command(&["prepare", "--no-tui", "auto"]);
    assert!(
        output.status.success(),
        "Prepare failed: {} | stdout: {}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );

    let setup_py = repo.read_file("setup.py");
    assert!(
        setup_py.contains("\"1.0.1\""),
        "setup.py version should be bumped to 1.0.1. Got: {setup_py}"
    );
}

#[test]
fn test_release_prepare_multiple_commits() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "multi-commit"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
    repo.commit("Initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(output.status.success());

    repo.write_file("src/fix1.rs", "pub fn fix1() {}\n");
    repo.commit("fix: first bug fix");

    repo.write_file("src/fix2.rs", "pub fn fix2() {}\n");
    repo.commit("fix: second bug fix");

    repo.write_file("src/feature.rs", "pub fn feature() {}\n");
    repo.commit("feat: new feature");

    let output = repo.run_belaf_command(&["prepare", "--no-tui", "auto"]);
    assert!(
        output.status.success(),
        "Prepare failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let cargo_toml = repo.read_file("Cargo.toml");
    assert!(
        cargo_toml.contains("version = \"1.1.0\""),
        "feat should result in minor bump (1.1.0). Got: {cargo_toml}"
    );
}

#[test]
fn test_release_prepare_no_changes() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "no-changes"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
    repo.commit("Initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(output.status.success());

    let _output = repo.run_belaf_command(&["prepare", "--no-tui", "auto"]);

    let cargo_toml = repo.read_file("Cargo.toml");
    assert!(
        cargo_toml.contains("version = \"1.0.0\""),
        "Version should remain 1.0.0 with no changes. Got: {cargo_toml}"
    );
}

#[test]
fn test_release_prepare_preserves_cargo_toml_formatting() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "format-test"
version = "1.0.0"
edition = "2021"
description = "A test package"

[dependencies]
serde = "1.0"
tokio = { version = "1.0", features = ["full"] }

[dev-dependencies]
tempfile = "3.0"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
    repo.commit("Initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(output.status.success());

    repo.write_file("src/fix.rs", "pub fn fix() {}\n");
    repo.commit("fix: bug fix");

    let output = repo.run_belaf_command(&["prepare", "--no-tui", "auto"]);
    assert!(output.status.success());

    let cargo_toml = repo.read_file("Cargo.toml");

    assert!(
        cargo_toml.contains("[dependencies]"),
        "Should preserve [dependencies] section"
    );
    assert!(
        cargo_toml.contains("serde = \"1.0\""),
        "Should preserve serde dependency"
    );
    assert!(
        cargo_toml.contains("tokio = { version = \"1.0\", features = [\"full\"] }"),
        "Should preserve tokio inline table format"
    );
    assert!(
        cargo_toml.contains("[dev-dependencies]"),
        "Should preserve [dev-dependencies] section"
    );
}

#[test]
fn test_release_prepare_monorepo_multiple_packages() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[workspace]
members = ["packages/*"]
resolver = "2"
"#,
    );

    repo.write_file(
        "packages/core/Cargo.toml",
        r#"[package]
name = "monorepo-core"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("packages/core/src/lib.rs", "pub fn core() {}\n");

    repo.write_file(
        "packages/utils/Cargo.toml",
        r#"[package]
name = "monorepo-utils"
version = "2.0.0"
edition = "2021"
"#,
    );
    repo.write_file("packages/utils/src/lib.rs", "pub fn utils() {}\n");

    repo.commit("Initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(output.status.success());

    repo.write_file("packages/core/src/feature.rs", "pub fn new_core() {}\n");
    repo.commit("feat(core): add core feature");

    repo.write_file("packages/utils/src/fix.rs", "pub fn fix_utils() {}\n");
    repo.commit("fix(utils): fix utils bug");

    let output = repo.run_belaf_command(&["prepare", "--no-tui", "auto"]);
    assert!(
        output.status.success(),
        "Prepare failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let core_toml = repo.read_file("packages/core/Cargo.toml");
    let utils_toml = repo.read_file("packages/utils/Cargo.toml");

    assert!(
        core_toml.contains("version = \"1.1.0\"") || core_toml.contains("version = \"1.0.1\""),
        "Core should be bumped. Got: {core_toml}"
    );
    assert!(
        utils_toml.contains("version = \"2.0.1\"") || utils_toml.contains("version = \"2.1.0\""),
        "Utils should be bumped. Got: {utils_toml}"
    );
}

#[test]
fn test_ci_mode_rejects_dirty_repository() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "ci-dirty-test"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
    repo.commit("Initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(output.status.success());

    repo.write_file("src/feature.rs", "pub fn feature() {}\n");
    repo.commit("feat: add feature");

    repo.write_file("src/uncommitted.rs", "pub fn uncommitted() {}\n");

    let output = repo.run_belaf_command(&["prepare", "--ci"]);

    assert!(
        !output.status.success(),
        "CI mode should fail with dirty repository"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("clean working directory") || stderr.contains("uncommitted"),
        "Error should mention clean working directory. Got: {stderr}"
    );
}

#[test]
fn test_ci_mode_succeeds_with_clean_repo_no_changes() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "ci-clean-test"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
    repo.commit("Initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(output.status.success());

    repo.commit("chore: add belaf config");

    let output = repo.run_belaf_command(&["prepare", "--ci"]);

    assert!(
        output.status.success(),
        "CI mode should succeed with clean repo when no changes need releasing. Got: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let cargo_toml = repo.read_file("Cargo.toml");
    assert!(
        cargo_toml.contains("version = \"1.0.0\""),
        "Version should remain unchanged when no releases needed. Got: {cargo_toml}"
    );
}

#[test]
fn test_release_prepare_explicit_bump_type() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "explicit-bump"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
    repo.commit("Initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(output.status.success());

    repo.write_file("src/fix.rs", "pub fn fix() {}\n");
    repo.commit("fix: small fix");

    let output = repo.run_belaf_command(&["prepare", "--no-tui", "major"]);
    assert!(
        output.status.success(),
        "Prepare with explicit major should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let cargo_toml = repo.read_file("Cargo.toml");
    assert!(
        cargo_toml.contains("version = \"2.0.0\""),
        "Explicit major bump should result in 2.0.0. Got: {cargo_toml}"
    );
}

#[test]
fn test_release_prepare_per_project_mode() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[workspace]
members = ["packages/*"]
resolver = "2"
"#,
    );

    repo.write_file(
        "packages/alpha/Cargo.toml",
        r#"[package]
name = "alpha"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("packages/alpha/src/lib.rs", "pub fn alpha() {}\n");

    repo.write_file(
        "packages/beta/Cargo.toml",
        r#"[package]
name = "beta"
version = "2.0.0"
edition = "2021"
"#,
    );
    repo.write_file("packages/beta/src/lib.rs", "pub fn beta() {}\n");

    repo.commit("Initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(output.status.success());

    repo.write_file("packages/alpha/src/new.rs", "pub fn new_alpha() {}\n");
    repo.commit("feat: add alpha feature");

    repo.write_file("packages/beta/src/new.rs", "pub fn new_beta() {}\n");
    repo.commit("feat: add beta feature");

    let output = repo.run_belaf_command(&["prepare", "--project", "alpha:major,beta:patch"]);
    assert!(
        output.status.success(),
        "Per-project mode should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let alpha_toml = repo.read_file("packages/alpha/Cargo.toml");
    let beta_toml = repo.read_file("packages/beta/Cargo.toml");

    assert!(
        alpha_toml.contains("version = \"2.0.0\""),
        "Alpha should be major bumped to 2.0.0. Got: {alpha_toml}"
    );
    assert!(
        beta_toml.contains("version = \"2.0.1\""),
        "Beta should be patch bumped to 2.0.1. Got: {beta_toml}"
    );
}

#[test]
fn test_ci_mode_creates_manifest_directory() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "manifest-test"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
    repo.commit("Initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(output.status.success());
    repo.commit("chore: add belaf config");

    repo.write_file("src/feature.rs", "pub fn feature() {}\n");
    repo.commit("feat: add new feature");

    let _ = repo.run_belaf_command(&["prepare", "--ci"]);

    assert!(
        repo.file_exists("belaf/releases"),
        "CI mode should create belaf/releases directory"
    );

    let manifest_files = repo.list_files_in_dir("belaf/releases");
    assert!(
        !manifest_files.is_empty(),
        "CI mode should create at least one manifest file"
    );

    let manifest_file = &manifest_files[0];
    assert!(
        manifest_file.starts_with("release-") && manifest_file.ends_with(".json"),
        "Manifest filename should match pattern release-*.json, got: {manifest_file}"
    );
}

#[test]
fn test_ci_mode_manifest_has_valid_json_structure() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "json-test"
version = "2.0.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
    repo.commit("Initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(output.status.success());
    repo.commit("chore: add belaf config");

    repo.write_file("src/fix.rs", "pub fn fix() {}\n");
    repo.commit("fix: critical bugfix");

    let _ = repo.run_belaf_command(&["prepare", "--ci"]);

    let manifest_files = repo.list_files_in_dir("belaf/releases");
    assert!(!manifest_files.is_empty(), "Should have manifest file");

    let manifest_path = format!("belaf/releases/{}", manifest_files[0]);
    let manifest_content = repo.read_file(&manifest_path);

    let manifest: serde_json::Value =
        serde_json::from_str(&manifest_content).expect("Manifest should be valid JSON");

    assert_eq!(
        manifest["schema_version"], "1.0",
        "Schema version should be 1.0"
    );
    assert!(
        manifest["created_at"].is_string(),
        "created_at should be a string"
    );
    assert!(
        manifest["created_by"].is_string(),
        "created_by should be a string"
    );
    assert!(
        manifest["base_branch"].is_string(),
        "base_branch should be a string"
    );
    assert!(
        manifest["releases"].is_array(),
        "releases should be an array"
    );

    let releases = manifest["releases"].as_array().unwrap();
    assert_eq!(releases.len(), 1, "Should have one release");

    let release = &releases[0];
    assert_eq!(release["name"], "json-test");
    assert_eq!(release["ecosystem"], "Rust (Cargo)");
    assert_eq!(release["previous_version"], "2.0.0");
    assert_eq!(release["new_version"], "2.0.1");
    assert_eq!(release["bump_type"], "patch");
    assert!(release["changelog"].is_string());
    assert!(release["tag_name"].is_string());
}

#[test]
fn test_ci_mode_manifest_contains_multiple_projects() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[workspace]
members = ["packages/*"]
"#,
    );

    repo.write_file(
        "packages/alpha/Cargo.toml",
        r#"[package]
name = "alpha"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("packages/alpha/src/lib.rs", "pub fn alpha() {}\n");

    repo.write_file(
        "packages/beta/Cargo.toml",
        r#"[package]
name = "beta"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("packages/beta/src/lib.rs", "pub fn beta() {}\n");

    repo.commit("Initial monorepo");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(output.status.success());
    repo.commit("chore: add belaf config");

    repo.write_file("packages/alpha/src/new.rs", "pub fn new_alpha() {}\n");
    repo.commit("feat(alpha): add new feature");

    repo.write_file("packages/beta/src/fix.rs", "pub fn fix_beta() {}\n");
    repo.commit("fix(beta): fix bug");

    let _ = repo.run_belaf_command(&["prepare", "--ci"]);

    let manifest_files = repo.list_files_in_dir("belaf/releases");
    assert!(!manifest_files.is_empty(), "Should have manifest file");

    let manifest_path = format!("belaf/releases/{}", manifest_files[0]);
    let manifest_content = repo.read_file(&manifest_path);

    let manifest: serde_json::Value =
        serde_json::from_str(&manifest_content).expect("Manifest should be valid JSON");

    let releases = manifest["releases"].as_array().unwrap();
    assert_eq!(releases.len(), 2, "Should have two releases in manifest");

    let names: Vec<&str> = releases
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"alpha"), "Should contain alpha project");
    assert!(names.contains(&"beta"), "Should contain beta project");
}
