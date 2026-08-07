// Copyright 2020 Peter Williams <peter@newton.cx> and collaborators
// Licensed under the MIT License.

//! State of the backing version control repository.

use anyhow::{anyhow, bail, Context};

use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};
use thiserror::Error as ThisError;
use tracing::{info, warn};

use crate::{
    atry,
    core::{
        config::syntax::RepoConfiguration, errors::Result, resolved_release_unit::DepRequirement,
    },
};

// These types used to live in this module and are imported from here across
// the crate; re-export them so those paths keep resolving after the split.
pub use crate::core::git::history::{HistoryBoundary, RepoHistory};
pub use crate::core::git::path_matcher::{is_binary_affecting, PathMatcher};
pub use crate::core::git::release_info::{
    ChangeList, ReleaseAvailability, ReleaseCommitInfo, ReleasedProjectInfo,
};
pub use crate::core::git::repo_path::{escape_pathlike, RepoPath, RepoPathBuf};

/// Opaque type representing a commit in the repository.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct CommitId(pub(super) git2::Oid);

impl std::fmt::Display for CommitId {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// An empty error returned when the backing repository is "bare", without a
/// working directory. Belaf cannot operate on such repositories.
#[derive(Debug, ThisError)]
#[error("cannot operate on a bare repository")]
pub struct BareRepositoryError;

/// An error returned when the backing repository is "dirty", i.e. there are
/// modified files, and this has situation has been deemed unacceptable. The
/// inner value is one of the culprit paths.
#[derive(Debug, ThisError)]
pub struct DirtyRepositoryError(pub RepoPathBuf);

/// An error returned when some metadata references a commit in the repository,
/// and that reference is bogus. The inner value is the text of the reference.
#[derive(Debug, ThisError)]
#[error("commit reference `{0}` is invalid or refers to a nonexistent commit")]
pub struct InvalidHistoryReferenceError(pub String);

impl std::fmt::Display for DirtyRepositoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "the file backing repository is dirty: file {} has been modified",
            self.0.escaped()
        )
    }
}

/// Information about the backing version control repository.
pub struct Repository {
    /// The underlying `git2` repository object.
    pub(super) repo: git2::Repository,

    /// The name of the "upstream" remote.
    pub(super) upstream_name: String,

    /// Analysis configuration for LRU cache sizes.
    pub(super) analysis_config: crate::core::config::syntax::AnalysisConfig,

    /// `[repo] release_branch` — see [`Repository::release_branch_name`].
    /// `None` until `apply_config` runs, and when the key is unset.
    pub(super) release_branch_template: Option<String>,
}

impl Repository {
    /// Open the repository using standard environmental cues.
    ///
    /// Initialization may fail if the process is not running inside a Git
    /// repository and the necessary Git environment variables are missing, if
    /// the repository is "bare" (has no working directory), if there is some
    /// data corruption issue, etc.
    ///
    /// If the repository is "bare", an error downcastable into
    /// BareRepositoryError will be returned.
    pub fn open_from_env() -> Result<Repository> {
        let repo = git2::Repository::open_from_env()?;

        if repo.is_bare() {
            return Err(BareRepositoryError.into());
        }

        let upstream_name = "origin".to_owned();

        Ok(Repository {
            repo,
            upstream_name,
            analysis_config: crate::core::config::syntax::AnalysisConfig {
                commit_cache_size: 512,
                tree_cache_size: 3,
            },
            release_branch_template: None,
        })
    }

    /// Open a repository at an explicit path. Mirrors
    /// [`Self::open_from_env`] but takes an explicit path so callers
    /// (e.g. integration tests) don't need to mutate the process-wide
    /// current working directory.
    ///
    /// Assumes:
    /// - `upstream_name = "origin"` — the standard remote name. Repos
    ///   that use a different name need [`Self::bootstrap_upstream`]
    ///   or [`Self::open_with`] to override.
    /// - `commit_cache_size = 512`, `tree_cache_size = 3` — the same
    ///   defaults `open_from_env` uses when no `belaf/config.toml`
    ///   has overridden them.
    ///
    /// Most callers should use this; the `_with` variant is for
    /// callers that need to construct a Repository with non-default
    /// config (rare in practice — tests and scripts).
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Repository> {
        Self::open_with(
            path,
            "origin",
            crate::core::config::syntax::AnalysisConfig {
                commit_cache_size: 512,
                tree_cache_size: 3,
            },
        )
    }

