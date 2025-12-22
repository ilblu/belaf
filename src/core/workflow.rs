//! Release workflow orchestration for PR-based releases.
//!
//! This module implements the [`ReleasePipeline`] which coordinates the complete
//! release workflow:
//!
//! 1. Version bumping across all selected projects
//! 2. Changelog generation using git-cliff style templates
//! 3. Release manifest creation in `belaf/releases/`
//! 4. Git branch management (create, commit, push)
//! 5. GitHub Pull Request creation
//!
//! The workflow is designed for CI/CD environments where releases go through
//! a PR review process before being finalized by a GitHub App.

use anyhow::{Context, Result};
use std::collections::HashMap;
use tracing::{debug, info};

use crate::core::{
    bump::{self, BumpConfig, BumpRecommendation},
    changelog::{Changelog, ChangelogConfig, Commit, GitConfig, Release},
    config::syntax::{BumpConfiguration, ChangelogConfiguration},
    ecosystem::types::EcosystemType,
    git::repository::{ChangeList, RepoPathBuf, Repository},
    github::{client::GitHubInformation, pr},
    graph::GraphQueryBuilder,
    manifest::{ProjectRelease, ReleaseManifest, ReleaseStatistics, MANIFEST_DIR},
    project::ProjectId,
    session::AppSession,
};

#[derive(Debug, Clone)]
pub struct ProjectCandidate {
    pub ident: ProjectId,
    pub name: String,
    pub prefix: String,
    pub current_version: String,
    pub commits: Vec<Commit>,
    pub commit_count: usize,
    pub suggested_bump: BumpRecommendation,
    pub ecosystem: EcosystemType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BumpChoice {
    Auto,
    Major,
    Minor,
    Patch,
}

impl BumpChoice {
    pub fn resolve(&self, suggested: BumpRecommendation) -> &'static str {
        match self {
            Self::Auto => suggested.as_str(),
            Self::Major => "major",
            Self::Minor => "minor",
            Self::Patch => "patch",
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Major => "major",
            Self::Minor => "minor",
            Self::Patch => "patch",
        }
    }

    pub fn all() -> Vec<Self> {
        vec![Self::Auto, Self::Major, Self::Minor, Self::Patch]
    }
}

#[derive(Debug, Clone)]
pub struct ProjectSelection {
    pub candidate: ProjectCandidate,
    pub bump_choice: BumpChoice,
    pub cached_changelog: Option<String>,
}

type ChangelogGenerationResult = (
    Vec<RepoPathBuf>,
    HashMap<String, String>,
    HashMap<String, Vec<Commit>>,
);

pub struct PrepareContext<'a> {
    pub sess: &'a mut AppSession,
    pub base_branch: String,
    pub release_branch: String,
    pub candidates: Vec<ProjectCandidate>,
    pub allow_dirty: bool,
    pub changelog_config: ChangelogConfiguration,
    pub bump_config: BumpConfiguration,
}

impl<'a> PrepareContext<'a> {
    pub fn initialize(sess: &'a mut AppSession, allow_dirty: bool) -> Result<Self> {
        if !allow_dirty {
            if let Some(dirty) = sess
                .repo
                .check_if_dirty(&[])
                .context("failed to check repository for modified files")?
            {
                return Err(anyhow::anyhow!(
                    "requires a clean working directory. Found uncommitted changes: {}",
                    dirty.escaped()
                ));
            }
        } else if let Some(dirty) = sess
            .repo
            .check_if_dirty(&[])
            .context("failed to check repository for modified files")?
        {
            info!(
                "preparing release with uncommitted changes in the repository (e.g.: `{}`)",
                dirty.escaped()
            );
        }

        let (base_branch, release_branch) = create_release_branch(sess)?;
        let changelog_config = sess.changelog_config.clone();
        let bump_config = sess.bump_config.clone();

        Ok(Self {
            sess,
            base_branch,
            release_branch,
            candidates: Vec::new(),
            allow_dirty,
            changelog_config,
            bump_config,
        })
    }

    pub fn resolve_workdir(
        &self,
        path: &crate::core::git::repository::RepoPath,
    ) -> std::path::PathBuf {
        self.sess.repo.resolve_workdir(path)
    }

