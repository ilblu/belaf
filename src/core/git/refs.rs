//! Branch, push and fetch operations against the working repository and
//! its upstream remote.

use anyhow::Context;
use tracing::info;
use uuid::Uuid;

use crate::core::{
    errors::Result,
    git::repository::{ChangeList, RepoPath, Repository},
};

/// Default `[repo] release_branch`. See [`Repository::release_branch_name`].
pub const DEFAULT_RELEASE_BRANCH_TEMPLATE: &str = "belaf/release--{base}";

impl Repository {
    /// Update the specified files in the working tree to reset them to what
    /// HEAD says they should be.
    pub fn hard_reset_changes(&self, changes: &ChangeList) -> Result<()> {
        // If no changes, do nothing. If we don't special-case this, the
        // checkout_head() will affect *all* files, i.e. perform a hard reset to
        // HEAD.
        if changes.paths.is_empty() {
            return Ok(());
        }

        let mut cb = git2::build::CheckoutBuilder::new();
        cb.force();

        // The key is that by specifying paths here, the checkout operation will
        // only affect those paths and not anything else.
        for path in &changes.paths[..] {
            let p: &RepoPath = path.as_ref();
            cb.path(p);
        }

        self.repo.checkout_head(Some(&mut cb))?;
        Ok(())
    }

    /// Create a branch at the current HEAD.
    ///
    /// With `force`, an existing branch of that name is moved to HEAD rather
    /// than erroring. `prepare` needs that: it reuses one release branch, so
    /// a leftover branch from an earlier run — including one a failed run
    /// left behind — has to be reset to the base commit, not rejected.
    pub fn create_branch(&self, name: &str, force: bool) -> Result<()> {
        let head_ref = self.repo.head()?;
        let head_commit = head_ref.peel_to_commit()?;
        self.repo.branch(name, &head_commit, force)?;
        info!("created branch {}", name);
        Ok(())
    }

    pub fn checkout_branch(&self, name: &str) -> Result<()> {
        let branch_ref = format!("refs/heads/{}", name);
        let obj = self
            .repo
            .revparse_single(&branch_ref)
            .with_context(|| format!("branch '{}' not found", name))?;

        self.repo.checkout_tree(&obj, None)?;
        self.repo.set_head(&branch_ref)?;
        info!("checked out branch {}", name);
        Ok(())
    }

    pub fn delete_branch(&self, name: &str) -> Result<()> {
        let mut branch = self
            .repo
            .find_branch(name, git2::BranchType::Local)
            .with_context(|| format!("branch '{}' not found", name))?;

        branch
            .delete()
            .with_context(|| format!("failed to delete branch '{}'", name))?;

        info!("deleted branch {}", name);
        Ok(())
    }

    /// Fetch tags from the configured upstream remote.
    ///
    /// Why this exists: tag-reading helpers like
    /// [`Self::find_latest_tag_for_project`] only see *local* refs.
    /// Release tags are typically created server-side (by the belaf
    /// GitHub App when a release PR merges), and `git pull --ff-only`
    /// does not fetch tags. Without an explicit fetch, the CLI
    /// picks a stale baseline and inflates the bump.
    ///
    /// Uses refspec `+refs/tags/*:refs/tags/*` (force) so that
    /// re-tagged versions (rare but legal) overwrite the local copy
    /// instead of erroring out. Mirrors the credential pattern from
    /// [`Self::push_branch`].
    pub fn fetch_tags(&self, git_token: Option<&str>) -> Result<()> {
        let mut remote = self
            .repo
            .find_remote(&self.upstream_name)
            .with_context(|| format!("cannot find upstream remote `{}`", self.upstream_name))?;

        let token_for_closure = git_token.map(str::to_owned);

        let mut callbacks = git2::RemoteCallbacks::new();
        callbacks.credentials(move |_url, username_from_url, allowed_types| {
            if allowed_types.contains(git2::CredentialType::SSH_KEY) {
                git2::Cred::ssh_key_from_agent(username_from_url.unwrap_or("git"))
            } else if allowed_types.contains(git2::CredentialType::USER_PASS_PLAINTEXT) {
                if let Some(ref token) = token_for_closure {
                    git2::Cred::userpass_plaintext("x-access-token", token)
                } else {
                    git2::Cred::default()
                }
            } else {
                git2::Cred::default()
            }
        });

        let mut fetch_options = git2::FetchOptions::new();
        fetch_options.remote_callbacks(callbacks);
        fetch_options.download_tags(git2::AutotagOption::All);

        remote
            .fetch(
                &["+refs/tags/*:refs/tags/*"],
                Some(&mut fetch_options),
                None,
            )
            .with_context(|| {
                format!(
                    "failed to fetch tags from `{}`. belaf needs an up-to-date \
                     view of release tags to pick the right baseline — check \
                     network connectivity and that you have credentials for the \
                     remote (SSH agent or git token). Set BELAF_NO_FETCH=1 to skip.",
                    self.upstream_name
                )
            })?;

        info!("fetched tags from {}", self.upstream_name);
        Ok(())
    }