    /// Open a repository with explicit upstream name and analysis
    /// config. Used by callers that don't want to inherit `open`'s
    /// defaults (forks with non-`origin` remotes, callers tuning
    /// cache sizes for very large repos, etc.).
    pub fn open_with<P: AsRef<Path>>(
        path: P,
        upstream_name: &str,
        analysis_config: crate::core::config::syntax::AnalysisConfig,
    ) -> Result<Repository> {
        let repo = git2::Repository::open(path.as_ref())?;
        if repo.is_bare() {
            return Err(BareRepositoryError.into());
        }
        Ok(Repository {
            repo,
            upstream_name: upstream_name.to_owned(),
            analysis_config,
            release_branch_template: None,
        })
    }

    /// Set up the upstream info in when bootstrapping.
    pub fn bootstrap_upstream(&mut self, name: Option<&str>) -> Result<String> {
        if let Some(name) = name {
            crate::core::git::validate::validate_remote_name(name)
                .context("invalid remote name")?;
        }

        let upstream_url = if let Some(name) = name {
            let remote = atry!(
                self.repo.find_remote(name);
                ["cannot look up the Git remote named `{}`", name]
            );

            remote
                .url()
                .ok_or_else(|| {
                    anyhow!(
                        "the URL of Git remote `{}` cannot be interpreted as UTF8",
                        name
                    )
                })?
                .to_owned()
        } else {
            let mut info = None;
            let mut n_remotes = 0;

            // `None` happens if a remote name is not valid UTF8. At the moment
            // I can't be bothered to properly handle that, so we just skip those
            // with the `flatten()`
            for remote_name in self.repo.remotes()?.into_iter().flatten() {
                n_remotes += 1;
                match self.repo.find_remote(remote_name) {
                    Err(e) => {
                        warn!("error querying Git remote `{}`: {}", remote_name, e);
                    }

                    Ok(remote) => {
                        if let Some(remote_url) = remote.url() {
                            if info.is_none() || remote_name == "origin" {
                                info = Some((remote_name.to_owned(), remote_url.to_owned()));
                            }
                        }
                    }
                }
            }

            let (name, url) = info.ok_or_else(|| anyhow!("no usable remotes in the Git repo"))?;

            if n_remotes > 1 && name != "origin" {
                bail!("no way to choose among multiple Git remotes");
            }

            info!("using Git remote `{}` as the upstream", name);
            url
        };

        Ok(upstream_url)
    }

    /// Update the repository configuration with values read from the config file.
    pub fn apply_config(&mut self, cfg: RepoConfiguration) -> Result<()> {
        // Get the name of the upstream remote. If there's only one remote, we
        // use it. If we're given a list of URLs and one matches, we use that.
        // If no URLs match but there is a remote named "origin", use that.

        let mut first_upstream_name = None;
        let mut n_remotes = 0;
        let mut url_matched = None;
        let mut saw_origin = false;

        for remote_name in &self.repo.remotes()? {
            // `None` happens if a remote name is not valid UTF8. At the moment
            // I can't be bothered to properly handle that.
            if let Some(remote_name) = remote_name {
                n_remotes += 1;

                if first_upstream_name.is_none() {
                    first_upstream_name = Some(remote_name.to_owned());
                }

                if remote_name == "origin" {
                    saw_origin = true;
                }

                match self.repo.find_remote(remote_name) {
                    Err(e) => {
                        warn!("error querying Git remote `{}`: {}", remote_name, e);
                    }

                    Ok(remote) => {
                        if let Some(remote_url) = remote.url() {
                            for url in &cfg.upstream_urls {
                                if remote_url == url {
                                    url_matched = Some(remote_name.to_owned());
                                    break;
                                }
                            }
                        }
                    }
                }
            }

            if url_matched.is_some() {
                break;
            }
        }

        self.upstream_name = if let Some(n) = url_matched {
            n
        } else if n_remotes == 1 {
            first_upstream_name.ok_or_else(|| anyhow!("remote name is not valid UTF-8"))?
        } else if saw_origin {
            "origin".to_owned()
        } else {
            bail!("cannot identify the upstream Git remote");
        };

        self.analysis_config = cfg.analysis;
        self.release_branch_template = cfg.release_branch;
        Ok(())
    }

    /// Get the URL of the upstream repository.
    pub fn upstream_url(&self) -> Result<String> {
        let upstream = self.repo.find_remote(&self.upstream_name)?;
        Ok(upstream
            .url()
            .ok_or_else(|| {
                anyhow!(
                    "URL of upstream remote {} not parseable as Unicode",
                    self.upstream_name
                )
            })?
            .to_owned())
    }