    pub fn discover_projects(&mut self) -> Result<()> {
        let q = GraphQueryBuilder::default();
        let idents = self
            .sess
            .graph()
            .query(q)
            .context("could not select projects")?;

        if idents.is_empty() {
            info!("no projects found in repository");
            return Ok(());
        }

        let histories = self
            .sess
            .analyze_histories()
            .context("failed to analyze project histories")?;

        for ident in &idents {
            let proj = self.sess.graph().lookup(*ident);
            let history = histories.lookup(*ident);
            let n_commits = history.n_commits();

            if n_commits == 0 {
                info!(
                    "{}: no changes since last release, skipping",
                    proj.user_facing_name
                );
                continue;
            }

            let commits: Vec<Commit> = history
                .commits()
                .into_iter()
                .filter_map(|cid| self.sess.repo.get_commit_details(*cid).ok())
                .collect();

            let current_version = proj.version.to_string();

            let analysis = bump::analyze_commits(&commits).with_context(|| {
                format!(
                    "failed to analyze commit messages for {}",
                    proj.user_facing_name
                )
            })?;

            let bump_config = BumpConfig::from_user_config(&self.bump_config);
            let suggested_bump = analysis
                .recommendation
                .apply_config(&bump_config, Some(&current_version));

            info!("{}: {}", proj.user_facing_name, analysis.summary());

            let qnames = proj.qualified_names();
            let ecosystem = qnames
                .get(1)
                .and_then(|s| EcosystemType::from_qname(s))
                .unwrap_or(EcosystemType::Cargo);

            self.candidates.push(ProjectCandidate {
                ident: *ident,
                name: proj.user_facing_name.clone(),
                prefix: proj.prefix().escaped(),
                current_version,
                commits,
                commit_count: n_commits,
                suggested_bump,
                ecosystem,
            });
        }

        Ok(())
    }

    pub fn has_candidates(&self) -> bool {
        !self.candidates.is_empty()
    }

    pub fn cleanup(self) {
        cleanup_release_branch(self.sess, &self.base_branch, &self.release_branch);
    }

    pub fn finalize(self, selections: Vec<ProjectSelection>) -> Result<String> {
        if selections.is_empty() {
            return Err(anyhow::anyhow!("no projects selected for release"));
        }

        let mut prepared: Vec<SelectedProject> = Vec::new();

        for selection in &selections {
            let proj = self.sess.graph().lookup(selection.candidate.ident);

            let bump_scheme_text = selection
                .bump_choice
                .resolve(selection.candidate.suggested_bump);

            if bump_scheme_text == "no bump" {
                info!("{}: no version bump needed", proj.user_facing_name);
                continue;
            }

            let bump_scheme = proj
                .version
                .parse_bump_scheme(bump_scheme_text)
                .with_context(|| {
                    format!(
                        "invalid bump scheme \"{}\" for project {}",
                        bump_scheme_text, proj.user_facing_name
                    )
                })?;

            let old_version = selection.candidate.current_version.clone();

            let proj_mut = self.sess.graph_mut().lookup_mut(selection.candidate.ident);

            bump_scheme.apply(&mut proj_mut.version).with_context(|| {
                format!(
                    "failed to apply version bump to {}",
                    proj_mut.user_facing_name
                )
            })?;

            let new_version = proj_mut.version.to_string();

            info!(
                "{}: {} -> {} ({} commit{})",
                proj_mut.user_facing_name,
                old_version,
                new_version,
                selection.candidate.commit_count,
                if selection.candidate.commit_count == 1 {
                    ""
                } else {
                    "s"
                }
            );

            prepared.push(SelectedProject {
                name: proj_mut.user_facing_name.clone(),
                prefix: selection.candidate.prefix.clone(),
                old_version,
                new_version,
                bump_type: bump_scheme_text.to_string(),
                commits: selection.candidate.commits.clone(),
                ecosystem: selection.candidate.ecosystem,
                cached_changelog: selection.cached_changelog.clone(),
            });
        }

        if prepared.is_empty() {
            return Err(anyhow::anyhow!("no projects needed version bumps"));
        }

        let pipeline = ReleasePipeline::new(self.sess, self.base_branch, self.release_branch)?;
        pipeline.execute(prepared)
    }
}

#[derive(Debug, Clone)]
pub struct SelectedProject {
    pub name: String,
    pub prefix: String,
    pub old_version: String,
    pub new_version: String,
    pub bump_type: String,
    pub commits: Vec<Commit>,
    pub ecosystem: EcosystemType,
    pub cached_changelog: Option<String>,
}

