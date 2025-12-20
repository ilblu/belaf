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
use tracing::info;

use crate::core::{
    bump::BumpConfig,
    changelog::{Changelog, ChangelogConfig, Commit, GitConfig, Release},
    ecosystem::types::EcosystemType,
    git::repository::{ChangeList, RepoPathBuf, Repository},
    github::{client::GitHubInformation, pr},
    manifest::{ProjectRelease, ReleaseManifest, MANIFEST_DIR},
    session::AppSession,
};

#[derive(Debug, Clone)]
pub struct SelectedProject {
    pub name: String,
    pub prefix: String,
    pub old_version: String,
    pub new_version: String,
    pub bump_type: String,
    pub commits: Vec<Commit<'static>>,
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
        let (changelog_paths, changelog_contents) = self.generate_changelogs(&projects)?;

        self.print_modified_files(&changes, &changelog_paths);

        info!("creating release manifest...");
        let (_manifest, manifest_filename, manifest_repo_path) =
            self.create_manifest(&projects, &changelog_contents)?;

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
    ) -> Result<(Vec<RepoPathBuf>, HashMap<String, String>)> {
        let mut changelog_paths: Vec<RepoPathBuf> = Vec::new();
        let mut changelog_contents: HashMap<String, String> = HashMap::new();

        let git_config = GitConfig::from_user_config(&self.sess.changelog_config);
        let changelog_config = ChangelogConfig::from_user_config(&self.sess.changelog_config);
        let bump_config = BumpConfig::from_user_config(&self.sess.bump_config);

        for project in projects {
            let changelog_entry = self.generate_project_changelog(
                project,
                &git_config,
                &changelog_config,
                &bump_config,
            )?;

            if changelog_entry.is_empty() {
                info!(
                    "{}: no user-facing changes, skipping changelog file update",
                    project.name
                );
                let now = time::OffsetDateTime::now_utc();
                changelog_contents.insert(
                    project.name.clone(),
                    format!(
                        "## [{}] - {}\n\nInternal changes only (no user-facing changes).\n",
                        project.new_version,
                        now.date()
                    ),
                );
                continue;
            }

            changelog_contents.insert(project.name.clone(), changelog_entry.clone());

            let changelog_rel_path = if project.prefix.is_empty() {
                "CHANGELOG.md".to_string()
            } else {
                let prefix = project.prefix.trim_end_matches('/');
                format!("{}/CHANGELOG.md", prefix)
            };

            let changelog_repo_path = RepoPathBuf::new(changelog_rel_path.as_bytes());
            let changelog_full_path = self.sess.repo.resolve_workdir(changelog_repo_path.as_ref());

            let existing_content =
                std::fs::read_to_string(&changelog_full_path).unwrap_or_default();

            let now = time::OffsetDateTime::now_utc();
            let release = Release {
                version: Some(project.new_version.clone()),
                commits: project.commits.clone(),
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
            changelog.prepend(existing_content, &mut output)?;

            let final_content =
                String::from_utf8(output).context("changelog contains invalid UTF-8")?;

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

            changelog_paths.push(changelog_repo_path);
            info!(
                "{}: wrote changelog to {}",
                project.name, changelog_rel_path
            );
        }

        Ok((changelog_paths, changelog_contents))
    }

    fn generate_project_changelog(
        &self,
        project: &SelectedProject,
        git_config: &GitConfig,
        changelog_config: &ChangelogConfig,
        bump_config: &BumpConfig,
    ) -> Result<String> {
        if project.commits.is_empty() {
            return Ok(String::new());
        }

        let now = time::OffsetDateTime::now_utc();
        let release = Release {
            version: Some(project.new_version.clone()),
            commits: project.commits.clone(),
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

        let processed_commits = changelog
            .releases
            .first()
            .map(|r| r.commits.len())
            .unwrap_or(0);

        if processed_commits == 0 {
            return Ok(String::new());
        }

        let mut output = Vec::new();
        changelog.generate(&mut output)?;

        String::from_utf8(output).context("changelog contains invalid UTF-8")
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
    ) -> Result<(ReleaseManifest, String, RepoPathBuf)> {
        let git_user = self
            .sess
            .repo
            .get_signature()
            .map(|sig| sig.name().unwrap_or("unknown").to_string())
            .unwrap_or_else(|_| "belaf-ci".to_string());

        let mut manifest = ReleaseManifest::new(self.base_branch.clone(), git_user);

        for project in projects {
            let changelog_content = changelog_contents
                .get(&project.name)
                .cloned()
                .unwrap_or_default();

            let release = ProjectRelease::new(
                project.name.clone(),
                project.ecosystem.display_name().to_string(),
                project.old_version.clone(),
                project.new_version.clone(),
                project.bump_type.clone(),
                changelog_content,
                project.prefix.clone(),
            );

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
    commits: &[Commit<'static>],
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
