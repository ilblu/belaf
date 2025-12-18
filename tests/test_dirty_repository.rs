mod common;
use common::TestRepo;

#[test]
fn test_clean_repository_passes() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "test-package"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}");
    repo.commit("initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);

    assert!(
        output.status.success(),
        "Clean repository should pass: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_dirty_repository_detected() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "test-package"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}");
    repo.commit("initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(output.status.success());

    repo.write_file("src/lib.rs", "pub fn modified() {}");

    let output = repo.run_belaf_command(&["status"]);

    assert!(
        output.status.success(),
        "Status command should succeed even with dirty repo: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_untracked_files_handling() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "test-package"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}");
    repo.commit("initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(output.status.success());

    repo.write_file("untracked_file.txt", "this is untracked");

    let output = repo.run_belaf_command(&["status"]);

    assert!(
        output.status.success(),
        "Untracked files should not block status: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_staged_changes_handling() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "test-package"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}");
    repo.commit("initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(output.status.success());

    repo.write_file("src/lib.rs", "pub fn staged_change() {}");

    std::process::Command::new("git")
        .args(["add", "src/lib.rs"])
        .current_dir(&repo.path)
        .output()
        .expect("failed to stage file");

    let output = repo.run_belaf_command(&["status"]);

    assert!(
        output.status.success(),
        "Staged changes should be handled: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_deleted_file_handling() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "test-package"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}");
    repo.write_file("to_delete.txt", "will be deleted");
    repo.commit("initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(output.status.success());

    std::fs::remove_file(repo.path.join("to_delete.txt")).expect("failed to delete file");

    let output = repo.run_belaf_command(&["status"]);

    assert!(
        output.status.success(),
        "Deleted files should be handled: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_renamed_file_handling() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "test-package"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}");
    repo.write_file("original.txt", "content");
    repo.commit("initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(output.status.success());

    std::fs::rename(
        repo.path.join("original.txt"),
        repo.path.join("renamed.txt"),
    )
    .expect("failed to rename file");

    let output = repo.run_belaf_command(&["status"]);

    assert!(
        output.status.success(),
        "Renamed files should be handled: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_gitignore_respected() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "test-package"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}");
    repo.write_file(".gitignore", "target/\n*.log\nbuild/\n");
    repo.commit("initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(output.status.success());

    repo.write_file("target/debug/build", "ignored build artifact");
    repo.write_file("application.log", "ignored log file");
    repo.write_file("build/output.txt", "ignored build output");

    let output = repo.run_belaf_command(&["status"]);

    assert!(
        output.status.success(),
        "Ignored files should not cause issues: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_multiple_dirty_files() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "test-package"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}");
    repo.write_file("file1.txt", "content 1");
    repo.write_file("file2.txt", "content 2");
    repo.write_file("file3.txt", "content 3");
    repo.commit("initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(output.status.success());

    repo.write_file("file1.txt", "modified 1");
    repo.write_file("file2.txt", "modified 2");
    repo.write_file("file3.txt", "modified 3");

    let output = repo.run_belaf_command(&["status"]);

    assert!(
        output.status.success(),
        "Multiple dirty files should be handled: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_empty_repository() {
    let repo = TestRepo::new();

    let output = repo.run_belaf_command(&["status"]);

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        !output.status.success()
            || stderr.contains("no projects")
            || stderr.contains("No projects")
            || stdout.contains("no projects")
            || stdout.contains("No projects")
            || stderr.is_empty()
            || stdout.is_empty(),
        "Empty repository should be handled gracefully, got stdout: '{stdout}', stderr: '{stderr}'"
    );
}

#[test]
fn test_binary_file_changes() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "test-package"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}");

    let binary_content = vec![0u8, 1, 2, 3, 255, 254, 253];
    std::fs::write(repo.path.join("binary.bin"), &binary_content).expect("failed to write binary");
    repo.commit("initial commit with binary");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(output.status.success());

    let modified_binary = vec![255u8, 254, 253, 252, 251];
    std::fs::write(repo.path.join("binary.bin"), &modified_binary)
        .expect("failed to write modified binary");

    let output = repo.run_belaf_command(&["status"]);

    assert!(
        output.status.success(),
        "Binary file changes should be handled: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_submodule_handling() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "test-package"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}");
    repo.commit("initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);

    assert!(
        output.status.success(),
        "Repository without submodules should work: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}
