use std::collections::HashMap;
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use secrecy::SecretString;
use serde::Serialize;

use super::commit::Commit;

const SHORT_SHA_LENGTH: usize = 7;
use super::config::{ChangelogConfig, GitConfig};
use super::error::{Error, Result};
use super::github::GitHubClient;
use super::release::{Release, Releases};
use super::remote::RemoteMetadata;
use super::template::Template;
use crate::core::bump::BumpConfig;

#[derive(Debug, Clone, Serialize)]
pub struct RemoteConfig {
    pub owner: String,
    pub repo: String,
}

#[derive(Debug)]
pub struct Changelog {
    pub releases: Vec<Release>,
    pub git_config: GitConfig,
    pub changelog_config: ChangelogConfig,
    pub bump_config: BumpConfig,
    header_template: Option<Template>,
    body_template: Template,
    footer_template: Option<Template>,
    additional_context: HashMap<String, serde_json::Value>,
    remote: Option<RemoteConfig>,
    github_token: Option<SecretString>,
}

impl Changelog {
    pub fn new(
        releases: Vec<Release>,
        git_config: GitConfig,
        changelog_config: ChangelogConfig,
        bump_config: BumpConfig,
    ) -> Result<Self> {
        let trim = changelog_config.trim;
        let mut additional_context = HashMap::new();

        additional_context.insert(
            "emoji_groups".to_string(),
            serde_json::to_value(changelog_config.emoji_groups)?,
        );
        additional_context.insert(
            "group_emojis".to_string(),
            serde_json::to_value(&changelog_config.group_emojis)?,
        );
        additional_context.insert(
            "include_breaking_section".to_string(),
            serde_json::to_value(changelog_config.include_breaking_section)?,
        );
        additional_context.insert(
            "include_contributors".to_string(),
            serde_json::to_value(changelog_config.include_contributors)?,
        );
        additional_context.insert(
            "include_statistics".to_string(),
            serde_json::to_value(changelog_config.include_statistics)?,
        );

        Ok(Self {
            releases,
            header_template: match &changelog_config.header {
                Some(header) => Some(Template::new("header", header.to_string(), trim)?),
                None => None,
            },
            body_template: Template::new("body", changelog_config.body.clone(), trim)?,
            footer_template: match &changelog_config.footer {
                Some(footer) => Some(Template::new("footer", footer.to_string(), trim)?),
                None => None,
            },
            git_config,
            changelog_config,
            bump_config,
            additional_context,
            remote: None,
            github_token: None,
        })
    }

    pub fn with_remote(mut self, owner: String, repo: String) -> Self {
        let remote = RemoteConfig {
            owner: owner.clone(),
            repo: repo.clone(),
        };
        let repository_url = format!("https://github.com/{}/{}", owner, repo);

        for release in &mut self.releases {
            release.repository = Some(repository_url.clone());
        }

        if let Ok(value) = serde_json::to_value(&remote) {
            self.additional_context
                .insert("remote".to_string(), value);
        }

        self.remote = Some(remote);
        self
    }

    pub fn with_github_token(mut self, token: SecretString) -> Self {
        self.github_token = Some(token);
        self
    }

    pub fn add_context(
        &mut self,
        key: impl Into<String>,
        value: impl serde::Serialize,
    ) -> Result<()> {
        self.additional_context
            .insert(key.into(), serde_json::to_value(value)?);
        Ok(())
    }

    fn process_commit(commit: &Commit, git_config: &GitConfig) -> Option<Commit> {
        match commit.process(git_config) {
            Ok(commit) => Some(commit),
            Err(e) => {
                let short_id = commit.id.chars().take(SHORT_SHA_LENGTH).collect::<String>();
                let summary = commit.message.lines().next().unwrap_or_default().trim();
                match &e {
                    Error::ParseError(_) | Error::FieldError(_) => {
                        log::warn!("{short_id} - {e} ({summary})");
                    }
                    _ => {
                        log::trace!("{short_id} - {e} ({summary})");
                    }
                }
                None
            }
        }
    }