pub struct ReleasePipeline<'a> {
    sess: &'a mut AppSession,
    base_branch: String,
    release_branch: String,
}

impl<'a> ReleasePipeline<'a> {
    pub fn new(
        sess: &'a mut AppSession,
        base_branch: String,
        release_branch: String,
    ) -> Result<Self> {
        Ok(Self {
            sess,
            base_branch,
            release_branch,
        })
    }

    pub fn execute(mut self, projects: Vec<SelectedProject>) -> Result<String> {
        if projects.is_empty() {
            return Err(anyhow::anyhow!("no projects to release"));
        }

        info!("updating project files with new versions...");
        let changes = self
            .sess
            .rewrite()
            .context("failed to update project files")?;

        info!("generating changelogs...");
        let (changelog_paths, changelog_contents, processed_commits) =
            self.generate_changelogs(&projects)?;

        self.print_modified_files(&changes, &changelog_paths);

        info!("creating release manifest...");
        let (_manifest, manifest_filename, manifest_repo_path) =
            self.create_manifest(&projects, &changelog_contents, &processed_commits)?;

        info!("creating release commit...");
        let all_changed_paths =
            self.collect_all_paths(&changes, &changelog_paths, &manifest_repo_path);
        self.create_commit(&projects, &all_changed_paths)?;

        info!("pushing release branch to remote...");
        self.push_branch()?;

        info!("creating pull request...");
        let pr_url =
            self.create_pull_request(&projects, &manifest_filename, &changelog_contents)?;

        self.print_summary(&projects, &pr_url);

        Ok(pr_url)
    }

    fn generate_changelogs(
        &self,
        projects: &[SelectedProject],
    ) -> Result<ChangelogGenerationResult> {
        let mut changelog_paths: Vec<RepoPathBuf> = Vec::new();
        let mut changelog_contents: HashMap<String, String> = HashMap::new();
        let mut processed_commits_map: HashMap<String, Vec<Commit>> = HashMap::new();

        let git_config = GitConfig::from_user_config(&self.sess.changelog_config);
        let changelog_config = ChangelogConfig::from_user_config(&self.sess.changelog_config);
        let bump_config = BumpConfig::from_user_config(&self.sess.bump_config);

        let github_remote = extract_github_remote(&self.sess.repo);
        let github_token = load_github_token();

        if github_remote.is_some() && github_token.is_some() {
            debug!("GitHub metadata will be fetched for changelog generation");
        }

        for project in projects {
            let params = ChangelogGenerationParams {
                repo: &self.sess.repo,
                project_name: &project.name,
                prefix: &project.prefix,
                version: Some(&project.new_version),
                commits: &project.commits,
                git_config: &git_config,
                changelog_config: &changelog_config,
                bump_config: &bump_config,
                write_to_file: true,
                custom_output_path: None,
                github_owner: github_remote.as_ref().map(|r| r.owner.as_str()),
                github_repo: github_remote.as_ref().map(|r| r.repo.as_str()),
                github_token: github_token.clone(),
            };
            let result = generate_and_write_project_changelog(&params)?;

            changelog_contents.insert(project.name.clone(), result.content);
            processed_commits_map.insert(project.name.clone(), result.processed_commits);

            if let Some(path) = result.path {
                changelog_paths.push(path);
            } else if !result.has_user_changes {
                info!(
                    "{}: no user-facing changes, skipping changelog file update",
                    project.name
                );
            }
        }

        Ok((changelog_paths, changelog_contents, processed_commits_map))
    }

    fn print_modified_files(&self, changes: &ChangeList, changelog_paths: &[RepoPathBuf]) {
        let paths: Vec<_> = changes
            .paths()
            .chain(changelog_paths.iter().map(|p| p.as_ref()))
            .collect();

        if !paths.is_empty() {
            info!("modified files:");
            for path in paths {
                info!("  {}", path.escaped());
            }
        }
    }

