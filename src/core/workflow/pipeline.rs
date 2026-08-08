//! The [`ReleasePipeline`] — executes an approved release selection.
//!
//! Bumps versions, generates changelogs, writes the release manifest,
//! commits and pushes the release branch, and opens the pull request that
//! the belaf GitHub App finalizes on merge.

use anyhow::{Context, Result};
use std::collections::HashMap;
use tracing::{debug, info, warn};

use crate::core::{
    bump::BumpConfig,
    changelog::{ChangelogConfig, Commit, GitConfig},
    git::repository::{ChangeList, RepoPathBuf},
    github::{
        client::{GitHubInformation, PrAction},
        pr,
    },
    manifest::{ReleaseEntry, ReleaseManifest, ReleaseStatistics, MANIFEST_DIR},
    session::AppSession,
};

use super::branch::format_commit_message;
use super::changelog_gen::{generate_and_write_project_changelog, ChangelogGenerationParams};
use super::{
    build_tag_name, extract_github_remote, load_github_token, ChangelogGenerationResult,
    PreparedRelease, SelectedReleaseUnit,
};

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

    pub fn execute(mut self, projects: Vec<SelectedReleaseUnit>) -> Result<PreparedRelease> {
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

        let removed_manifests = self.remove_processed_manifests(&manifest_repo_path)?;

        info!("creating release commit...");
        let all_changed_paths = self.collect_all_paths(
            &changes,
            &changelog_paths,
            &manifest_repo_path,
            &removed_manifests,
        );
        self.create_commit(&projects, &all_changed_paths)?;

        info!("pushing release branch to remote...");
        self.push_branch()?;

        info!("opening or updating pull request...");
        let (pr_url, pr_action) =
            self.create_pull_request(&projects, &manifest_filename, &changelog_contents)?;

        self.print_summary(&projects, &pr_url, pr_action);

        Ok(PreparedRelease { pr_url, pr_action })
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
            .with_prerelease(project.is_prerelease)
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

            // `[cascade_inputs]` provenance. Emitted on the *dependent's*
            // release entry, never on the input node: input nodes are
            // `UnitKind::Internal` and never produce a release.
            if !project.cascade_inputs.is_empty() {
                release = release.with_cascade_inputs(project.cascade_inputs.clone());
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
        removed_manifests: &'b [RepoPathBuf],
    ) -> Vec<&'b crate::core::git::repository::RepoPath> {
        changes
            .paths()
            .chain(changelog_paths.iter().map(|p| p.as_ref()))
            .chain(std::iter::once(manifest_repo_path.as_ref()))
            .chain(removed_manifests.iter().map(|p| p.as_ref()))
            .collect()
    }

    /// Delete the manifests whose releases have already been tagged, so they
    /// travel out in this release's pull request.
    ///
    /// The github-app tries this first with a direct commit and cannot manage
    /// it on a protected branch — it is not a bypass actor, and making it one
    /// would defeat the protection to solve housekeeping. Doing it here instead
    /// goes through the same reviewed pull request as everything else `prepare`
    /// writes, and restores the directory's meaning: a manifest still present
    /// is one still waiting to be released.
    ///
    /// Best-effort by construction — see `manifest::cleanup` for why the check
    /// can only err towards leaving files in place.
    fn remove_processed_manifests(&self, fresh_manifest: &RepoPathBuf) -> Result<Vec<RepoPathBuf>> {
        let processed = crate::core::manifest::cleanup::find_processed_manifests(
            &self.sess.repo,
            Some(fresh_manifest),
        );

        let mut removed = Vec::new();
        for entry in processed {
            let abs = self.sess.repo.resolve_workdir(entry.path.as_ref());
            match std::fs::remove_file(&abs) {
                Ok(()) => {
                    info!(
                        "removed released manifest {} ({})",
                        entry.path.escaped(),
                        entry.tags.join(", ")
                    );
                    removed.push(entry.path);
                }
                // Already gone — the app managed the cleanup itself. Nothing
                // to stage, nothing to report.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    warn!(
                        "could not remove released manifest {}: {e}",
                        entry.path.escaped()
                    );
                }
            }
        }
        Ok(removed)
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
        // Force: the release branch is rebuilt from the base commit on every
        // run, so its remote counterpart is never a fast-forward ancestor.
        // The branch already carries the release commit at this point — see
        // the invariant on `Repository::push_branch`.
        self.sess
            .repo
            .push_branch(&self.release_branch, Some(&git_token), true)
            .with_context(|| {
                format!(
                    "failed to push release branch `{}`. If branch protection forbids \
                     force-pushes on that ref, point `[repo] release_branch` in \
                     belaf/config.toml at an unprotected name.",
                    self.release_branch
                )
            })?;
        Ok(())
    }

    fn fetch_git_credentials(&self) -> Result<String> {
        crate::core::github::client::fetch_git_credentials(&self.sess.repo)
    }

    fn create_pull_request(
        &self,
        projects: &[SelectedReleaseUnit],
        manifest_filename: &str,
        changelog_contents: &HashMap<String, String>,
    ) -> Result<(String, PrAction)> {
        let github =
            GitHubInformation::new(self.sess).context("failed to initialize GitHub client")?;

        let pr_title = pr::generate_pr_title(projects);
        let pr_body = pr::generate_pr_body(projects, manifest_filename, changelog_contents);

        github
            .create_or_update_pull_request(
                &self.release_branch,
                &self.base_branch,
                &pr_title,
                &pr_body,
            )
            .context("failed to open or update pull request")
    }

    fn print_summary(&self, projects: &[SelectedReleaseUnit], pr_url: &str, action: PrAction) {
        info!(
            "prepared {} project{} for release",
            projects.len(),
            if projects.len() == 1 { "" } else { "s" }
        );
        info!("pull request {}: {}", action.as_str(), pr_url);
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
