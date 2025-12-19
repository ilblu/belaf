//! Release workflow orchestration for PR-based releases.
//!
//! This module implements the [`ReleasePipeline`] which coordinates the complete
//! release workflow:
//!
//! 1. Version bumping across all selected projects
//! 2. Changelog generation (with optional AI enhancement)
//! 3. Release manifest creation in `belaf/releases/`
//! 4. Git branch management (create, commit, push)
//! 5. GitHub Pull Request creation
//!
//! The workflow is designed for CI/CD environments where releases go through
//! a PR review process before being finalized by a GitHub App.

use anyhow::{Context, Result};
use std::collections::HashMap;
use tracing::{info, warn};

use crate::core::{
    ecosystem::types::EcosystemType,
    github::client::GitHubInformation,
    release::{
        changelog_generator::{self, ChangelogEntry},
        commit_analyzer,
        manifest::{ProjectRelease, ReleaseManifest, MANIFEST_DIR},
        repository::{ChangeList, RepoPathBuf, Repository},
        session::AppSession,
    },
};

use super::pr_generator;

#[derive(Debug, Clone)]
pub struct SelectedProject {
    pub name: String,
    pub prefix: String,
    pub old_version: String,
    pub new_version: String,
    pub bump_type: String,
    pub commit_messages: Vec<String>,
    pub ecosystem: EcosystemType,
    pub cached_changelog: Option<String>,
}

pub struct ReleasePipeline<'a> {
    sess: &'a mut AppSession,
    base_branch: String,
    release_branch: String,
    ai_enabled: bool,
}

