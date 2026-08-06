//! Release-branch lifecycle helpers.
//!
//! Creating and tearing down the throwaway branch that carries the release
//! commit, plus the commit-message formatting for that commit.

use anyhow::{Context, Result};
use tracing::info;

use crate::core::{git::repository::Repository, session::AppSession};

use super::SelectedReleaseUnit;

/// Put the session on a release branch built from the current HEAD, and
/// report `(base_branch, release_branch)`.
///
/// `separate` asks for a throwaway timestamped branch instead of the repo's
/// stable release branch, so the run ends up in a release PR of its own
/// rather than updating the open one. Only the interactive wizard offers
/// that; `--ci` always takes the stable branch.
pub fn create_release_branch(sess: &mut AppSession, separate: bool) -> Result<(String, String)> {
    let base_branch = sess
        .repo
        .current_branch_name()
        .context("failed to get current branch")?
        .ok_or_else(|| anyhow::anyhow!("not on a branch (detached HEAD state)"))?;

    // Starting from a release branch would name the next one after it and,
    // worse, base the pull request on it. This is where a run that failed
    // after creating its branch leaves you, so say what to do about it.
    if sess.repo.is_release_branch(&base_branch) {
        anyhow::bail!(
            "`{base_branch}` is a belaf release branch, not a base branch. A previous \
             run most likely failed after creating it. Switch back to the branch you \
             release from (e.g. `git switch main`) and run again — the release branch \
             is rebuilt from scratch each time, so nothing is lost."
        );
    }

    let release_branch = if separate {
        Repository::generate_release_branch_name()
    } else {
        sess.repo.release_branch_name(&base_branch)?
    };
    info!("creating release branch: {}", release_branch);

    // Force: the stable branch usually already exists, whether from the run
    // that opened the PR we are about to update or from a run that failed
    // partway. Either way it has to be reset to the base commit.
    sess.repo
        .create_branch(&release_branch, true)
        .context("failed to create release branch")?;
    // Not a forced checkout: the branch points at HEAD, so the working tree
    // does not change, and uncommitted work survives (the wizard permits a
    // dirty tree).
    sess.repo
        .checkout_branch(&release_branch)
        .context("failed to checkout release branch")?;

    Ok((base_branch, release_branch))
}

pub fn cleanup_release_branch(sess: &mut AppSession, base_branch: &str, release_branch: &str) {
    if let Err(e) = sess.repo.checkout_branch(base_branch) {
        tracing::warn!("failed to checkout base branch '{}': {}", base_branch, e);
    }

    if let Err(e) = sess.repo.delete_branch(release_branch) {
        tracing::warn!(
            "failed to delete release branch '{}': {}",
            release_branch,
            e
        );
    }

    info!("cleaned up release branch, returned to '{}'", base_branch);
}

pub(super) fn format_commit_message(projects: &[SelectedReleaseUnit]) -> String {
    if projects.len() == 1 {
        let p = &projects[0];
        format!(
            "chore(release): {} v{}\n\n\
            Bump {} from {} to {}",
            p.name, p.new_version, p.name, p.old_version, p.new_version
        )
    } else {
        let mut msg = format!("chore(release): release {} packages\n\n", projects.len());
        for p in projects {
            msg.push_str(&format!(
                "- {}: {} -> {}\n",
                p.name, p.old_version, p.new_version
            ));
        }
        msg
    }
}
