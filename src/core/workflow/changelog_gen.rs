//! Changelog generation helpers for the release pipeline.
//!
//! Extracted from `core::workflow` so the orchestrator only deals with the
//! pipeline itself; everything that constructs and writes per-project
//! changelog entries lives here.

use anyhow::{Context, Result};
use tracing::info;

use crate::core::{
    bump::BumpConfig,
    changelog::{Changelog, ChangelogConfig, Commit, GitConfig, Release},
    git::repository::{RepoPathBuf, Repository},
};

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