    /// Get the name of the currently active branch, if there is one.
    ///
    /// There might not be such a branch if the repository is in a "detached
    /// HEAD" state, for instance.
    pub fn current_branch_name(&self) -> Result<Option<String>> {
        let head_ref = self.repo.head()?;

        Ok(if !head_ref.is_branch() {
            None
        } else {
            Some(
                head_ref
                    .shorthand()
                    .ok_or_else(|| anyhow!("current branch name not Unicode"))?
                    .to_owned(),
            )
        })
    }

    /// Parse a textual reference to a commit within the repository.
    pub fn parse_history_ref<T: AsRef<str>>(&self, text: T) -> Result<ParsedHistoryRef> {
        let text = text.as_ref();

        if let Ok(id) = text.parse() {
            Ok(ParsedHistoryRef::Id(CommitId(id)))
        } else if let Some(tctext) = text.strip_prefix("thiscommit:") {
            Ok(ParsedHistoryRef::ThisCommit {
                salt: tctext.to_owned(),
            })
        } else if let Some(manual_text) = text.strip_prefix("manual:") {
            Ok(ParsedHistoryRef::Manual(manual_text.to_owned()))
        } else {
            Err(InvalidHistoryReferenceError(text.to_owned()).into())
        }
    }

    /// Resolve a parsed history reference to its specific value.
    pub fn resolve_history_ref(
        &self,
        href: &ParsedHistoryRef,
        ref_source_path: &RepoPath,
    ) -> Result<DepRequirement> {
        let cid = match href {
            ParsedHistoryRef::Id(id) => *id,
            ParsedHistoryRef::ThisCommit { ref salt } => lookup_this(self, salt, ref_source_path)?,
            ParsedHistoryRef::Manual(t) => return Ok(DepRequirement::Manual(t.clone())),
        };

        // Double-check that the ID actually resolves to a commit.
        self.repo.find_commit(cid.0)?;
        return Ok(DepRequirement::Commit(cid));

        fn lookup_this(
            repo: &Repository,
            salt: &str,
            ref_source_path: &RepoPath,
        ) -> Result<CommitId> {
            let file = File::open(repo.resolve_workdir(ref_source_path))?;
            let reader = BufReader::new(file);
            let mut line_no = 1; // blames start at line 1.
            let mut found_it = false;

            for maybe_line in reader.lines() {
                let line = maybe_line?;
                if line.contains(salt) {
                    found_it = true;
                    break;
                }

                line_no += 1;
            }

            if !found_it {
                return Err(anyhow!(
                    "commit-ref key `{}` not found in contents of file {}",
                    salt,
                    ref_source_path.escaped(),
                ));
            }

            let blame = repo.repo.blame_file(ref_source_path.as_path(), None)?;
            let hunk = blame.get_line(line_no).ok_or_else(|| {
                anyhow!(
                    "commit-ref key `{}` found in uncommitted or non-existent line {} of file {}. \
                     The line must be committed before it can be referenced.",
                    salt,
                    line_no,
                    ref_source_path.escaped()
                )
            })?;

            Ok(CommitId(hunk.final_commit_id()))
        }
    }

    /// Resolve a `RepoPath` repository path to a filesystem path in the working
    /// directory.
    pub fn resolve_workdir(&self, p: &RepoPath) -> PathBuf {
        let mut fullpath = self
            .repo
            .workdir()
            .expect("BUG: workdir() should never be None as bare repos are rejected at open()")
            .to_owned();
        fullpath.push(p.as_path());
        fullpath
    }

    /// Resolve the path to the per-repository configuration directory.
    pub fn resolve_config_dir(&self) -> PathBuf {
        self.resolve_workdir(RepoPath::new(b"belaf"))
    }

    /// Convert a filesystem path pointing inside the working directory into a
    /// RepoPathBuf.
    ///
    /// Some external tools (e.g. `cargo metadata`) make it so that it is useful
    /// to be able to do this reverse conversion.
    pub fn convert_path<P: AsRef<Path>>(&self, p: P) -> Result<RepoPathBuf> {
        let c_root = self
            .repo
            .workdir()
            .expect("BUG: workdir() should never be None as bare repos are rejected at open()")
            .canonicalize()?;
        let c_p = p.as_ref().canonicalize()?;
        let rel = c_p.strip_prefix(&c_root).map_err(|_| {
            anyhow!(
                "path `{}` lies outside of the working directory",
                c_p.display()
            )
        })?;
        RepoPathBuf::from_path(rel)
    }

    /// Scan the paths in the repository index.
    pub fn scan_paths<F>(&self, mut f: F) -> Result<()>
    where
        F: FnMut(&RepoPath) -> Result<()>,
    {
        self.scan_paths_with_progress(|p, _, _| f(p))
    }