    fn create_manifest(
        &mut self,
        projects: &[SelectedProject],
        changelog_contents: &HashMap<String, String>,
        processed_commits: &HashMap<String, Vec<Commit>>,
    ) -> Result<(ReleaseManifest, String, RepoPathBuf)> {
        let git_user = self
            .sess
            .repo
            .get_signature()
            .map(|sig| sig.name().unwrap_or("unknown").to_string())
            .unwrap_or_else(|_| "belaf-ci".to_string());

        let github_base_url = self.get_github_compare_base_url();

        let mut manifest = ReleaseManifest::new(self.base_branch.clone(), git_user);

        for project in projects {
            let changelog_content = changelog_contents
                .get(&project.name)
                .cloned()
                .unwrap_or_default();

            let commits = processed_commits
                .get(&project.name)
                .map(|c| c.as_slice())
                .unwrap_or(&project.commits);

            let contributors = Self::extract_contributors(commits);
            let first_time_contributors = Self::extract_first_time_contributors(commits);
            let statistics = Self::extract_commit_statistics(commits);

            let mut release = ProjectRelease::new(
                project.name.clone(),
                project.ecosystem.display_name().to_string(),
                project.old_version.clone(),
                project.new_version.clone(),
                project.bump_type.clone(),
                changelog_content,
                project.prefix.clone(),
            )
            .with_contributors(contributors)
            .with_first_time_contributors(first_time_contributors)
            .with_statistics(statistics);

            if let Some(base_url) = &github_base_url {
                release = release.with_compare_url(base_url, |tag| self.sess.repo.tag_exists(tag));
            }

            manifest.add_release(release);
        }

        let manifest_dir = self
            .sess
            .repo
            .resolve_workdir(RepoPathBuf::new(MANIFEST_DIR.as_bytes()).as_ref());
        std::fs::create_dir_all(&manifest_dir)
            .context(format!("failed to create {} directory", MANIFEST_DIR))?;

        let manifest_filename = ReleaseManifest::generate_filename();
        let manifest_path = manifest_dir.join(&manifest_filename);

        manifest
            .save_to_file(&manifest_path)
            .context("failed to save release manifest")?;

        info!("wrote manifest to {}/{}", MANIFEST_DIR, manifest_filename);

        let manifest_repo_path =
            RepoPathBuf::new(format!("{}/{}", MANIFEST_DIR, manifest_filename).as_bytes());

        Ok((manifest, manifest_filename, manifest_repo_path))
    }

    fn collect_all_paths<'b>(
        &self,
        changes: &'b ChangeList,
        changelog_paths: &'b [RepoPathBuf],
        manifest_repo_path: &'b RepoPathBuf,
    ) -> Vec<&'b crate::core::git::repository::RepoPath> {
        changes
            .paths()
            .chain(changelog_paths.iter().map(|p| p.as_ref()))
            .chain(std::iter::once(manifest_repo_path.as_ref()))
            .collect()
    }

    fn create_commit(
        &self,
        projects: &[SelectedProject],
        all_changed_paths: &[&crate::core::git::repository::RepoPath],
    ) -> Result<()> {
        let commit_message = format_commit_message(projects);
        self.sess
            .repo
            .create_commit(&commit_message, all_changed_paths)
            .context("failed to create release commit")?;
        Ok(())
    }

    fn push_branch(&self) -> Result<()> {
        self.sess
            .repo
            .push_branch(&self.release_branch)
            .context("failed to push release branch")?;
        Ok(())
    }

    fn create_pull_request(
        &self,
        projects: &[SelectedProject],
        manifest_filename: &str,
        changelog_contents: &HashMap<String, String>,
    ) -> Result<String> {
        let github =
            GitHubInformation::new(self.sess).context("failed to initialize GitHub client")?;

        let pr_title = pr::generate_pr_title(projects);
        let pr_body = pr::generate_pr_body(projects, manifest_filename, changelog_contents);

        let pr_url = github
            .create_pull_request(&self.release_branch, &self.base_branch, &pr_title, &pr_body)
            .context("failed to create pull request")?;

        Ok(pr_url)
    }

    fn print_summary(&self, projects: &[SelectedProject], pr_url: &str) {
        info!(
            "prepared {} project{} for release",
            projects.len(),
            if projects.len() == 1 { "" } else { "s" }
        );
        info!("pull request created: {}", pr_url);
    }

    fn get_github_compare_base_url(&self) -> Option<String> {
        self.sess.repo.upstream_url().ok().and_then(|url| {
            let url = url
                .trim_end_matches(".git")
                .replace("git@github.com:", "https://github.com/");
            if url.contains("github.com") {
                Some(url)
            } else {
                None
            }
        })
    }

    fn extract_contributors(commits: &[Commit]) -> Vec<String> {
        let mut contributors: Vec<String> = commits
            .iter()
            .filter_map(|c| c.author.name.clone())
            .collect();
        contributors.sort();
        contributors.dedup();
        contributors
    }

    fn extract_first_time_contributors(commits: &[Commit]) -> Vec<String> {
        let mut first_timers: Vec<String> = commits
            .iter()
            .filter_map(|c| {
                c.remote.as_ref().and_then(|r| {
                    if r.is_first_time {
                        r.username.clone()
                    } else {
                        None
                    }
                })
            })
            .collect();
        first_timers.sort();
        first_timers.dedup();
        first_timers
    }

    fn extract_commit_statistics(commits: &[Commit]) -> ReleaseStatistics {
        let commit_count = commits.len();

        let breaking_changes_count = commits
            .iter()
            .filter(|c| c.conv.as_ref().map(|conv| conv.breaking).unwrap_or(false))
            .count();

        let features_count = commits
            .iter()
            .filter(|c| {
                c.conv
                    .as_ref()
                    .map(|conv| conv.type_ == "feat")
                    .unwrap_or(false)
            })
            .count();

        let fixes_count = commits
            .iter()
            .filter(|c| {
                c.conv
                    .as_ref()
                    .map(|conv| conv.type_ == "fix")
                    .unwrap_or(false)
            })
            .count();

        let pr_count_value = commits
            .iter()
            .filter(|c| c.remote.as_ref().and_then(|r| r.pr_number).is_some())
            .count();

        ReleaseStatistics {
            commit_count,
            days_since_last_release: None,
            breaking_changes_count,
            features_count,
            fixes_count,
            pr_count: if pr_count_value > 0 {
                Some(pr_count_value)
            } else {
                None
            },
        }
    }
}