    /// Push a local branch to the upstream remote.
    ///
    /// `force` prefixes the refspec with `+`, which `prepare` needs because
    /// it rebuilds its release branch from the base commit on every run.
    ///
    /// Invariant: the caller must push a branch that *already* carries the
    /// release commit, in this one operation. Never reset the remote branch
    /// to the base commit first — GitHub closes a pull request whose head
    /// branch no longer sits ahead of its base, which would orphan the open
    /// release PR and force a new one on the next run.
    pub fn push_branch(
        &self,
        branch_name: &str,
        git_token: Option<&str>,
        force: bool,
    ) -> Result<()> {
        let mut remote = self.repo.find_remote(&self.upstream_name)?;
        let refspec = format!(
            "{}refs/heads/{}:refs/heads/{}",
            if force { "+" } else { "" },
            branch_name,
            branch_name
        );

        let token_for_closure = git_token.map(|s| s.to_string());

        let mut callbacks = git2::RemoteCallbacks::new();
        callbacks.credentials(move |_url, username_from_url, allowed_types| {
            if allowed_types.contains(git2::CredentialType::SSH_KEY) {
                git2::Cred::ssh_key_from_agent(username_from_url.unwrap_or("git"))
            } else if allowed_types.contains(git2::CredentialType::USER_PASS_PLAINTEXT) {
                if let Some(ref token) = token_for_closure {
                    git2::Cred::userpass_plaintext("x-access-token", token)
                } else {
                    git2::Cred::default()
                }
            } else {
                git2::Cred::default()
            }
        });

        let mut push_options = git2::PushOptions::new();
        push_options.remote_callbacks(callbacks);

        remote.push(&[&refspec], Some(&mut push_options))?;
        info!("pushed branch {} to {}", branch_name, self.upstream_name);
        Ok(())
    }

    /// The release branch this repo uses for `base_branch`.
    ///
    /// Stable by design: re-running `prepare` targets the same branch and
    /// force-updates it, which is what keeps one open release PR up to date
    /// instead of opening a new one per run.
    ///
    /// `{base}` is part of the default template because releasing from
    /// `main` and from a maintenance branch are separate release trains —
    /// one global branch name would let one clobber the other's PR.
    pub fn release_branch_name(&self, base_branch: &str) -> Result<String> {
        let template = self
            .release_branch_template
            .as_deref()
            .unwrap_or(DEFAULT_RELEASE_BRANCH_TEMPLATE);

        // `/` is legal in a ref name, but `belaf/release--feature/x` would
        // collide with a `belaf/release--feature` ref (git cannot hold both a
        // ref and a directory of the same name).
        let name = template.replace("{base}", &base_branch.replace('/', "-"));

        crate::core::tag_format::validate_git_ref_format(&name).with_context(|| {
            format!(
                "release branch template `{}` produced an invalid branch name for base \
                 branch `{}` — fix `[repo] release_branch` in belaf/config.toml",
                template, base_branch
            )
        })?;

        Ok(name)
    }

    /// Whether `branch` is itself one of this repo's release branches.
    ///
    /// A run that failed after creating its branch leaves HEAD there. Naming
    /// the next release branch after it would produce
    /// `belaf/release--belaf-release--main` and, worse, a pull request based
    /// on the release branch instead of the real base — so callers refuse
    /// rather than guess.
    pub fn is_release_branch(&self, branch: &str) -> bool {
        let template = self
            .release_branch_template
            .as_deref()
            .unwrap_or(DEFAULT_RELEASE_BRANCH_TEMPLATE);

        match template.split_once("{base}") {
            Some((prefix, suffix)) => {
                branch.len() > prefix.len() + suffix.len()
                    && branch.starts_with(prefix)
                    && branch.ends_with(suffix)
            }
            // A template without `{base}` is one fixed branch name.
            None => branch == template,
        }
    }

    /// A unique, timestamped branch name.
    ///
    /// Used when the caller deliberately wants a release PR *alongside* the
    /// existing one rather than updating it — the interactive wizard offers
    /// this; `--ci` never does.
    pub fn generate_release_branch_name() -> String {
        let now = time::OffsetDateTime::now_utc();
        let formatted =
            time::format_description::parse("[year][month][day]-[hour][minute][second]")
                .ok()
                .and_then(|format| now.format(&format).ok())
                .unwrap_or_else(|| now.unix_timestamp().to_string());
        let suffix = &Uuid::new_v4().to_string()[..8];

        format!("release/{}-{}", formatted, suffix)
    }
}
