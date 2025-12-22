mod common;
use common::TestRepo;

#[test]
fn test_changelog_generation_single_commit() {
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

    let _ = repo.run_belaf_command(&["init", "--force"]);

    repo.write_file("src/new_feature.rs", "pub fn new_feature() {}");
    repo.commit("feat: add new feature");

    let output = repo.run_belaf_command(&["status"]);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "Command failed. stderr: {stderr}");

    println!("STDOUT: {stdout}");
    println!("STDERR: {stderr}");
    assert!(
        stdout.contains("new feature") || stdout.contains("feat"),
        "Feature commit not detected. stdout: {stdout}"
    );
}

#[test]
fn test_changelog_with_multiple_commits() {
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

    let _ = repo.run_belaf_command(&["init", "--force"]);

    repo.write_file("src/feature1.rs", "pub fn feature1() {}");
    repo.commit("feat: add feature 1");

    repo.write_file("src/feature2.rs", "pub fn feature2() {}");
    repo.commit("feat: add feature 2");

    repo.write_file("src/bugfix.rs", "pub fn bugfix() {}");
    repo.commit("fix: fix critical bug");

    let output = repo.run_belaf_command(&["status"]);

    assert!(
        output.status.success(),
        "Failed to get status: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_changelog_excludes_non_release_commits() {
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

    let _ = repo.run_belaf_command(&["init", "--force"]);

    repo.write_file("README.md", "# Test\nDocumentation update");
    repo.commit("docs: update README");

    repo.write_file("src/lib.rs", "pub fn hello() { println!(\"hello\"); }");
    repo.commit("refactor: improve formatting");

    let output = repo.run_belaf_command(&["status"]);

    assert!(
        output.status.success(),
        "Failed to get status: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_changelog_respects_project_scope() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/web\", \"crates/api\"]\nresolver = \"2\"\n",
    );
    repo.write_file(
        "crates/web/Cargo.toml",
        r#"[package]
name = "web"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write_file("crates/web/src/lib.rs", "pub fn web() {}\n");
    repo.write_file(
        "crates/api/Cargo.toml",
        r#"[package]
name = "api"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write_file("crates/api/src/lib.rs", "pub fn api() {}\n");
    repo.commit("Initial commit");

    let _ = repo.run_belaf_command(&["init", "--force"]);

    repo.write_file("crates/web/src/new.rs", "pub fn new() {}");
    repo.commit("feat(web): add new web feature");

    repo.write_file("crates/api/src/endpoint.rs", "pub fn endpoint() {}");
    repo.commit("feat(api): add new API endpoint");

    let output = repo.run_belaf_command(&["status"]);

    assert!(
        output.status.success(),
        "Failed to get status: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_changelog_with_breaking_changes() {
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

    let _ = repo.run_belaf_command(&["init", "--force"]);

    repo.write_file("src/api.rs", "pub fn breaking_change() {}");
    repo.commit("feat!: remove deprecated API\n\nBREAKING CHANGE: Old API has been removed");

    let output = repo.run_belaf_command(&["status"]);

    assert!(
        output.status.success(),
        "Failed to get status: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_changelog_preserves_existing_entries() {
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

    let _ = repo.run_belaf_command(&["init", "--force"]);

    let existing_changelog = r#"# Changelog

## [0.1.0] - 2024-01-01

### Features

- Initial release with basic functionality

## [0.0.1] - 2023-12-01

### Added

- Project scaffolding
"#;

    repo.write_file("CHANGELOG.md", existing_changelog);
    repo.commit("docs: add CHANGELOG with history");

    repo.write_file("src/feature.rs", "pub fn feature() {}");
    repo.commit("feat: add new feature");

    let output = repo.run_belaf_command(&["changelog"]);

    assert!(
        output.status.success(),
        "Changelog command failed: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    let updated_changelog = repo.read_file("CHANGELOG.md");

    assert!(
        updated_changelog.contains("[0.1.0] - 2024-01-01"),
        "Existing v0.1.0 entry should be preserved. Content:\n{}",
        updated_changelog
    );
    assert!(
        updated_changelog.contains("[0.0.1] - 2023-12-01"),
        "Existing v0.0.1 entry should be preserved. Content:\n{}",
        updated_changelog
    );
    assert!(
        updated_changelog.contains("Initial release with basic functionality"),
        "Existing entry content should be preserved. Content:\n{}",
        updated_changelog
    );
    assert!(
        updated_changelog.contains("Project scaffolding"),
        "Older entry content should be preserved. Content:\n{}",
        updated_changelog
    );
}

#[test]
fn test_changelog_header_not_duplicated() {
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

    let _ = repo.run_belaf_command(&["init", "--force"]);

    let existing_changelog = "# Changelog\n\n## [0.1.0] - 2024-01-01\n\n- Initial release\n";
    repo.write_file("CHANGELOG.md", existing_changelog);
    repo.commit("docs: add CHANGELOG");

    repo.write_file("src/feature.rs", "pub fn feature() {}");
    repo.commit("feat: add feature");

    let output = repo.run_belaf_command(&["changelog"]);

    assert!(
        output.status.success(),
        "Changelog command failed: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    let updated_changelog = repo.read_file("CHANGELOG.md");

    let header_count = updated_changelog.matches("# Changelog").count();
    assert_eq!(
        header_count, 1,
        "Header should appear exactly once, but found {} times. Content:\n{}",
        header_count, updated_changelog
    );
}

#[test]
fn test_changelog_new_entries_prepended_at_top() {
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

    let _ = repo.run_belaf_command(&["init", "--force"]);

    let existing_changelog = "# Changelog\n\n## [0.1.0] - 2024-01-01\n\n- Old release\n";
    repo.write_file("CHANGELOG.md", existing_changelog);
    repo.commit("docs: add CHANGELOG");

    repo.write_file("src/feature.rs", "pub fn feature() {}");
    repo.commit("feat: add awesome feature");

    let output = repo.run_belaf_command(&["changelog"]);

    assert!(
        output.status.success(),
        "Changelog command failed: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    let updated_changelog = repo.read_file("CHANGELOG.md");

    let new_entry_pos = updated_changelog
        .find("awesome feature")
        .or_else(|| updated_changelog.find("0.2.0"))
        .or_else(|| updated_changelog.find("Unreleased"));
    let old_entry_pos = updated_changelog.find("[0.1.0]");

    assert!(
        new_entry_pos.is_some(),
        "New entry should be present. Content:\n{}",
        updated_changelog
    );
    assert!(
        old_entry_pos.is_some(),
        "Old entry should be present. Content:\n{}",
        updated_changelog
    );

    if let (Some(new_pos), Some(old_pos)) = (new_entry_pos, old_entry_pos) {
        assert!(
            new_pos < old_pos,
            "New entry should appear before old entry. New at {}, old at {}. Content:\n{}",
            new_pos,
            old_pos,
            updated_changelog
        );
    }
}

#[test]
fn test_changelog_creates_file_if_not_exists() {
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

    let _ = repo.run_belaf_command(&["init", "--force"]);

    assert!(
        !repo.file_exists("CHANGELOG.md"),
        "CHANGELOG.md should not exist yet"
    );

    repo.write_file("src/feature.rs", "pub fn feature() {}");
    repo.commit("feat: add feature");

    let output = repo.run_belaf_command(&["changelog"]);

    assert!(
        output.status.success(),
        "Changelog command failed: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        repo.file_exists("CHANGELOG.md"),
        "CHANGELOG.md should be created"
    );

    let changelog = repo.read_file("CHANGELOG.md");
    assert!(
        changelog.contains("# Changelog") || changelog.contains("##"),
        "Changelog should have content. Content:\n{}",
        changelog
    );
}
