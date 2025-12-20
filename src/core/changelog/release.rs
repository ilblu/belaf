use std::collections::HashMap;

use next_version::VersionUpdater;
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::value::Value;

use super::commit::{commits_to_conventional_commits, Commit, Range};
use super::contributor::RemoteContributor;
use super::error::Result;
use super::remote::{RemoteCommit, RemotePullRequest, RemoteReleaseMetadata};
use super::statistics::Statistics;
use crate::core::release::bump::BumpConfig;

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all(serialize = "camelCase"))]
pub struct Release<'a> {
    pub version: Option<String>,
    pub message: Option<String>,
    #[serde(deserialize_with = "commits_to_conventional_commits")]
    pub commits: Vec<Commit<'a>>,
    #[serde(rename = "commit_id")]
    pub commit_id: Option<String>,
    pub timestamp: Option<i64>,
    pub previous: Option<Box<Release<'a>>>,
    pub repository: Option<String>,
    #[serde(rename = "commit_range")]
    pub commit_range: Option<Range>,
    #[serde(rename = "submodule_commits")]
    pub submodule_commits: HashMap<String, Vec<Commit<'a>>>,
    pub statistics: Option<Statistics>,
    pub extra: Option<Value>,
    pub github: RemoteReleaseMetadata,
}

impl Release<'_> {
    pub fn with_statistics(mut self) -> Self {
        self.statistics = Some((&self).into());
        self
    }

    pub(super) fn calculate_next_version_with_config(&self, config: &BumpConfig) -> Result<String> {
        match self
            .previous
            .as_ref()
            .and_then(|release| release.version.clone())
        {
            Some(version) => {
                let mut semver = Version::parse(&version);
                let mut prefix = None;
                if semver.is_err() && version.split('.').count() >= 2 {
                    let mut found_numeric = false;
                    for (i, c) in version.char_indices() {
                        if c.is_numeric() && !found_numeric {
                            found_numeric = true;
                            let version_prefix = version[..i].to_string();
                            let remaining = version[i..].to_string();
                            let version = Version::parse(&remaining);
                            if version.is_ok() {
                                semver = version;
                                prefix = Some(version_prefix);
                                break;
                            }
                        } else if !c.is_numeric() && found_numeric {
                            found_numeric = false;
                        }
                    }
                }

                let next = VersionUpdater::new()
                    .with_features_always_increment_minor(config.features_always_bump_minor)
                    .with_breaking_always_increment_major(config.breaking_always_bump_major);

                let next_version = next
                    .increment(
                        &semver?,
                        self.commits
                            .iter()
                            .map(|commit| commit.message.trim_end().to_string())
                            .collect::<Vec<String>>(),
                    )
                    .to_string();

                if let Some(prefix) = prefix {
                    Ok(format!("{prefix}{next_version}"))
                } else {
                    Ok(next_version)
                }
            }
            None => Ok(config.initial_tag.clone()),
        }
    }

    pub fn update_github_metadata(
        &mut self,
        mut commits: Vec<Box<dyn RemoteCommit>>,
        pull_requests: Vec<Box<dyn RemotePullRequest>>,
    ) -> Result<()> {
        let mut contributors: Vec<RemoteContributor> = Vec::new();
        let mut release_commit_timestamp: Option<i64> = None;

        commits.retain(|v| {
            if let Some(commit) = self.commits.iter_mut().find(|commit| commit.id == v.id()) {
                let sha_short: Option<String> = Some(v.id().chars().take(12).collect());
                let pull_request = pull_requests.iter().find(|pr| {
                    pr.merge_commit() == Some(v.id()) || pr.merge_commit() == sha_short
                });

                let remote_contributor = RemoteContributor {
                    username: v.username(),
                    pr_number: pull_request.map(|v| v.number()),
                    pr_title: pull_request.and_then(|v| v.title()),
                    pr_labels: pull_request.map(|v| v.labels()).unwrap_or_default(),
                    is_first_time: false,
                };

                commit.remote = Some(remote_contributor.clone());

                if !contributors
                    .iter()
                    .any(|c| c.username == remote_contributor.username)
                {
                    contributors.push(remote_contributor);
                }

                if Some(v.id()) == self.commit_id {
                    release_commit_timestamp = v.timestamp();
                }
                false
            } else {
                true
            }
        });

        self.github.contributors = contributors
            .into_iter()
            .map(|mut v| {
                v.is_first_time = !commits
                    .iter()
                    .filter(|commit| {
                        self.timestamp.is_none() || commit.timestamp() < release_commit_timestamp
                    })
                    .any(|commit| commit.username() == v.username);
                v
            })
            .collect();

        Ok(())
    }
}

#[derive(Serialize)]
pub struct Releases<'a> {
    pub releases: &'a Vec<Release<'a>>,
}

impl Releases<'_> {
    pub fn as_json(&self) -> Result<String> {
        Ok(serde_json::to_string(self.releases)?)
    }
}
