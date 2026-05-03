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
    api::{ApiClient, ApiError},
    auth::token::load_token,
    bump::{self, BumpConfig, BumpRecommendation},
    changelog::{ChangelogConfig, Commit, GitConfig},
    config::syntax::{BumpConfiguration, ChangelogConfiguration},
    ecosystem::format_handler::FormatHandlerRegistry,
    git::repository::{ChangeList, RepoPathBuf, Repository},
    github::{client::GitHubInformation, pr},
    graph::GraphQueryBuilder,
    group::GroupSet,
    manifest::{ReleaseEntry, ReleaseManifest, ReleaseStatistics, MANIFEST_DIR},
    resolved_release_unit::ReleaseUnitId,
    session::AppSession,
    tag_format::{format_tag, split_maven_coords, TagFormatInputs},
    wire::known::Ecosystem,
};

#[derive(Debug, Clone)]
pub struct ReleaseUnitCandidate {
    pub ident: ReleaseUnitId,
    pub name: String,
    pub prefix: String,
    pub current_version: String,
    pub commits: Vec<Commit>,
    pub commit_count: usize,
    pub suggested_bump: BumpRecommendation,
    pub ecosystem: Ecosystem,
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
pub struct ReleaseUnitSelection {
    pub candidate: ReleaseUnitCandidate,
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
    pub candidates: Vec<ReleaseUnitCandidate>,
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
            let unit = self.sess.graph().lookup(*ident);
            let history = histories.lookup(*ident);
            let n_commits = history.n_commits();

            if n_commits == 0 {
                info!(
                    "{}: no changes since last release, skipping",
                    unit.user_facing_name
                );
                continue;
            }

            let commits: Vec<Commit> = history
                .commits()
                .into_iter()
                .filter_map(|cid| self.sess.repo.get_commit_details(*cid).ok())
                .collect();

            let current_version = unit.version.to_string();

            let analysis = bump::analyze_commits(&commits).with_context(|| {
                format!(
                    "failed to analyze commit messages for {}",
                    unit.user_facing_name
                )
            })?;

            let bump_config = BumpConfig::from_user_config(&self.bump_config);
            let suggested_bump = analysis
                .recommendation
                .apply_config(&bump_config, Some(&current_version));

            info!("{}: {}", unit.user_facing_name, analysis.summary());

            let qnames = unit.qualified_names();
            let ecosystem = qnames
                .get(1)
                .map(|s| Ecosystem::classify(s))
                .unwrap_or_else(|| Ecosystem::classify("cargo"));