    /// Get the number of entries in the repository index.
    pub fn index_entry_count(&self) -> Result<usize> {
        let index = self.repo.index()?;
        Ok(index.len())
    }

    /// Whether `path` is tracked by git — i.e. present in the index.
    ///
    /// Distinct from "exists on disk": a generated file the repo deliberately
    /// gitignores (a library crate's `Cargo.lock`, say) is on disk but is not
    /// part of what a release commits. `libgit2`'s `add_path` bypasses ignore
    /// rules, so anything staged for the release commit has to be checked
    /// here first rather than relying on `git add` to refuse it.
    pub fn is_tracked(&self, path: &RepoPath) -> Result<bool> {
        let index = self.repo.index()?;
        Ok(index
            .get_path(std::path::Path::new(std::str::from_utf8(&path.0)?), 0)
            .is_some())
    }

    /// Scan the paths in the repository index with progress information.
    /// The callback receives: (path, current_index, total_count)
    pub fn scan_paths_with_progress<F>(&self, mut f: F) -> Result<()>
    where
        F: FnMut(&RepoPath, usize, usize) -> Result<()>,
    {
        let index = self.repo.index()?;
        let total = index.len();

        for (i, entry) in index.iter().enumerate() {
            let p = RepoPath::new(&entry.path);
            atry!(
                f(p, i, total);
                ["encountered a problem while scanning repository entry `{}`", p.escaped()]
            );
        }

        Ok(())
    }

    /// Check if the working tree is clean. Returns None if there are no
    /// modifications and Some(escaped_path) if there are any. (The escaped_path
    /// will be the first one encountered in the check, an essentially arbitrary
    /// selection.) Modifications to any of the paths matched by `ok_matchers`
    /// are allowed.
    pub fn check_if_dirty(&self, ok_matchers: &[PathMatcher]) -> Result<Option<RepoPathBuf>> {
        let mut opts = git2::StatusOptions::new();
        opts.include_untracked(true);
        opts.include_ignored(false);

        for entry in self.repo.statuses(Some(&mut opts))?.iter() {
            // Is this correct / sufficient?
            if entry.status() != git2::Status::CURRENT {
                let repo_path = RepoPath::new(entry.path_bytes());
                let mut is_ok = false;

                for matcher in ok_matchers {
                    if matcher.repo_path_matches(repo_path) {
                        is_ok = true;
                        break;
                    }
                }

                if !is_ok {
                    // Issue #41: on Windows we sometimes think that things are
                    // dirty when they're not actually. As far as I can tell,
                    // this appears to be due to an issue with CRLF processing
                    // when different builds of Git are being invoked on the
                    // same machine, which can happen in Azure Pipelines agents
                    // if you mix and match the pure-Windows environments and
                    // bash scripts. Running a `git status` to refresh the index
                    // can make it go away, but I don't want CI scripts to have
                    // to rely on that kind of thing. Setting up a
                    // .gitattributes seems to fix it even though it seems like
                    // it's just codifying default behavior?
                    if cfg!(windows) {
                        warn!("detected a dirty repository while running on Windows");
                        warn!("if this appears to be spurious, you may need to add a `.gitattributes` file");
                        warn!("to your repo with the contents `* text=auto`, to work around issues related");
                        warn!("to newline processing (CRLF vs LF line endings)");
                    }

                    return Ok(Some(repo_path.to_owned()));
                }
            }
        }

        Ok(None)
    }

    /// Get the binary content of the file at the specified path, at the time of
    /// the specified commit. If the path did not exist, `Ok(None)` is returned.
    pub fn get_file_at_commit(&self, cid: &CommitId, path: &RepoPath) -> Result<Option<Vec<u8>>> {
        let commit = self.repo.find_commit(cid.0)?;
        let tree = commit.tree()?;
        let entry = match tree.get_path(path.as_path()) {
            Ok(e) => e,
            Err(e) => {
                return if e.code() == git2::ErrorCode::NotFound {
                    Ok(None)
                } else {
                    Err(e.into())
                };
            }
        };
        let object = entry.to_object(&self.repo)?;
        let blob = object.as_blob().ok_or_else(|| {
            anyhow!(
                "path `{}` should correspond to a Git blob but does not",
                path.escaped(),
            )
        })?;

        Ok(Some(blob.content().to_owned()))
    }

    /// Get a ReleaseCommitInfo corresponding to the project's history before
    /// Belaf. Always empty in 3.0 — the per-project release history is
    /// derived from git tags + the `belaf-baseline` tag.
    pub(super) fn get_bootstrap_release_info(&self) -> ReleaseCommitInfo {
        ReleaseCommitInfo::default()
    }