    fn process_commit_list(commits: &mut Vec<Commit>, git_config: &GitConfig) -> Result<()> {
        *commits = commits
            .iter()
            .filter_map(|commit| Self::process_commit(commit, git_config))
            .flat_map(|commit| {
                if git_config.split_commits {
                    commit
                        .message
                        .lines()
                        .filter_map(|line| {
                            let mut c = commit.clone();
                            c.message = line.to_string();
                            c.links.clear();
                            if c.message.is_empty() {
                                None
                            } else {
                                Self::process_commit(&c, git_config)
                            }
                        })
                        .collect()
                } else {
                    vec![commit]
                }
            })
            .collect::<Vec<Commit>>();

        if git_config.require_conventional {
            let unconventional_count = commits.iter().filter(|c| c.conv.is_none()).count();
            if unconventional_count > 0 {
                for commit in commits.iter().filter(|c| c.conv.is_none()) {
                    log::error!(
                        "Commit {} is not conventional:\n{}",
                        &commit.id[..SHORT_SHA_LENGTH.min(commit.id.len())],
                        commit
                            .message
                            .lines()
                            .map(|line| format!("    | {}", line.trim()))
                            .collect::<Vec<String>>()
                            .join("\n")
                    );
                }
                return Err(Error::ChangelogError(format!(
                    "{} unconventional commit(s) found",
                    unconventional_count
                )));
            }
        }

        if git_config.fail_on_unmatched_commit {
            let unmatched_count = commits.iter().filter(|c| c.group.is_none()).count();
            if unmatched_count > 0 {
                for commit in commits.iter().filter(|c| c.group.is_none()) {
                    log::error!(
                        "Commit {} was not matched by any commit parser:\n{}",
                        &commit.id[..SHORT_SHA_LENGTH.min(commit.id.len())],
                        commit
                            .message
                            .lines()
                            .map(|line| format!("    | {}", line.trim()))
                            .collect::<Vec<String>>()
                            .join("\n")
                    );
                }
                return Err(Error::ChangelogError(format!(
                    "{} unmatched commit(s) found",
                    unmatched_count
                )));
            }
        }

        if git_config.topo_order_commits {
            log::trace!("Sorting the commits topologically");
        } else {
            match git_config.sort_commits.to_lowercase().as_str() {
                "oldest" => {
                    log::trace!("Sorting the commits from oldest to newest");
                    commits.sort_by(|a, b| a.committer.timestamp.cmp(&b.committer.timestamp));
                }
                "newest" => {
                    log::trace!("Sorting the commits from newest to oldest");
                    commits.sort_by(|a, b| b.committer.timestamp.cmp(&a.committer.timestamp));
                }
                _ => {
                    log::warn!(
                        "Unknown sort_commits value: '{}', using default 'oldest'",
                        git_config.sort_commits
                    );
                    commits.sort_by(|a, b| a.committer.timestamp.cmp(&b.committer.timestamp));
                }
            }
        }

        if let Some(limit) = git_config.limit_commits {
            commits.truncate(limit);
        }

        Ok(())
    }

    pub fn process_commits(&mut self) -> Result<()> {
        log::debug!("Processing the commits");
        for release in self.releases.iter_mut() {
            Self::process_commit_list(&mut release.commits, &self.git_config)?;
            for submodule_commits in release.submodule_commits.values_mut() {
                Self::process_commit_list(submodule_commits, &self.git_config)?;
            }
        }
        Ok(())
    }

    pub fn process_releases(&mut self) {
        log::debug!("Processing {} release(s)", self.releases.len());
        let skip_regex = self.git_config.skip_tags.as_ref();
        let mut skipped_tags = Vec::new();
        self.releases = std::mem::take(&mut self.releases)
            .into_iter()
            .rev()
            .filter(|release| {
                if let Some(version) = &release.version {
                    if skip_regex.is_some_and(|r| r.is_match(version)) {
                        skipped_tags.push(version.clone());
                        log::debug!("Skipping release: {}", version);
                        return false;
                    }
                }
                if release.commits.is_empty() {
                    if let Some(version) = release.version.clone() {
                        log::debug!("Release doesn't have any commits: {}", version);
                    }
                    match &release.previous {
                        Some(prev_release) if prev_release.commits.is_empty() => {
                            return self.changelog_config.render_always;
                        }
                        _ => return false,
                    }
                }
                true
            })
            .map(|release| release.with_statistics())
            .collect();

        for skipped_tag in &skipped_tags {
            if let Some(release_index) = self.releases.iter().position(|release| {
                release
                    .previous
                    .as_ref()
                    .and_then(|release| release.version.as_ref())
                    == Some(skipped_tag)
            }) {
                if let Some(previous_release) = self.releases.get_mut(release_index + 1) {
                    previous_release.previous = None;
                    self.releases[release_index].previous =
                        Some(Box::new(previous_release.clone()));
                } else if release_index == self.releases.len() - 1 {
                    self.releases[release_index].previous = None;
                }
            }
        }
    }

