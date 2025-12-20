// Copyright 2020 Peter Williams <peter@newton.cx> and collaborators
// Licensed under the MIT License.

//! State of the backing version control repository.

use anyhow::{anyhow, bail, Context};
use ref_cast::RefCast;
use serde::{Deserialize, Serialize};

use std::{
    fs::File,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
};
use thiserror::Error as ThisError;
use tracing::{info, warn};
use uuid::Uuid;

use crate::{
    atry,
    cmd::init::BootstrapConfiguration,
    core::{
        bump::{extract_scope, ScopeMatcher},
        config::syntax::RepoConfiguration,
        errors::{Error, Result},
        project::{DepRequirement, Project},
        version::Version,
    },
};

/// Opaque type representing a commit in the repository.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommitId(git2::Oid);

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
    repo: git2::Repository,

    /// The name of the "upstream" remote.
    upstream_name: String,

    /// "Bootstrap" versioning information used to tell us where versions were at
    /// before the first Belaf release commit.
    bootstrap_info: BootstrapConfiguration,

    /// Analysis configuration for LRU cache sizes.
    analysis_config: crate::core::config::syntax::AnalysisConfig,
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
            bootstrap_info: BootstrapConfiguration::default(),
            analysis_config: crate::core::config::syntax::AnalysisConfig {
                commit_cache_size: 512,
                tree_cache_size: 3,
            },
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

        // While we're here, let's also read in the versioning bootstrap
        // information, if it's available.

        let mut bs_path = self.resolve_config_dir();
        bs_path.push("bootstrap.toml");

        let maybe_file = match File::open(&bs_path) {
            Ok(f) => Some(f),

            Err(e) => {
                if e.kind() == std::io::ErrorKind::NotFound {
                    None
                } else {
                    return Err(Error::new(e).context(format!(
                        "failed to open config file `{}`",
                        bs_path.display()
                    )));
                }
            }
        };

        if let Some(mut f) = maybe_file {
            let mut text = String::new();
            atry!(
                f.read_to_string(&mut text);
                ["failed to read bootstrap file `{}`", bs_path.display()]
            );

            self.bootstrap_info = atry!(
                toml::from_str(&text);
                ["could not parse bootstrap file `{}` as TOML", bs_path.display()]
            );
        }

        // All done.
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
    /// Belaf.
    fn get_bootstrap_release_info(&self) -> ReleaseCommitInfo {
        let mut rel_info = ReleaseCommitInfo::default();

        for bs_info in &self.bootstrap_info.project[..] {
            rel_info.projects.push(ReleasedProjectInfo {
                qnames: bs_info.qnames.clone(),
                version: bs_info.version.clone(),
                age: 999,
            })
        }

        rel_info
    }

    pub fn get_signature(&self) -> Result<git2::Signature<'_>> {
        self.repo
            .signature()
            .or_else(|_| git2::Signature::now("belaf", "belaf@devnull"))
            .map_err(|e| e.into())
    }

    /// Find the latest release tag for a project.
    ///
    /// For single-project repos (`is_single_project = true`), matches both:
    /// - Plain version tags: `v1.2.3`
    /// - Prefixed tags: `project-name-v1.2.3`
    ///
    /// For multi-project repos, only matches prefixed tags to avoid ambiguity.
    ///
    /// Returns the commit OID and tag name of the latest matching tag,
    /// sorted by semantic version (highest first).
    pub fn find_latest_tag_for_project(
        &self,
        project_name: &str,
        is_single_project: bool,
    ) -> Result<Option<(git2::Oid, String)>> {
        let tags = self.repo.tag_names(None)?;

        let mut matching_tags = Vec::new();

        for tag_name in tags.iter().flatten() {
            let is_prefixed_tag = tag_name.starts_with(&format!("{}-v", project_name));
            let is_plain_v_tag = is_single_project
                && tag_name.starts_with('v')
                && tag_name.chars().nth(1).is_some_and(|c| c.is_ascii_digit());

            if is_prefixed_tag || is_plain_v_tag {
                if let Ok(tag_ref) = self.repo.find_reference(&format!("refs/tags/{}", tag_name)) {
                    if let Some(target_oid) = tag_ref.target() {
                        matching_tags.push((target_oid, tag_name.to_string()));
                    } else if let Ok(tag_obj) = tag_ref.peel_to_tag() {
                        matching_tags.push((tag_obj.target_id(), tag_name.to_string()));
                    }
                }
            }
        }

        if matching_tags.is_empty() {
            return Ok(None);
        }

        matching_tags.sort_by(|a, b| {
            let v1 = Self::parse_version_from_tag(&a.1);
            let v2 = Self::parse_version_from_tag(&b.1);
            v2.cmp(&v1)
        });

        Ok(matching_tags.into_iter().next())
    }

    /// Parse a semantic version from a tag name.
    ///
    /// Supports two formats:
    /// - Plain version tags: `v1.2.3` → `1.2.3`
    /// - Prefixed tags: `project-name-v1.2.3` → `1.2.3`
    ///
    /// Returns `0.0.0` if the tag cannot be parsed as a valid semver.
    pub fn parse_version_from_tag(tag_name: &str) -> semver::Version {
        if let Some(version_str) = tag_name.strip_prefix('v') {
            if let Ok(version) = semver::Version::parse(version_str) {
                return version;
            }
        }
        if let Some(version_str) = tag_name.rsplit("-v").next() {
            if let Ok(version) = semver::Version::parse(version_str) {
                return version;
            }
        }
        semver::Version::new(0, 0, 0)
    }

    fn find_baseline_tag(&self) -> Result<Option<git2::Oid>> {
        match self.repo.find_reference("refs/tags/belaf-baseline") {
            Ok(tag_ref) => {
                if let Some(target_oid) = tag_ref.target() {
                    Ok(Some(target_oid))
                } else if let Ok(tag_obj) = tag_ref.peel_to_tag() {
                    Ok(Some(tag_obj.target_id()))
                } else {
                    Ok(None)
                }
            }
            Err(_) => Ok(None),
        }
    }

    pub fn create_baseline_tag(&self) -> Result<()> {
        let head = self.repo.head()?;
        let target_oid = head.target().context("HEAD has no target")?;

        match self.repo.tag_lightweight(
            "belaf-baseline",
            &self.repo.find_object(target_oid, None)?,
            false,
        ) {
            Ok(_) => {
                info!("created baseline tag 'belaf-baseline' at HEAD");
                Ok(())
            }
            Err(e) if e.code() == git2::ErrorCode::Exists => {
                warn!("baseline tag 'belaf-baseline' already exists, not creating");
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
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
        let signature = self.repo.signature()?;

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

    /// Figure out which commits in the history affect each project since its
    /// last release.
    ///
    /// This gets a little tricky since not all projects in the repo are
    /// released in lockstep. For each individiual project, we need to analyze
    /// the history from HEAD to its most recent release commit. I worry about
    /// the efficiency of this so we trace all the histories at once to try to
    /// improve that.
    pub fn analyze_histories(&self, projects: &[Project]) -> Result<Vec<RepoHistory>> {
        let mut histories = vec![
            RepoHistory {
                commits: Vec::new(),
                release_tag: None,
            };
            projects.len()
        ];

        let baseline_tag_oid = self.find_baseline_tag()?;
        let is_single_project = projects.len() == 1;

        for (i, proj) in projects.iter().enumerate() {
            if let Some((tag_oid, tag_name)) =
                self.find_latest_tag_for_project(&proj.user_facing_name, is_single_project)?
            {
                let version = Self::parse_version_from_tag(&tag_name);
                info!(
                    "found release tag for {}: {} (v{})",
                    proj.user_facing_name, tag_name, version
                );
                histories[i].release_tag = Some(ReleaseTagInfo {
                    commit: CommitId(tag_oid),
                    tag_name,
                    version,
                });
            } else if let Some(baseline_oid) = baseline_tag_oid {
                info!(
                    "no release tag for {}, using baseline tag belaf-baseline",
                    proj.user_facing_name
                );
                histories[i].release_tag = Some(ReleaseTagInfo {
                    commit: CommitId(baseline_oid),
                    tag_name: "belaf-baseline".to_string(),
                    version: semver::Version::new(0, 0, 0),
                });
            } else {
                warn!(
                    "no release tag or baseline found for {}, analyzing all commits since repo start",
                    proj.user_facing_name
                );
            }
        }

        let commit_cache_size = std::num::NonZeroUsize::new(self.analysis_config.commit_cache_size)
            .unwrap_or(std::num::NonZeroUsize::new(512).expect("BUG: 512 is non-zero"));
        let tree_cache_size = std::num::NonZeroUsize::new(self.analysis_config.tree_cache_size)
            .unwrap_or(std::num::NonZeroUsize::new(3).expect("BUG: 3 is non-zero"));

        let mut commit_data = lru::LruCache::new(commit_cache_size);
        let mut trees = lru::LruCache::new(tree_cache_size);

        let mut dopts = git2::DiffOptions::new();
        dopts.include_typechange(true);

        let project_names: Vec<String> = projects
            .iter()
            .map(|p| p.user_facing_name.clone())
            .collect();
        let scope_matcher = ScopeMatcher::default();

        // note that we don't "know" that proj_idx = project.ident
        for proj_idx in 0..projects.len() {
            let mut walk = self.repo.revwalk()?;
            walk.push_head()?;

            if let Some(tag_info) = &histories[proj_idx].release_tag {
                walk.hide(tag_info.commit.0)?;
            }

            // Walk through the history, finding relevant commits. The full
            // codepath loads up trees for each commit and its parents, computes
            // the diff, and compares that against the path-matchers for each
            // project to decide if a given commit affects a given project. The
            // intention is that the LRU caches will make it so that little
            // redundant work is performed.

            for maybe_oid in walk {
                let oid = maybe_oid?;

                // Hopefully this commit is already in the cache, but if not ...
                if !commit_data.contains(&oid) {
                    // Get the two relevant trees and compute their diff. We have to
                    // jump through some hoops to support the root commit (with no
                    // parents) but it's not really that bad. We also have to pop() the
                    // trees out of the LRU because get() holds a mutable reference to
                    // the cache, which prevents us from looking at two trees
                    // simultaneously.

                    let commit = self.repo.find_commit(oid)?;
                    let ctid = commit.tree_id();
                    let cur_tree = match trees.pop(&ctid) {
                        Some(t) => t,
                        None => self.repo.find_tree(ctid)?,
                    };

                    let (maybe_ptid, maybe_parent_tree) = if commit.parent_count() == 0 {
                        (None, None) // this is the first commit in the history!
                    } else {
                        let parent = commit.parent(0)?;
                        let ptid = parent.tree_id();
                        let parent_tree = match trees.pop(&ptid) {
                            Some(t) => t,
                            None => self.repo.find_tree(ptid)?,
                        };
                        (Some(ptid), Some(parent_tree))
                    };

                    let diff = self.repo.diff_tree_to_tree(
                        maybe_parent_tree.as_ref(),
                        Some(&cur_tree),
                        Some(&mut dopts),
                    )?;

                    trees.put(ctid, cur_tree);
                    if let (Some(ptid), Some(pt)) = (maybe_ptid, maybe_parent_tree) {
                        trees.put(ptid, pt);
                    }

                    let mut hit_buf = vec![false; projects.len()];

                    if commit.parent_count() < 2 {
                        let mut scope_matched = false;

                        if let Some(summary) = commit.summary() {
                            if let Some(scope) = extract_scope(summary) {
                                if let Some(matched_name) =
                                    scope_matcher.find_matching_project(&scope, &project_names)
                                {
                                    for (idx, proj) in projects.iter().enumerate() {
                                        if &proj.user_facing_name == matched_name {
                                            hit_buf[idx] = true;
                                            scope_matched = true;
                                            break;
                                        }
                                    }
                                }
                            }
                        }

                        if !scope_matched {
                            for delta in diff.deltas() {
                                for file in &[delta.old_file(), delta.new_file()] {
                                    if let Some(path_bytes) = file.path_bytes() {
                                        let path = RepoPath::new(path_bytes);
                                        for (idx, proj) in projects.iter().enumerate() {
                                            if proj.repo_paths.repo_path_matches(path) {
                                                hit_buf[idx] = true;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    commit_data.put(oid, hit_buf);
                }

                let hits = commit_data
                    .get(&oid)
                    .expect("BUG: commit data should be in cache after put()");

                if hits[proj_idx] {
                    histories[proj_idx].commits.push(CommitId(oid));
                }
            }
        }

        Ok(histories)
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

    pub fn create_branch(&self, name: &str) -> Result<()> {
        let head_ref = self.repo.head()?;
        let head_commit = head_ref.peel_to_commit()?;
        self.repo.branch(name, &head_commit, false)?;
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

    pub fn push_branch(&self, branch_name: &str) -> Result<()> {
        let mut remote = self.repo.find_remote(&self.upstream_name)?;
        let refspec = format!("refs/heads/{}:refs/heads/{}", branch_name, branch_name);

        let mut callbacks = git2::RemoteCallbacks::new();
        callbacks.credentials(|_url, username_from_url, allowed_types| {
            if allowed_types.contains(git2::CredentialType::SSH_KEY) {
                git2::Cred::ssh_key_from_agent(username_from_url.unwrap_or("git"))
            } else if allowed_types.contains(git2::CredentialType::USER_PASS_PLAINTEXT) {
                let token = std::env::var("GITHUB_TOKEN")
                    .ok()
                    .or_else(|| crate::core::auth::token::load_token().ok());
                if let Some(token) = token {
                    git2::Cred::userpass_plaintext("x-access-token", &token)
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

/// Describes the availability of a given commit in the release of a project.
/// Note that because different projects are released at different times, the
/// availability for the same commit might vary depending on which project we're
/// considering.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReleaseAvailability {
    /// The commit has already been released. The earliest release containing it
    /// has the given version.
    ExistingRelease(Version),

    /// The commit has not been released, but is an ancestor of HEAD, so it
    /// would be available if a new release of the target project were to be
    /// created. We need to pay attention to this case to allow people to stage
    /// and release multiple projects in one batch.
    NewRelease,

    /// Neither of the above applies.
    NotAvailable,
}

impl Repository {
    pub fn find_earliest_release_containing(
        &self,
        proj: &Project,
        cid: &CommitId,
        is_single_project: bool,
    ) -> Result<ReleaseAvailability> {
        if let Some((tag_oid, tag_name)) =
            self.find_latest_tag_for_project(&proj.user_facing_name, is_single_project)?
        {
            if self.repo.graph_descendant_of(tag_oid, cid.0)? || tag_oid == cid.0 {
                let version = Self::parse_version_from_tag(&tag_name);
                let v = Version::parse_like(&proj.version, version.to_string())?;
                return Ok(ReleaseAvailability::ExistingRelease(v));
            }
        }

        let head_ref = self.repo.head()?;
        let head_commit = head_ref.peel_to_commit()?;
        let head_id = head_commit.id();

        if head_id == cid.0 || self.repo.graph_descendant_of(head_id, cid.0)? {
            Ok(ReleaseAvailability::NewRelease)
        } else {
            Ok(ReleaseAvailability::NotAvailable)
        }
    }
}

/// Information about the state of the projects in the repository corresponding
/// to a "release" commit where all of the projects have been assigned version
/// numbers, and the commit should have made it out into the wild only if all of
/// the CI tests passed.
#[derive(Clone, Debug, Default)]
pub struct ReleaseCommitInfo {
    /// The Git commit-ish that this object describes. May be None when there is
    /// no upstream `release` branch, in which case this struct will contain no
    /// genuine information.
    pub commit: Option<CommitId>,

    /// A list of projects and their release information as of this commit. This
    /// list includes every tracked project in this commit. Not all of those
    /// projects necessarily were released with this commit, if they were
    /// unchanged from a previous release commit.
    pub projects: Vec<ReleasedProjectInfo>,
}

impl ReleaseCommitInfo {
    /// Attempt to find info for a prior release of the named project.
    ///
    /// Information may be missing if the project was only added to the
    /// repository after this information was recorded.
    pub fn lookup_project(&self, proj: &Project) -> Option<&ReleasedProjectInfo> {
        self.projects
            .iter()
            .find(|&rpi| rpi.qnames == *proj.qualified_names())
    }

    /// Find information about a project release if it occurred at this moment.
    ///
    /// This function is like `lookup_project()`, but also returns None if the
    /// "age" of any identified release is not zero.
    pub fn lookup_if_released(&self, proj: &Project) -> Option<&ReleasedProjectInfo> {
        self.lookup_project(proj).filter(|rel| rel.age == 0)
    }
}

/// Serializable state information about a single project in a release commit.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReleasedProjectInfo {
    /// The qualified names of this project, equivalent to the same-named
    /// property of the Project struct.
    pub qnames: Vec<String>,

    /// The version of the project in this commit, as text.
    pub version: String,

    /// The number of consecutive release commits for which this project
    /// has had the assigned version string. If zero, that means that the
    /// specified version was first released with this commit.
    pub age: usize,
}

/// A data structure recording changes made when rewriting files
/// in the repository.
#[derive(Debug, Default)]
pub struct ChangeList {
    paths: Vec<RepoPathBuf>,
}

impl ChangeList {
    /// Mark the file at this path as having been updated.
    pub fn add_path(&mut self, p: &RepoPath) {
        self.paths.push(p.to_owned());
    }

    /// Get the paths in this changelist.
    pub fn paths(&self) -> impl Iterator<Item = &RepoPath> {
        self.paths[..].iter().map(|p| p.as_ref())
    }
}

#[derive(Clone, Debug)]
pub struct ReleaseTagInfo {
    pub commit: CommitId,
    pub tag_name: String,
    pub version: semver::Version,
}

#[derive(Clone, Debug)]
pub struct RepoHistory {
    commits: Vec<CommitId>,
    release_tag: Option<ReleaseTagInfo>,
}

impl RepoHistory {
    pub fn release_tag(&self) -> Option<&ReleaseTagInfo> {
        self.release_tag.as_ref()
    }

    pub fn release_commit(&self) -> Option<CommitId> {
        self.release_tag.as_ref().map(|t| t.commit)
    }

    pub fn release_version(&self) -> Option<&semver::Version> {
        self.release_tag.as_ref().map(|t| &t.version)
    }

    pub fn release_info(&self, repo: &Repository) -> Result<ReleaseCommitInfo> {
        let mut info = repo.get_bootstrap_release_info();

        if let Some(tag) = &self.release_tag {
            for proj_info in &mut info.projects {
                if proj_info.version == tag.version.to_string() {
                    proj_info.age = 0;
                }
            }
        }

        Ok(info)
    }

    pub fn n_commits(&self) -> usize {
        self.commits.len()
    }

    pub fn commits(&self) -> impl IntoIterator<Item = &CommitId> {
        &self.commits[..]
    }
}

/// A filter that matches paths inside the repository and/or working directory.
///
/// We're not trying to get fully general here, but there is a common use case
/// that we need to support. A monorepo might contain a toplevel project, rooted
/// at the repo base, plus one or more subprojects in some kind of
/// subdirectories. For the toplevel project, we need to express a match for a
/// file anywhere in the repo *except* ones that match any of the subprojects.
#[derive(Debug)]
pub struct PathMatcher {
    terms: Vec<PathMatcherTerm>,
}

impl PathMatcher {
    /// Create a new matcher that includes only files in the specified repopath
    /// prefix.
    pub fn new_include(p: RepoPathBuf) -> Self {
        let terms = vec![PathMatcherTerm::Include(p)];
        PathMatcher { terms }
    }

    /// Modify this matcher to exclude any paths that *other* would include.
    ///
    /// This whole framework could surely be a lot more efficient, but unless
    /// your repo has 1000 projects it's just not going to matter, I think.
    pub fn make_disjoint(&mut self, other: &PathMatcher) -> &mut Self {
        let mut new_terms = Vec::new();

        for other_term in &other.terms {
            if let PathMatcherTerm::Include(ref other_pfx) = other_term {
                for term in &self.terms {
                    if let PathMatcherTerm::Include(ref pfx) = term {
                        // We only need to exclude terms in the other matcher
                        // that are more specific than ours.
                        if other_pfx.starts_with(pfx) {
                            new_terms.push(PathMatcherTerm::Exclude(other_pfx.clone()));
                        }
                    }
                }
            }
        }

        new_terms.append(&mut self.terms);
        self.terms = new_terms;
        self
    }

    /// Test whether a repo-path matches.
    pub fn repo_path_matches(&self, p: &RepoPath) -> bool {
        for term in &self.terms {
            match term {
                PathMatcherTerm::Include(pfx) => {
                    if p.starts_with(pfx) {
                        return true;
                    }
                }

                PathMatcherTerm::Exclude(pfx) => {
                    if p.starts_with(pfx) {
                        return false;
                    }
                }
            }
        }

        false
    }
}

#[derive(Debug)]
enum PathMatcherTerm {
    /// Include paths prefixed by the value.
    Include(RepoPathBuf),

    /// Exclude paths prefixed by the value.
    Exclude(RepoPathBuf),
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

// Below we have helpers for trying to deal with git's paths properly, on the
// off-chance that they contain invalid UTF-8 and the like.

/// A borrowed reference to a pathname as understood by the backing repository.
///
/// In git, such a path is a byte array. The directory separator is always "/".
/// The bytes are often convertible to UTF-8, but not always. (These are the
/// same semantics as Unix paths.)
#[derive(Debug, Eq, Hash, PartialEq, RefCast)]
#[repr(transparent)]
pub struct RepoPath([u8]);

impl std::convert::AsRef<RepoPath> for [u8] {
    fn as_ref(&self) -> &RepoPath {
        RepoPath::ref_cast(self)
    }
}

impl std::convert::AsRef<[u8]> for RepoPath {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl RepoPath {
    fn new(p: &[u8]) -> &Self {
        p.as_ref()
    }

    /// Split a path into a directory name and a file basename.
    ///
    /// Returns `(dirname, basename)`. The dirname will be empty if the path
    /// contains no separator. Otherwise, it will end with the path separator.
    /// It is always true that `self = concat(dirname, basename)`.
    pub fn split_basename(&self) -> (&RepoPath, &RepoPath) {
        let basename = self
            .0
            .rsplit(|c| *c == b'/')
            .next()
            .expect("BUG: rsplit always returns at least one element");
        let ndir = self.0.len() - basename.len();
        (self.0[..ndir].as_ref(), basename.as_ref())
    }

    /// Return this path with a trailing directory separator removed, if one is
    /// present.
    pub fn pop_sep(&self) -> &RepoPath {
        let n = self.0.len();

        if n == 0 || self.0[n - 1] != b'/' {
            self
        } else {
            self.0[..n - 1].as_ref()
        }
    }

    /// Get the length of the path, in bytes
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Check if the path is empty
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Convert the repository path into an OS path.
    pub fn as_path(&self) -> &Path {
        bytes2path(&self.0)
    }

    /// Convert this borrowed reference into an owned copy.
    pub fn to_owned(&self) -> RepoPathBuf {
        RepoPathBuf::new(&self.0[..])
    }

    /// Compute a user-displayable escaped version of this path.
    pub fn escaped(&self) -> String {
        escape_pathlike(&self.0)
    }

    /// Return true if this path starts with the argument.
    pub fn starts_with<P: AsRef<[u8]>>(&self, other: P) -> bool {
        let other = other.as_ref();
        let sn = self.len();
        let on = other.len();

        if sn < on {
            false
        } else {
            &self.0[..on] == other
        }
    }

    /// Return true if this path ends with the argument.
    pub fn ends_with<P: AsRef<[u8]>>(&self, other: P) -> bool {
        let other = other.as_ref();
        let sn = self.len();
        let on = other.len();

        if sn < on {
            false
        } else {
            &self.0[(sn - on)..] == other
        }
    }
}

impl git2::IntoCString for &RepoPath {
    fn into_c_string(self) -> std::result::Result<std::ffi::CString, git2::Error> {
        self.0.into_c_string()
    }
}

// Copied from git2-rs src/util.rs
#[cfg(unix)]
fn bytes2path(b: &[u8]) -> &Path {
    use std::{ffi::OsStr, os::unix::prelude::*};
    Path::new(OsStr::from_bytes(b))
}
#[cfg(windows)]
fn bytes2path(b: &[u8]) -> &Path {
    use std::str;
    Path::new(str::from_utf8(b).expect("BUG: git paths should be valid UTF-8 on Windows"))
}

/// An owned reference to a pathname as understood by the backing repository.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[repr(transparent)]
pub struct RepoPathBuf(Vec<u8>);

impl std::convert::AsRef<RepoPath> for RepoPathBuf {
    fn as_ref(&self) -> &RepoPath {
        RepoPath::new(&self.0[..])
    }
}

impl std::convert::AsRef<[u8]> for RepoPathBuf {
    fn as_ref(&self) -> &[u8] {
        &self.0[..]
    }
}

fn validate_safe_repo_path(path: &Path) -> Result<()> {
    use std::path::Component;

    if path.as_os_str().is_empty() {
        return Ok(());
    }

    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let bytes = path.as_os_str().as_bytes();
        if bytes.contains(&0) {
            bail!("path contains null byte: `{}`", path.display());
        }

        if (bytes.windows(2).any(|w| w == b"/.") || bytes.starts_with(b"."))
            && bytes
                .split(|&b| b == b'/')
                .any(|seg| seg == b"." || seg == b"..")
        {
            bail!(
                "path contains current or parent directory reference (. or ..): `{}`",
                path.display()
            );
        }
    }

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;

        let wide_chars: Vec<u16> = path.as_os_str().encode_wide().collect();

        if wide_chars.contains(&0) {
            bail!("path contains null byte: `{}`", path.display());
        }

        let has_dot_segment = wide_chars
            .split(|&c| c == b'/' as u16 || c == b'\\' as u16)
            .any(|seg| seg == [b'.' as u16] || seg == [b'.' as u16, b'.' as u16]);

        if has_dot_segment {
            bail!(
                "path contains current or parent directory reference (. or ..): `{}`",
                path.display()
            );
        }

        let reserved_names = [
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        ];

        for component in path.components() {
            if let Component::Normal(comp) = component {
                if let Some(comp_str) = comp.to_str() {
                    let base = comp_str.split('.').next().unwrap_or("");
                    if reserved_names.iter().any(|&r| base.eq_ignore_ascii_case(r)) {
                        bail!(
                            "path contains reserved Windows filename: `{}`",
                            path.display()
                        );
                    }
                }
            }
        }
    }

    for component in path.components() {
        match component {
            Component::ParentDir => {
                bail!(
                    "path contains parent directory reference (..): `{}`",
                    path.display()
                );
            }
            Component::RootDir | Component::Prefix(_) => {
                bail!("path must be relative: `{}`", path.display());
            }
            Component::CurDir => {
                bail!(
                    "path contains current directory reference (.): `{}`",
                    path.display()
                );
            }
            Component::Normal(_) => {}
        }
    }

    Ok(())
}

impl RepoPathBuf {
    pub fn new(b: &[u8]) -> Self {
        RepoPathBuf(b.to_vec())
    }

    /// Create a RepoPathBuf from a Path-like. It is assumed that the path is
    /// relative to the repository working directory root and doesn't have any
    /// funny business like ".." in it.
    #[cfg(unix)]
    fn from_path<P: AsRef<Path>>(p: P) -> Result<Self> {
        use std::os::unix::ffi::OsStrExt;
        let path = p.as_ref();

        validate_safe_repo_path(path)?;

        Ok(Self::new(path.as_os_str().as_bytes()))
    }

    /// Create a RepoPathBuf from a Path-like. It is assumed that the path is
    /// relative to the repository working directory root and doesn't have any
    /// funny business like ".." in it.
    #[cfg(windows)]
    fn from_path<P: AsRef<Path>>(p: P) -> Result<Self> {
        let path = p.as_ref();

        validate_safe_repo_path(path)?;

        let mut first = true;
        let mut b = Vec::new();

        for cmpt in path.components() {
            if first {
                first = false;
            } else {
                b.push(b'/');
            }

            if let std::path::Component::Normal(c) = cmpt {
                let s = c
                    .to_str()
                    .ok_or_else(|| anyhow!("path component `{:?}` is not valid UTF-8", c))?;
                b.extend(s.as_bytes());
            } else {
                bail!("path with unexpected components: `{}`", path.display());
            }
        }

        Ok(RepoPathBuf(b))
    }

    pub fn truncate(&mut self, len: usize) {
        self.0.truncate(len);
    }

    pub fn push<C: AsRef<[u8]>>(&mut self, component: C) {
        let n = self.0.len();

        if n > 0 && self.0[n - 1] != b'/' {
            self.0.push(b'/');
        }

        self.0.extend(component.as_ref());
    }
}

impl std::ops::Deref for RepoPathBuf {
    type Target = RepoPath;

    fn deref(&self) -> &RepoPath {
        RepoPath::new(&self.0[..])
    }
}

/// Convert an arbitrary byte slice to something printable.
///
/// If the bytes can be interpreted as UTF-8, their Unicode stringification will
/// be returned. Otherwise, bytes that aren't printable ASCII will be
/// backslash-escaped, and the whole string will be wrapped in double quotes.
///
/// Special handling for security-relevant characters (null bytes, control chars).
pub fn escape_pathlike(b: &[u8]) -> String {
    if b.contains(&0) {
        let mut buf = String::from("\"<path-with-null-byte:");
        for (i, &byte) in b.iter().enumerate() {
            if byte == 0 {
                buf.push_str(&format!("\\0@{}", i));
            }
        }
        buf.push_str(">\"");
        return buf;
    }

    if let Ok(s) = std::str::from_utf8(b) {
        if s.chars().all(|c| {
            (c.is_ascii_graphic() && c != '"' && c != '\\')
                || c == '/'
                || c == '-'
                || c == '_'
                || c == '.'
        }) {
            return s.to_owned();
        }

        let mut buf = String::from("\"");
        for ch in s.chars() {
            match ch {
                '"' => buf.push_str("\\\""),
                '\\' => buf.push_str("\\\\"),
                '\n' => buf.push_str("\\n"),
                '\r' => buf.push_str("\\r"),
                '\t' => buf.push_str("\\t"),
                c if c.is_control() => buf.push_str(&format!("\\u{{{:04x}}}", c as u32)),
                c => buf.push(c),
            }
        }
        buf.push('"');
        buf
    } else {
        let mut buf = vec![b'\"'];
        buf.extend(b.iter().flat_map(|c| std::ascii::escape_default(*c)));
        buf.push(b'\"');
        String::from_utf8(buf).expect("BUG: ASCII escape sequences should always be valid UTF-8")
    }
}

#[cfg(test)]
#[path = "repository_tests.rs"]
mod repository_tests;