pub fn create_release_branch(sess: &mut AppSession) -> Result<(String, String)> {
    let base_branch = sess
        .repo
        .current_branch_name()
        .context("failed to get current branch")?
        .ok_or_else(|| anyhow::anyhow!("not on a branch (detached HEAD state)"))?;

    let release_branch = Repository::generate_release_branch_name();
    info!("creating release branch: {}", release_branch);

    sess.repo
        .create_branch(&release_branch)
        .context("failed to create release branch")?;
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

fn format_commit_message(projects: &[SelectedProject]) -> String {
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

pub fn generate_changelog_entry(
    version: &str,
    commits: &[Commit],
    git_config: &GitConfig,
    changelog_config: &ChangelogConfig,
    bump_config: &BumpConfig,
) -> Result<String> {
    if commits.is_empty() {
        let now = time::OffsetDateTime::now_utc();
        return Ok(format!(
            "## [{}] - {}\n\n\
            No user-facing changes in this release.\n\
            (Internal: docs, chore, ci, test, style)\n",
            version,
            now.date()
        ));
    }

    let now = time::OffsetDateTime::now_utc();
    let release = Release {
        version: Some(version.to_string()),
        commits: commits.to_vec(),
        timestamp: Some(now.unix_timestamp()),
        ..Default::default()
    };

    let mut changelog = Changelog::new(
        vec![release],
        git_config.clone(),
        changelog_config.clone(),
        bump_config.clone(),
    )?;
    changelog.process_commits()?;

    let mut output = Vec::new();
    changelog.generate(&mut output)?;

    String::from_utf8(output).context("changelog contains invalid UTF-8")
}

pub struct GitHubRemoteInfo {
    pub owner: String,
    pub repo: String,
}

pub fn extract_github_remote(repo: &Repository) -> Option<GitHubRemoteInfo> {
    let upstream_url = repo.upstream_url().ok()?;
    let parsed = git_url_parse::GitUrl::parse(&upstream_url).ok()?;
    let provider: git_url_parse::types::provider::GenericProvider = parsed.provider_info().ok()?;

    Some(GitHubRemoteInfo {
        owner: provider.owner().to_string(),
        repo: provider.repo().to_string(),
    })
}

pub fn load_github_token() -> Option<crate::core::api::StoredToken> {
    crate::core::auth::token::load_token()
        .ok()
        .flatten()
        .filter(|t| !t.is_expired())
}

#[derive(Debug)]
pub struct ChangelogResult {
    pub content: String,
    pub path: Option<RepoPathBuf>,
    pub has_user_changes: bool,
    pub processed_commits: Vec<Commit>,
}

pub struct ChangelogGenerationParams<'a> {
    pub repo: &'a Repository,
    pub project_name: &'a str,
    pub prefix: &'a str,
    pub version: Option<&'a str>,
    pub commits: &'a [Commit],
    pub git_config: &'a GitConfig,
    pub changelog_config: &'a ChangelogConfig,
    pub bump_config: &'a BumpConfig,
    pub write_to_file: bool,
    pub custom_output_path: Option<&'a str>,
    pub github_owner: Option<&'a str>,
    pub github_repo: Option<&'a str>,
    pub github_token: Option<crate::core::api::StoredToken>,
}

pub fn generate_and_write_project_changelog(
    params: &ChangelogGenerationParams,
) -> Result<ChangelogResult> {
    let repo = params.repo;
    let project_name = params.project_name;
    let prefix = params.prefix;
    let version = params.version;
    let commits = params.commits;
    let git_config = params.git_config;
    let changelog_config = params.changelog_config;
    let bump_config = params.bump_config;
    let write_to_file = params.write_to_file;
    let custom_output_path = params.custom_output_path;
    if commits.is_empty() {
        let now = time::OffsetDateTime::now_utc();
        let version_str = version.unwrap_or("Unreleased");
        let content = format!(
            "## [{}] - {}\n\nNo user-facing changes in this release.\n(Internal: docs, chore, ci, test, style)\n",
            version_str,
            now.date()
        );
        return Ok(ChangelogResult {
            content,
            path: None,
            has_user_changes: false,
            processed_commits: Vec::new(),
        });
    }

    let now = time::OffsetDateTime::now_utc();
    let release = Release {
        version: version.map(String::from),
        commits: commits.to_vec(),
        timestamp: Some(now.unix_timestamp()),
        ..Default::default()
    };

    let mut changelog = Changelog::new(
        vec![release],
        git_config.clone(),
        changelog_config.clone(),
        bump_config.clone(),
    )?;

    if let (Some(owner), Some(repo_name)) = (params.github_owner, params.github_repo) {
        changelog = changelog.with_remote(owner.to_string(), repo_name.to_string());
        if let Some(token) = params.github_token.clone() {
            changelog = changelog.with_github_token(token);
        }
    }

    changelog.process_commits()?;
    changelog.add_github_metadata_sync(None)?;

    let commit_list = changelog
        .releases
        .first()
        .map(|r| r.commits.clone())
        .unwrap_or_default();

    if commit_list.is_empty() {
        let now = time::OffsetDateTime::now_utc();
        let version_str = version.unwrap_or("Unreleased");
        let content = format!(
            "## [{}] - {}\n\nNo user-facing changes in this release.\n(Internal: docs, chore, ci, test, style)\n",
            version_str,
            now.date()
        );
        return Ok(ChangelogResult {
            content,
            path: None,
            has_user_changes: false,
            processed_commits: Vec::new(),
        });
    }

    let mut output = Vec::new();
    changelog.generate(&mut output)?;
    let generated_content =
        String::from_utf8(output).context("changelog contains invalid UTF-8")?;

    if !write_to_file {
        return Ok(ChangelogResult {
            content: generated_content,
            path: None,
            has_user_changes: true,
            processed_commits: commit_list.clone(),
        });
    }

    let default_output = changelog_config
        .output
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "CHANGELOG.md".to_string());

    let changelog_rel_path = if let Some(path) = custom_output_path {
        path.to_string()
    } else if prefix.is_empty() {
        default_output
    } else {
        let prefix = prefix.trim_end_matches('/');
        format!("{}/{}", prefix, default_output)
    };

    let changelog_repo_path = RepoPathBuf::new(changelog_rel_path.as_bytes());
    let changelog_full_path = repo.resolve_workdir(changelog_repo_path.as_ref());

    let existing_content = std::fs::read_to_string(&changelog_full_path).unwrap_or_default();

    let mut prepend_output = Vec::new();
    changelog.prepend(existing_content, &mut prepend_output)?;
    let final_content =
        String::from_utf8(prepend_output).context("changelog contains invalid UTF-8")?;

    if let Some(parent) = changelog_full_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create directory for {}",
                changelog_full_path.display()
            )
        })?;
    }

    std::fs::write(&changelog_full_path, &final_content).with_context(|| {
        format!(
            "failed to write changelog to {}",
            changelog_full_path.display()
        )
    })?;

    info!(
        "{}: wrote changelog to {}",
        project_name, changelog_rel_path
    );

    Ok(ChangelogResult {
        content: generated_content,
        path: Some(changelog_repo_path),
        has_user_changes: true,
        processed_commits: commit_list,
    })
}
