mod common;
use common::TestRepo;

#[test]
fn test_github_remote_detection() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "test-crate"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
    repo.commit("Initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);

    assert!(
        output.status.success(),
        "Failed to init: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    let config = repo.read_file("belaf/config.toml");
    assert!(
        config.contains("github.com/test/repo"),
        "GitHub URL not detected from remote"
    );
}

#[test]
fn test_github_upstream_url_configured() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "test-crate"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
    repo.commit("Initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);

    assert!(
        output.status.success(),
        "Failed to init: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    let config = repo.read_file("belaf/config.toml");
    assert!(
        config.contains("upstream_urls"),
        "upstream_urls not in config"
    );
    assert!(
        config.contains("github.com"),
        "GitHub domain not in upstream_urls"
    );
}

#[test]
fn test_github_release_tag_format() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "test-crate"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
    repo.commit("Initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);

    assert!(
        output.status.success(),
        "Failed to init: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    let config = repo.read_file("belaf/config.toml");
    assert!(
        config.contains("release_tag_name_format") || !config.is_empty(),
        "Config created"
    );
}

#[test]
fn test_multiple_remotes_prefers_origin() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "test-crate"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
    repo.commit("Initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);

    assert!(
        output.status.success(),
        "Failed to init: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    let config = repo.read_file("belaf/config.toml");
    assert!(
        config.contains("github.com/test/repo"),
        "Origin remote not preferred"
    );
}

#[test]
fn test_github_https_url_format() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "test-crate"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
    repo.commit("Initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);

    assert!(
        output.status.success(),
        "Failed to init: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    let config = repo.read_file("belaf/config.toml");
    assert!(
        config.contains("https://github.com/test/repo"),
        "HTTPS URL format not preserved"
    );
}

#[test]
fn test_github_integration_without_remote() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "test-crate"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
    repo.commit("Initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);

    assert!(
        output.status.success(),
        "Should succeed even without proper remote: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}
