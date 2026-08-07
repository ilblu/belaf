//! Keeping `Cargo.lock` in step with the versions `prepare` writes.
//!
//! A release commit that carries bumped `Cargo.toml`s against an unchanged
//! `Cargo.lock` is not merely untidy — it locks the repo out of releasing. The
//! next checkout regenerates the lockfile, finds a modified working tree, and
//! `prepare` refuses to run in a dirty tree. That is a permanent stop until
//! someone syncs the lock by hand.
//!
//! So two things have to hold, and both were missing:
//!
//! - The lockfile is refreshed **whenever a Cargo manifest's version changed**,
//!   not only for auto-discovered units. Configured `[release_unit.X]` blocks
//!   write their versions through `MultiManifestRewriter`, which never went
//!   near the lockfile — in a repo where every cargo unit is configured, the
//!   lock was never updated at all.
//! - The refreshed lockfile is **added to the release commit**. Refreshing a
//!   file that is then left out of the commit changes nothing about what the
//!   PR merges.
//!
//! `cargo update --workspace` is the precise operation here: it re-records the
//! workspace members' own versions and deliberately leaves external
//! dependencies alone.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use thiserror::Error;
use toml_edit::DocumentMut;
use wait_timeout::ChildExt as _;

use crate::core::git::repository::{RepoPath, RepoPathBuf, Repository};
use crate::utils::file_io::read_config_file;

/// Wall-clock cap for any single `cargo update` invocation. Picked
/// generously enough that a real workspace recompute fits, tight
/// enough that an offline / network-stuck run doesn't hang `belaf
/// prepare` indefinitely. The previous unbounded behaviour bricked
/// CI environments without crates.io connectivity.
const CARGO_UPDATE_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Error)]
pub enum CargoLockError {
    #[error("`cargo update --workspace` failed: {stderr}")]
    UpdateFailed { stderr: String },

    #[error("failed to spawn `cargo update`: {source}")]
    Spawn {
        #[source]
        source: std::io::Error,
    },

    #[error("`cargo update {args:?}` exceeded its {timeout_sec}s timeout")]
    Timeout { args: Vec<String>, timeout_sec: u64 },
}

pub type Result<T> = std::result::Result<T, CargoLockError>;

/// Re-record the workspace members' own versions in `Cargo.lock`.
///
/// `--workspace` is the narrow operation: it syncs the members' entries and
/// leaves external dependencies at their locked versions, so a release commit
/// never smuggles in an unrelated dependency upgrade. It also sidesteps the
/// question `cargo update -p <name>` forces and that belaf kept getting wrong —
/// which cargo *package* a release unit corresponds to. A unit named after its
/// directory (`observability`) is not the crate (`clikd-observability`), and
/// the mismatch failed silently on every run.
pub fn update_workspace(cwd: &Path) -> Result<()> {
    run_cargo(&["update", "--workspace"], cwd)
}

/// Bring every `Cargo.lock` affected by this release back in step.
///
/// `changed_manifests` is the set of files the rewriters just wrote. Cargo
/// manifests among them are mapped to the workspace root that governs them,
/// deduplicated, and each workspace is synced once — one `cargo update
/// --workspace` for a repo with one workspace, however many crates bumped.
///
/// Returns the repo-relative path of every lockfile that was refreshed, so the
/// caller can add them to the release commit.
///
/// A lockfile the repo does not track is left alone entirely — not refreshed,
/// not committed. A published library commonly gitignores `Cargo.lock` while
/// still having one on disk; it is not part of what a release ships, and
/// `libgit2` would happily stage it past the ignore rules if we asked.
pub fn sync_after_rewrite<'a>(
    repo: &Repository,
    changed_manifests: impl IntoIterator<Item = &'a RepoPath>,
) -> anyhow::Result<Vec<RepoPathBuf>> {
    let repo_root = repo.resolve_workdir(&RepoPathBuf::new(b""));

    let mut workspace_roots: Vec<PathBuf> = Vec::new();
    for path in changed_manifests {
        let (_, basename) = path.split_basename();
        if basename.as_ref() != b"Cargo.toml" {
            continue;
        }
        let root = workspace_root_for(&repo.resolve_workdir(path), &repo_root);
        if !workspace_roots.contains(&root) {
            workspace_roots.push(root);
        }
    }

    let mut lockfiles = Vec::new();
    for root in workspace_roots {
        let lockfile = root.join("Cargo.lock");
        if !lockfile.exists() {
            continue;
        }
        let lockfile_repopath = repo.convert_path(&lockfile)?;
        if !repo.is_tracked(lockfile_repopath.as_ref())? {
            continue;
        }
        update_workspace(&root).map_err(|e| {
            anyhow::anyhow!(
                "failed to refresh `{}` after bumping the manifests it locks: {e}. \
                 Committing the bumped manifests against this stale lockfile would leave \
                 every later checkout with a dirty working tree, which `prepare` refuses \
                 to run in.",
                lockfile.display()
            )
        })?;
        lockfiles.push(lockfile_repopath);
    }

    Ok(lockfiles)
}