    async fn get_github_metadata(&self, ref_name: Option<&str>) -> Result<RemoteMetadata> {
        let remote = self
            .remote
            .as_ref()
            .ok_or_else(|| Error::RemoteNotConfigured)?;

        let github_client = GitHubClient::new(
            remote.owner.clone(),
            remote.repo.clone(),
            self.github_token.clone(),
        )?;

        log::info!(
            "Retrieving data from GitHub ({}/{})...",
            remote.owner,
            remote.repo
        );

        let (commits, pull_requests) = tokio::try_join!(
            github_client.get_commits(ref_name),
            github_client.get_pull_requests(),
        )?;

        log::debug!("Number of GitHub commits: {}", commits.len());
        log::debug!("Number of GitHub pull requests: {}", pull_requests.len());
        log::info!("Done fetching GitHub data.");

        Ok((commits, pull_requests))
    }

    pub async fn add_github_metadata(&mut self, ref_name: Option<&str>) -> Result<()> {
        if self.remote.is_none() {
            log::debug!("Remote not configured, skipping GitHub metadata");
            return Ok(());
        }

        if let Some(remote) = &self.remote {
            self.additional_context
                .insert("remote".to_string(), serde_json::to_value(remote)?);
        }

        let (github_commits, github_pull_requests) = self.get_github_metadata(ref_name).await?;

        for release in &mut self.releases {
            release.update_github_metadata(github_commits.clone(), github_pull_requests.clone())?;
        }

        Ok(())
    }

    pub fn add_github_metadata_sync(&mut self, ref_name: Option<&str>) -> Result<()> {
        if self.remote.is_none() || self.github_token.is_none() {
            log::debug!("Remote or GitHub token not configured, skipping GitHub metadata");
            return Ok(());
        }

        let future = self.add_github_metadata(ref_name);

        let result = match tokio::runtime::Handle::try_current() {
            Ok(handle) => tokio::task::block_in_place(|| handle.block_on(future)),
            Err(_) => {
                let rt = match tokio::runtime::Runtime::new() {
                    Ok(rt) => rt,
                    Err(e) => {
                        log::warn!("Failed to create tokio runtime for GitHub metadata: {}", e);
                        return Ok(());
                    }
                };
                rt.block_on(future)
            }
        };

        if let Err(e) = result {
            log::warn!(
                "Failed to fetch GitHub metadata (continuing without it): {}",
                e
            );
        }

        Ok(())
    }

    pub fn bump_version(&mut self) -> Result<Option<String>> {
        if let Some(ref mut last_release) = self.releases.iter_mut().next() {
            if last_release.version.is_none() {
                let next_version =
                    last_release.calculate_next_version_with_config(&self.bump_config)?;
                log::debug!("Bumping the version to {next_version}");
                last_release.version = Some(next_version.to_string());
                last_release.timestamp = Some(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_err(|e| Error::ChangelogError(e.to_string()))?
                        .as_secs() as i64,
                );
                return Ok(Some(next_version));
            }
        }
        Ok(None)
    }

    pub fn generate<W: Write + ?Sized>(&self, out: &mut W) -> Result<()> {
        log::debug!("Generating changelog");
        let postprocessors = self.changelog_config.postprocessors.clone();

        if let Some(header_template) = &self.header_template {
            let write_result = writeln!(
                out,
                "{}",
                header_template.render(
                    &Releases {
                        releases: &self.releases,
                    },
                    Some(&self.additional_context),
                    &postprocessors,
                )?
            );
            if let Err(e) = write_result {
                if e.kind() != std::io::ErrorKind::BrokenPipe {
                    return Err(e.into());
                }
            }
        }

        for release in &self.releases {
            let write_result = write!(
                out,
                "{}",
                self.body_template.render(
                    release,
                    Some(&self.additional_context),
                    &postprocessors
                )?
            );
            if let Err(e) = write_result {
                if e.kind() != std::io::ErrorKind::BrokenPipe {
                    return Err(e.into());
                }
            }
        }

        if let Some(footer_template) = &self.footer_template {
            let write_result = writeln!(
                out,
                "{}",
                footer_template.render(
                    &Releases {
                        releases: &self.releases,
                    },
                    Some(&self.additional_context),
                    &postprocessors,
                )?
            );
            if let Err(e) = write_result {
                if e.kind() != std::io::ErrorKind::BrokenPipe {
                    return Err(e.into());
                }
            }
        }

        Ok(())
    }

    pub fn prepend<W: Write + ?Sized>(&self, mut changelog: String, out: &mut W) -> Result<()> {
        log::debug!("Generating changelog and prepending");
        if let Some(header) = &self.changelog_config.header {
            changelog = changelog.replacen(header, "", 1);
        }
        self.generate(out)?;
        write!(out, "{changelog}")?;
        Ok(())
    }

    pub fn write_context<W: Write + ?Sized>(&self, out: &mut W) -> Result<()> {
        let output = Releases {
            releases: &self.releases,
        }
        .as_json()?;
        writeln!(out, "{output}")?;
        Ok(())
    }
}
