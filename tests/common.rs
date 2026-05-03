#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

pub struct TestRepo {
    _dir: TempDir,
    pub path: PathBuf,
}

impl Default for TestRepo {
    fn default() -> Self {
        Self::new()
    }
}

impl TestRepo {
    #[must_use]
    pub fn new() -> Self {
        let dir = TempDir::new().expect("failed to create temp dir");
        let path = dir.path().to_path_buf();

        Self::init_git(&path);

        Self { _dir: dir, path }
    }

    fn init_git(path: &Path) {
        Command::new("git")
            .args(["init"])
            .current_dir(path)
            .output()
            .expect("failed to init git");

        Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(path)
            .output()
            .expect("failed to set git email");

        Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(path)
            .output()
            .expect("failed to set git name");

        Command::new("git")
            .args([
                "remote",
                "add",
                "origin",
                "https://github.com/test/repo.git",
            ])
            .current_dir(path)
            .output()
            .expect("failed to add remote");
    }

    pub fn write_file(&self, relative_path: &str, content: &str) {
        let full_path = self.path.join(relative_path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent).expect("failed to create parent dirs");
        }
        std::fs::write(full_path, content).expect("failed to write file");
    }

    pub fn commit(&self, message: &str) {
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(&self.path)
            .output()
            .expect("failed to git add");

        Command::new("git")
            .args(["commit", "-m", message])
            .current_dir(&self.path)
            .output()
            .expect("failed to git commit");
    }

    #[must_use]
    pub fn run_belaf_command(&self, args: &[&str]) -> std::process::Output {
        let belaf_bin = env!("CARGO_BIN_EXE_belaf");

        Command::new(belaf_bin)
            .args(args)
            .current_dir(&self.path)
            .env("GITHUB_TOKEN", "test-token-for-tests")
            .output()
            .expect("failed to run belaf command")
    }

    #[must_use]
    pub fn file_exists(&self, relative_path: &str) -> bool {
        self.path.join(relative_path).exists()
    }

    #[must_use]
    pub fn read_file(&self, relative_path: &str) -> String {
        std::fs::read_to_string(self.path.join(relative_path)).expect("failed to read file")
    }

    #[must_use]
    pub fn has_config_dir(&self) -> bool {
        self.path.join("belaf").is_dir()
    }

    #[must_use]
    pub fn list_files_in_dir(&self, relative_dir: &str) -> Vec<String> {
        let dir_path = self.path.join(relative_dir);
        if !dir_path.exists() {
            return Vec::new();
        }
        std::fs::read_dir(dir_path)
            .map(|entries| {
                entries
                    .filter_map(std::result::Result::ok)
                    .filter_map(|e| e.file_name().into_string().ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    #[must_use]
    pub fn run_belaf_command_with_env(
        &self,
        args: &[&str],
        env_vars: &[(&str, &str)],
    ) -> std::process::Output {
        let belaf_bin = env!("CARGO_BIN_EXE_belaf");

        let mut cmd = Command::new(belaf_bin);
        cmd.args(args).current_dir(&self.path);

        for (key, value) in env_vars {
            cmd.env(key, value);
        }

        cmd.output().expect("failed to run belaf command")
    }
}