impl<'a> ReleasePipeline<'a> {
    pub fn new(
        sess: &'a mut AppSession,
        base_branch: String,
        release_branch: String,
    ) -> Result<Self> {
        let ai_enabled = sess.changelog_config.ai_enabled;

        Ok(Self {
            sess,
            base_branch,
            release_branch,
            ai_enabled,
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
        use crate::core::ai::changelog::AiChangelogGenerator;

        let mut changelog_paths: Vec<RepoPathBuf> = Vec::new();
        let mut changelog_contents: HashMap<String, String> = HashMap::new();

        // AI Caching Strategy:
        // - Projects may have a `cached_changelog` from a previous TUI selection session
        // - We only initialize the AI generator if at least one project:
        //   1. Has no cached changelog (needs AI polish)
        //   2. Has user-facing commits (categorized commits not empty)
        // - The generator is created once and reused for all projects
        let ai_generator: Option<AiChangelogGenerator> = if self.ai_enabled {
            let needs_ai = projects.iter().any(|p| {
                p.cached_changelog.is_none()
                    && !commit_analyzer::categorize_commits(&p.commit_messages).is_empty()
            });

            if needs_ai {
                let init_future = AiChangelogGenerator::new();
                let result = match tokio::runtime::Handle::try_current() {
                    Ok(handle) => tokio::task::block_in_place(|| handle.block_on(init_future)),
                    Err(_) => match tokio::runtime::Runtime::new() {
                        Ok(rt) => rt.block_on(init_future),
                        Err(e) => {
                            warn!("failed to create async runtime: {}", e);
                            return Ok((changelog_paths, changelog_contents));
                        }
                    },
                };

                match result {
                    Ok(gen) => Some(gen),
                    Err(e) => {
                        warn!("failed to initialize AI generator: {}", e);
                        None
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

        for project in projects {
            let categorized = commit_analyzer::categorize_commits(&project.commit_messages);

            if categorized.is_empty() {
                info!(
                    "{}: no user-facing changes, skipping changelog file update",
                    project.name
                );
                changelog_contents.insert(
                    project.name.clone(),
                    format!(
                        "## [{}] - {}\n\nInternal changes only (no user-facing changes).\n",
                        project.new_version,
                        time::OffsetDateTime::now_utc().date()
                    ),
                );
                continue;
            }

            let mut entry = ChangelogEntry::new(project.new_version.clone());
            entry.add_commits(&categorized);

            let draft_changelog = entry.to_markdown();

            let final_changelog_entry = if self.ai_enabled {
                if let Some(cached) = &project.cached_changelog {
                    cached.clone()
                } else if let Some(ref generator) = ai_generator {
                    info!("{}: polishing changelog with AI...", project.name);
                    let draft_clone = draft_changelog.clone();
                    let polish_future = generator.polish(&draft_clone, &project.commit_messages);
                    let result = match tokio::runtime::Handle::try_current() {
                        Ok(handle) => {
                            tokio::task::block_in_place(|| handle.block_on(polish_future))
                        }
                        Err(_) => match tokio::runtime::Runtime::new() {
                            Ok(rt) => rt.block_on(polish_future),
                            Err(e) => {
                                warn!(
                                    "{}: failed to create runtime ({}), using standard changelog",
                                    project.name, e
                                );
                                Ok(draft_changelog.clone())
                            }
                        },
                    };

                    match result {
                        Ok(polished) => polished,
                        Err(e) => {
                            warn!(
                                "{}: AI polish failed ({}), using standard changelog",
                                project.name, e
                            );
                            draft_changelog
                        }
                    }
                } else {
                    draft_changelog
                }
            } else {
                project
                    .cached_changelog
                    .clone()
                    .unwrap_or(draft_changelog.clone())
            };

            changelog_contents.insert(project.name.clone(), final_changelog_entry.clone());

            let changelog_rel_path = if project.prefix.is_empty() {
                "CHANGELOG.md".to_string()
            } else {
                format!("{}/CHANGELOG.md", project.prefix)
            };

            let changelog_repo_path = RepoPathBuf::new(changelog_rel_path.as_bytes());
            let changelog_full_path = self.sess.repo.resolve_workdir(changelog_repo_path.as_ref());

            let existing_content =
                changelog_generator::parse_existing_changelog(&changelog_full_path)
                    .unwrap_or_default();

            let full_changelog =
                changelog_generator::generate_changelog(&project.name, &entry, &existing_content);

            let final_content = if self.ai_enabled && !final_changelog_entry.is_empty() {
                let header = format!(
                    "# Changelog\n\n\
                    All notable changes to {} will be documented in this file.\n\n\
                    The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),\n\
                    and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).\n\n",
                    project.name
                );
                let ai_entry = if final_changelog_entry.starts_with("## [") {
                    final_changelog_entry.clone()
                } else {
                    entry.to_markdown()
                };
                format!("{}{}\n{}", header, ai_entry, existing_content)
            } else {
                full_changelog
            };

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
    ) -> Vec<&'b crate::core::release::repository::RepoPath> {
        changes
            .paths()
            .chain(changelog_paths.iter().map(|p| p.as_ref()))
            .chain(std::iter::once(manifest_repo_path.as_ref()))
            .collect()
    }

    fn create_commit(
        &self,
        projects: &[SelectedProject],
        all_changed_paths: &[&crate::core::release::repository::RepoPath],
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

        let pr_title = pr_generator::generate_pr_title(projects);
        let pr_body =
            pr_generator::generate_pr_body(projects, manifest_filename, changelog_contents);

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
        tracing::warn!("failed to delete release branch '{}': {}", release_branch, e);
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

pub fn polish_changelog_with_ai(draft: &str, commits: &[String]) -> Result<String> {
    use crate::core::ai::changelog::AiChangelogGenerator;

    let future = async {
        let generator = AiChangelogGenerator::new()
            .await
            .context("failed to initialize AI changelog generator")?;

        generator
            .polish(draft, commits)
            .await
            .context("failed to polish changelog with AI")
    };

    match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(future)),
        Err(_) => {
            let rt = tokio::runtime::Runtime::new().context("failed to create async runtime")?;
            rt.block_on(future)
        }
    }
}

pub fn generate_changelog_body(commit_messages: &[String]) -> String {
    let categorized = commit_analyzer::categorize_commits(commit_messages);

    if categorized.is_empty() {
        return "## No User-Facing Changes\n\n\
            All commits are internal (docs, chore, ci, test, style).\n\
            These are typically excluded from user-facing changelogs.\n"
            .to_string();
    }

    use std::collections::BTreeMap;
    let mut by_category: BTreeMap<
        commit_analyzer::ChangelogCategory,
        Vec<&commit_analyzer::CategorizedCommit>,
    > = BTreeMap::new();

    for commit in &categorized {
        by_category.entry(commit.category).or_default().push(commit);
    }

    let mut content = String::new();
    for (category, commits) in by_category {
        content.push_str(&format!("## {}\n\n", category.as_str()));
        for commit in commits {
            content.push_str(&commit.format_for_changelog());
            content.push('\n');
        }
        content.push('\n');
    }

    content
}
