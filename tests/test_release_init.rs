mod common;
use common::TestRepo;

#[test]
fn test_release_init_creates_config_directory() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        "[package]\nname = \"test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}");
    repo.commit("Initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);

    assert!(
        output.status.success(),
        "Command failed: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(repo.has_config_dir(), "belaf directory not created");
    assert!(
        repo.file_exists("belaf/config.toml"),
        "config.toml not created"
    );
    assert!(
        repo.file_exists("belaf/bootstrap.toml"),
        "bootstrap.toml not created"
    );
}

#[test]
fn test_release_init_detects_single_cargo_project() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "my-crate"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
    repo.commit("Initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);

    assert!(output.status.success());

    let bootstrap = repo.read_file("belaf/bootstrap.toml");
    assert!(
        bootstrap.contains("my-crate"),
        "Project name not in bootstrap.toml"
    );
    assert!(bootstrap.contains("0.1.0"), "Version not in bootstrap.toml");
}

#[test]
fn test_release_init_sets_upstream_url() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "test-crate"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
    repo.commit("Initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);

    assert!(output.status.success());

    let config = repo.read_file("belaf/config.toml");
    assert!(
        config.contains("upstream_urls"),
        "upstream_urls not in config"
    );
    assert!(
        config.contains("github.com/test/repo"),
        "upstream URL not set correctly"
    );
}

#[test]
fn test_release_init_fails_with_dirty_repo() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "test"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
    repo.commit("Initial commit");

    repo.write_file("README.md", "# Test");

    let output = repo.run_belaf_command(&["init"]);

    assert!(!output.status.success(), "Should fail with dirty repo");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("uncommitted changes") || stderr.contains("refusing to proceed"));
}

#[test]
fn test_release_init_with_force_allows_dirty_repo() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "test"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
    repo.commit("Initial commit");

    repo.write_file("README.md", "# Test");

    let output = repo.run_belaf_command(&["init", "--force"]);

    assert!(output.status.success(), "Should succeed with --force");
    assert!(repo.has_config_dir());
}

#[test]
fn test_release_init_does_not_overwrite_existing_config() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "test"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
    repo.commit("Initial commit");

    let _ = repo.run_belaf_command(&["init", "--force"]);

    let original_config = repo.read_file("belaf/config.toml");

    let output = repo.run_belaf_command(&["init", "--force"]);

    assert!(output.status.success());
    let new_config = repo.read_file("belaf/config.toml");
    assert_eq!(
        original_config, new_config,
        "Config should not be overwritten"
    );
}