            self.candidates.push(ReleaseUnitCandidate {
                ident: *ident,
                name: unit.user_facing_name.clone(),
                prefix: unit.prefix().escaped(),
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

    pub fn finalize(self, selections: Vec<ReleaseUnitSelection>) -> Result<String> {
        if selections.is_empty() {
            return Err(anyhow::anyhow!("no projects selected for release"));
        }

        let mut prepared: Vec<SelectedReleaseUnit> = Vec::new();

        for selection in &selections {
            let unit = self.sess.graph().lookup(selection.candidate.ident);

            let bump_scheme_text = selection
                .bump_choice
                .resolve(selection.candidate.suggested_bump);

            if bump_scheme_text == "no bump" {
                info!("{}: no version bump needed", unit.user_facing_name);
                continue;
            }

            let bump_scheme = unit
                .version
                .parse_bump_scheme(bump_scheme_text)
                .with_context(|| {
                    format!(
                        "invalid bump scheme \"{}\" for project {}",
                        bump_scheme_text, unit.user_facing_name
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

            prepared.push(SelectedReleaseUnit {
                ident: selection.candidate.ident,
                name: proj_mut.user_facing_name.clone(),
                prefix: selection.candidate.prefix.clone(),
                old_version,
                new_version,
                bump_type: bump_scheme_text.to_string(),
                commits: selection.candidate.commits.clone(),
                ecosystem: selection.candidate.ecosystem.clone(),
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
pub struct SelectedReleaseUnit {
    pub ident: ReleaseUnitId,
    pub name: String,
    pub prefix: String,
    pub old_version: String,
    pub new_version: String,
    pub bump_type: String,
    pub commits: Vec<Commit>,
    pub ecosystem: Ecosystem,
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

    pub fn execute(mut self, projects: Vec<SelectedReleaseUnit>) -> Result<String> {
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
        projects: &[SelectedReleaseUnit],
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
        projects: &[SelectedReleaseUnit],
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

        // Emit `groups[]` entries for any group that has at least one
        // member in this release set. The github-app reads this to drive
        // atomic group releases (G6) — releases sharing a `group_id` are
        // tagged + published as one transaction.
        let groups = self.sess.graph().groups();
        let mut emitted_groups: HashMap<String, Vec<String>> = HashMap::new();
        for project in projects {
            if let Some(g) = groups.group_of(project.ident) {
                emitted_groups
                    .entry(g.id.as_str().to_string())
                    .or_default()
                    .push(project.name.clone());
            }
        }
        // Stable order so the manifest diff is deterministic.
        let mut emitted_keys: Vec<&String> = emitted_groups.keys().collect();
        emitted_keys.sort();
        for key in emitted_keys {
            let members = &emitted_groups[key];
            manifest.add_group(crate::core::manifest::Group {
                id: key.clone(),
                members: members.clone(),
                x: serde_json::Map::new(),
            });
        }

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

            let mut release = ReleaseEntry::new(
                project.name.clone(),
                project.ecosystem.as_str().to_string(),
                project.old_version.clone(),
                project.new_version.clone(),
                project.bump_type.clone(),
                changelog_content,
                project.prefix.clone(),
            )
            .with_contributors(contributors)
            .with_first_time_contributors(first_time_contributors)
            .with_statistics(statistics);

            // B10: per-ecosystem tag_format with project / group overrides.
            // Precedence: [project."<name>".tag_format] > [group.<id>.tag_format]
            // > ecosystem default. Validation errors fail the whole prepare —
            // we'd rather catch a bad template here than surprise users with
            // a github-app rollback when it tries to push the broken tag.
            let tag_name = build_tag_name(self.sess, project, groups)?;
            release.tag_name = tag_name;
            // The previous_tag default uses the same prefix-based scheme,
            // which doesn't compose with the new tag_format. Recompute it
            // from the new tag's "shape": replace the new version with the
            // old version textually. This is best-effort — if it doesn't
            // produce a real tag, `with_compare_url` will catch it below
            // and clear `previous_tag` again.
            release.previous_tag = release.previous_tag.as_ref().map(|_| {
                release
                    .tag_name
                    .replacen(&project.new_version, &project.old_version, 1)
            });

            if let Some(g) = groups.group_of(project.ident) {
                release = release.with_group_id(g.id.as_str());
            }

            // Carry typed wire fields from the ResolvedReleaseUnit, if
            // the user declared one. Auto-detected projects (no source
            // unit) leave the fields at their empty defaults.
            if let Some(unit) = self
                .sess
                .resolved_release_units()
                .iter()
                .find(|r| r.unit.name == project.name)
            {
                if let crate::core::release_unit::VersionSource::Manifests(ms) = &unit.unit.source {
                    let bundle: Vec<String> =
                        ms.iter().map(|m| m.path.escaped().to_string()).collect();
                    if !bundle.is_empty() {
                        release = release.with_bundle_manifests(bundle);
                    }
                    // version_field_spec — use the first manifest's
                    // spec as the unit-level value (multi-manifest
                    // bundles share the same ecosystem and so the
                    // same spec; mixed-spec is rejected by the
                    // resolver in Phase B).
                    if let Some(first) = ms.first() {
                        release = release.with_version_field_spec(first.version_field.wire_key());
                    }
                }
                if let crate::core::release_unit::VersionSource::External(ext) = &unit.unit.source {
                    release = release.with_external_versioner(
                        crate::core::wire::domain::ExternalVersionerWire {
                            tool: ext.tool.clone(),
                            read_command: Some(ext.read_command.clone()),
                            write_command: Some(ext.write_command.clone()),
                            cwd: ext.cwd.as_ref().map(|p| p.escaped().to_string()),
                            timeout_sec: Some(ext.timeout_sec as i64),
                            env: if ext.env.is_empty() {
                                None
                            } else {
                                Some(ext.env.clone())
                            },
                        },
                    );
                }
                if !unit.unit.satellites.is_empty() {
                    release = release.with_satellites(
                        unit.unit
                            .satellites
                            .iter()
                            .map(|p| p.escaped().to_string())
                            .collect(),
                    );
                }
                if let Some(cascade) = &unit.unit.cascade_from {
                    release =
                        release.with_cascade_from(crate::core::wire::domain::CascadeFromWire {
                            source: cascade.source.clone(),
                            bump: cascade.bump.wire_key().to_string(),
                        });
                }
                if unit.unit.visibility != crate::core::release_unit::Visibility::Public {
                    release = release.with_visibility(unit.unit.visibility.wire_key());
                }
            }

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

        let manifest_filename = manifest.generate_filename();
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
        projects: &[SelectedReleaseUnit],
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
        let git_token = self.fetch_git_credentials()?;
        self.sess
            .repo
            .push_branch(&self.release_branch, Some(&git_token))
            .context("failed to push release branch")?;
        Ok(())
    }

    fn fetch_git_credentials(&self) -> Result<String> {
        let token = load_token()
            .context("failed to load token")?
            .context("not authenticated - run 'belaf install' first")?;

        let upstream_url = self
            .sess
            .repo
            .upstream_url()
            .context("failed to get upstream URL")?;

        let (owner, repo) =
            parse_github_url(&upstream_url).context("failed to parse GitHub URL from upstream")?;

        let api_client = ApiClient::new();

        let future = async {
            api_client
                .get_git_credentials(&token, &owner, &repo)
                .await
                .map_err(|e| match &e {
                    ApiError::ApiResponse { status, message } => {
                        anyhow::anyhow!("failed to get git credentials ({}): {}", status, message)
                    }
                    ApiError::Unauthorized => {
                        anyhow::anyhow!(
                            "authentication expired - run 'belaf login' to re-authenticate"
                        )
                    }
                    _ => anyhow::anyhow!("failed to get git credentials: {}", e),
                })
        };

        let credentials = match tokio::runtime::Handle::try_current() {
            Ok(handle) => tokio::task::block_in_place(|| handle.block_on(future)),
            Err(_) => {
                let rt =
                    tokio::runtime::Runtime::new().context("failed to create async runtime")?;
                rt.block_on(future)
            }
        }?;

        Ok(credentials.token)
    }

    fn create_pull_request(
        &self,
        projects: &[SelectedReleaseUnit],
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

    fn print_summary(&self, projects: &[SelectedReleaseUnit], pr_url: &str) {
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
            commit_count: commit_count as u64,
            days_since_last_release: None,
            breaking_changes_count: breaking_changes_count as u64,
            features_count: features_count as u64,
            fixes_count: fixes_count as u64,
            pr_count: if pr_count_value > 0 {
                Some(pr_count_value as u64)
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

fn format_commit_message(projects: &[SelectedReleaseUnit]) -> String {
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

/// Resolve the per-release tag name using the precedence chain:
/// `[release_unit.<name>].tag_format` > `[group.<id>].tag_format` > the
/// ecosystem trait's `tag_format_default()`. The ecosystem registry
/// is instantiated fresh here
/// (the Loader instances are stateless once `finalize` has run).
fn build_tag_name(
    sess: &AppSession,
    project: &SelectedReleaseUnit,
    groups: &GroupSet,
) -> Result<String> {
    let registry = FormatHandlerRegistry::with_defaults();
    let eco_name = project.ecosystem.as_str();

    // Bundle / synthetic ecosystems (`tauri`, `hexagonal-cargo`,
    // `jvm-library`) aren't `FormatHandler`-backed — they're
    // taxonomy labels on configured `[release_unit.X]` blocks.
    // For those we fall back to a generic tag template; the unit's
    // own `tag_format` override (set by auto-detect or by the user)
    // covers customisation, the default is a sensible last resort.
    let (eco_default_tag, eco_allowed_vars): (&'static str, &'static [&'static str]) =
        match registry.lookup(eco_name) {
            Some(h) => (h.tag_format_default(), h.tag_template_vars()),
            None => ("{name}@v{version}", &["name", "version", "ecosystem"]),
        };

    // tag-format precedence: explicit [release_unit.<name>] > [group.<id>]
    // > ecosystem default.
    let unit_override = sess
        .resolved_release_units()
        .iter()
        .find(|r| r.unit.name == project.name)
        .and_then(|r| r.unit.tag_format.as_deref());
    let group_override = groups
        .group_of(project.ident)
        .and_then(|g| g.tag_format.as_deref());
    let template = unit_override.or(group_override);

    let maven_coords = if eco_name == "maven" {
        split_maven_coords(&project.name)
    } else {
        None
    };

    let inputs = TagFormatInputs {
        project_name: &project.name,
        version: &project.new_version,
        ecosystem: eco_name,
        ecosystem_default: eco_default_tag,
        allowed_vars: eco_allowed_vars,
        override_template: template,
        maven_coords,
        module_path: if eco_name == "go" {
            Some(&project.name)
        } else {
            None
        },
    };
    format_tag(&inputs)
}

mod changelog_gen;
mod github;

pub use changelog_gen::{
    generate_and_write_project_changelog, generate_changelog_entry, ChangelogGenerationParams,
    ChangelogResult,
};
pub use github::{extract_github_remote, load_github_token, GitHubRemoteInfo};

use github::parse_github_url;