/// The cargo workspace root governing `manifest_abs`: the nearest ancestor
/// manifest carrying a `[workspace]` table, searching no further up than
/// `repo_root`. Falls back to the manifest's own directory, which is right for
/// a standalone crate — it owns its own `Cargo.lock`.
fn workspace_root_for(manifest_abs: &Path, repo_root: &Path) -> PathBuf {
    let start = manifest_abs.parent().unwrap_or(repo_root);
    let mut dir = start;
    loop {
        if let Ok(content) = read_config_file(&dir.join("Cargo.toml")) {
            if content
                .parse::<DocumentMut>()
                .is_ok_and(|doc| doc.contains_key("workspace"))
            {
                return dir.to_path_buf();
            }
        }
        if dir == repo_root {
            break;
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => break,
        }
    }
    start.to_path_buf()
}

fn run_cargo(args: &[&str], cwd: &Path) -> Result<()> {
    let mut child = Command::new("cargo")
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| CargoLockError::Spawn { source: e })?;

    // Cap the run so an offline / unreachable registry can't hang
    // `belaf prepare` indefinitely.
    let status = match child
        .wait_timeout(CARGO_UPDATE_TIMEOUT)
        .map_err(|e| CargoLockError::Spawn { source: e })?
    {
        Some(s) => s,
        None => {
            // Timeout expired — kill the child and surface a clear error.
            let _ = child.kill();
            let _ = child.wait();
            return Err(CargoLockError::Timeout {
                args: args.iter().map(|a| (*a).to_string()).collect(),
                timeout_sec: CARGO_UPDATE_TIMEOUT.as_secs(),
            });
        }
    };

    let mut stderr_buf = String::new();
    if let Some(mut s) = child.stderr.take() {
        use std::io::Read as _;
        let _ = s.read_to_string(&mut stderr_buf);
    }

    if status.success() {
        return Ok(());
    }

    Err(CargoLockError::UpdateFailed {
        stderr: stderr_buf.trim().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Build a tiny single-crate cargo project so `cargo update`
    /// has something to operate on.
    fn make_temp_crate(name: &str, version: &str) -> TempDir {
        let dir = TempDir::new().unwrap();
        let cargo_toml =
            format!("[package]\nname = \"{name}\"\nversion = \"{version}\"\nedition = \"2021\"\n");
        std::fs::write(dir.path().join("Cargo.toml"), cargo_toml).unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn x() {}\n").unwrap();
        // Generate Cargo.lock by running cargo generate-lockfile (best
        // effort — if cargo isn't available the test will skip).
        let _ = Command::new("cargo")
            .args(["generate-lockfile"])
            .current_dir(dir.path())
            .output();
        dir
    }

    #[test]
    fn update_workspace_records_the_new_manifest_version() {
        let dir = make_temp_crate("belaf-test-crate", "0.1.0");
        if !dir.path().join("Cargo.lock").exists() {
            eprintln!("cargo unavailable in test env, skipping");
            return;
        }

        // Bump the manifest the way a rewriter would, then sync.
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"belaf-test-crate\"\nversion = \"0.2.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        update_workspace(dir.path()).expect("must succeed");

        let lock = std::fs::read_to_string(dir.path().join("Cargo.lock")).unwrap();
        assert!(
            lock.contains("version = \"0.2.0\""),
            "the lockfile must follow the manifest; got:\n{lock}"
        );
    }

    #[test]
    fn update_workspace_succeeds_on_simple_project() {
        let dir = make_temp_crate("belaf-test-crate", "0.1.0");
        if !dir.path().join("Cargo.lock").exists() {
            eprintln!("cargo unavailable in test env, skipping");
            return;
        }
        update_workspace(dir.path()).expect("must succeed");
    }

    #[test]
    fn a_failing_update_surfaces_as_an_error() {
        // No Cargo.toml at all — `cargo update` cannot succeed. The point of
        // this test is the return type: a failed lock update must be
        // reportable, never swallowed into a log line.
        let dir = TempDir::new().unwrap();
        assert!(
            update_workspace(dir.path()).is_err(),
            "a failed `cargo update` must be an Err"
        );
    }
}