    pub fn get_signature(&self) -> Result<git2::Signature<'_>> {
        self.repo
            .signature()
            .or_else(|_| git2::Signature::now("belaf", "belaf@devnull"))
            .map_err(|e| e.into())
    }

    pub fn create_commit(&self, message: &str, files: &[&RepoPath]) -> Result<()> {
        let mut index = self.repo.index()?;

        for file in files {
            index.add_path(std::path::Path::new(std::str::from_utf8(&file.0)?))?;
        }

        index.write()?;
        let tree_id = index.write_tree()?;
        let tree = self.repo.find_tree(tree_id)?;

        let parent_commit = self.repo.head()?.peel_to_commit()?;
        // Not `repo.signature()` directly: that errors when `user.name` /
        // `user.email` are unset, which is the default on a fresh CI runner —
        // `prepare` would push nothing and fail at the commit. `get_signature`
        // falls back to a belaf identity instead.
        let signature = self.get_signature()?;

        self.repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &[&parent_commit],
        )?;

        info!("created commit: {}", message);
        Ok(())
    }

    /// Get the brief message associated with a commit.
    pub fn get_commit_summary(&self, cid: CommitId) -> Result<String> {
        let commit = self.repo.find_commit(cid.0)?;

        if let Some(s) = commit.summary() {
            Ok(s.to_owned())
        } else {
            Ok(format!("[commit {0}: non-Unicode summary]", cid.0))
        }
    }

    /// Get full commit details including author and committer information.
    pub fn get_commit_details(&self, cid: CommitId) -> Result<crate::core::changelog::Commit> {
        let commit = self.repo.find_commit(cid.0)?;
        Ok(crate::core::changelog::Commit::from(&commit))
    }

    /// The repo-relative paths changed by a commit (diff vs its first parent;
    /// for a root commit, vs the empty tree). Used by `belaf check` (F8) to
    /// validate that a commit's changed paths belong to its scope's closure.
    pub fn commit_changed_paths(&self, cid: CommitId) -> Result<Vec<RepoPathBuf>> {
        let commit = self.repo.find_commit(cid.0)?;
        let cur_tree = commit.tree()?;
        let parent_tree = if commit.parent_count() == 0 {
            None
        } else {
            Some(commit.parent(0)?.tree()?)
        };
        let mut dopts = git2::DiffOptions::new();
        dopts.include_typechange(true);
        let diff =
            self.repo
                .diff_tree_to_tree(parent_tree.as_ref(), Some(&cur_tree), Some(&mut dopts))?;
        let mut out = Vec::new();
        for delta in diff.deltas() {
            for file in &[delta.old_file(), delta.new_file()] {
                if let Some(b) = file.path_bytes() {
                    let p = RepoPathBuf::new(b);
                    if !out.contains(&p) {
                        out.push(p);
                    }
                }
            }
        }
        Ok(out)
    }

    /// Resolve a `git`-style commit range (`A..B`, or a single revision meaning
    /// "just that commit") into the list of [`CommitId`]s it contains, newest
    /// first. Used by `belaf check --range` (F8).
    pub fn commits_in_range(&self, range: &str) -> Result<Vec<CommitId>> {
        let mut walk = self.repo.revwalk()?;
        if range.contains("..") {
            walk.push_range(range)
                .with_context(|| format!("invalid commit range `{range}`"))?;
        } else {
            let oid = self
                .repo
                .revparse_single(range)
                .with_context(|| format!("could not resolve revision `{range}`"))?
                .id();
            walk.push(oid)?;
            // A bare revision means "just that commit", not its whole ancestry.
            if let Ok(c) = self.repo.find_commit(oid) {
                if c.parent_count() > 0 {
                    walk.hide(c.parent_id(0)?)?;
                }
            }
        }
        let mut out = Vec::new();
        for oid in walk {
            out.push(CommitId(oid?));
        }
        Ok(out)
    }
}

/// A reference to something in the repository history. Ideally this is to a
/// specific commit, but to allow bootstrapping internal dependencies on old
/// versions we also have an escape-hatch mode. We also have some special
/// machinery to allow people to create commits that reference themselves.
pub enum ParsedHistoryRef {
    /// A reference to a specific commit ID
    Id(CommitId),

    /// A reference to the commit that introduced this reference into the
    /// repository contents. `salt` is a random string allowing different
    /// this-commit references to be distinguished and to ease identification of
    /// the relevant commit through "blame" tracing of the repository history.
    ThisCommit { salt: String },

    /// A ref that is manually specified, which we're unable to resolve into a
    /// specific commit.
    Manual(String),
}

#[cfg(test)]
#[path = "repository_tests.rs"]
mod repository_tests;
