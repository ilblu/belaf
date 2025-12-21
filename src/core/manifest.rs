//! Release manifest handling for PR-based releases.
//!
//! The release manifest is a JSON file stored in `belaf/releases/` that contains
//! metadata about a pending release. It serves as a contract between the CLI
//! (which creates releases) and the GitHub App (which finalizes them).
//!
//! # Schema version `1.0`
//!
//! - Release metadata (timestamp, author, base branch)
//! - Per-project release info (versions, changelog, tag names)

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use time::OffsetDateTime;
use tracing::warn;
use uuid::Uuid;

const SCHEMA_VERSION: &str = "1.2";
pub const MANIFEST_DIR: &str = "belaf/releases";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseManifest {
    pub schema_version: String,
    pub created_at: String,
    pub created_by: String,
    pub base_branch: String,
    pub releases: Vec<ProjectRelease>,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReleaseStatistics {
    pub commit_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub days_since_last_release: Option<i64>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub breaking_changes_count: usize,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub features_count: usize,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub fixes_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr_count: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectRelease {
    pub name: String,
    pub ecosystem: String,
    pub previous_version: String,
    pub new_version: String,
    pub bump_type: String,
    pub changelog: String,
    pub tag_name: String,
    pub prefix: String,
    #[serde(default)]
    pub is_prerelease: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compare_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contributors: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub first_time_contributors: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub statistics: Option<ReleaseStatistics>,
}

impl ReleaseManifest {
    pub fn new(base_branch: String, created_by: String) -> Self {
        let now = OffsetDateTime::now_utc();
        let format = time::format_description::well_known::Rfc3339;
        let created_at = now.format(&format).unwrap_or_else(|e| {
            warn!("failed to format timestamp as RFC3339: {}", e);
            now.to_string()
        });

        Self {
            schema_version: SCHEMA_VERSION.to_string(),
            created_at,
            created_by,
            base_branch,
            releases: Vec::new(),
        }
    }

    pub fn add_release(&mut self, release: ProjectRelease) {
        self.releases.push(release);
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn save_to_file(&self, path: &Path) -> Result<(), std::io::Error> {
        let json = self
            .to_json()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::write(path, json)
    }

    pub fn generate_filename() -> String {
        let now = OffsetDateTime::now_utc();
        let formatted =
            time::format_description::parse("[year][month][day]-[hour][minute][second]")
                .ok()
                .and_then(|format| now.format(&format).ok())
                .unwrap_or_else(|| now.unix_timestamp().to_string());
        let suffix = &Uuid::new_v4().to_string()[..8];

        format!("release-{}-{}.json", formatted, suffix)
    }
}

impl ProjectRelease {
    pub fn new(
        name: String,
        ecosystem: String,
        previous_version: String,
        new_version: String,
        bump_type: String,
        changelog: String,
        prefix: String,
    ) -> Self {
        let tag_name = if prefix.is_empty() {
            format!("v{new_version}")
        } else {
            format!("{prefix}/v{new_version}")
        };

        let previous_tag = if previous_version.is_empty() {
            None
        } else if prefix.is_empty() {
            Some(format!("v{previous_version}"))
        } else {
            Some(format!("{prefix}/v{previous_version}"))
        };

        let is_prerelease = Self::detect_prerelease(&new_version);

        Self {
            name,
            ecosystem,
            previous_version,
            new_version,
            bump_type,
            changelog,
            tag_name,
            prefix,
            is_prerelease,
            previous_tag,
            compare_url: None,
            contributors: Vec::new(),
            first_time_contributors: Vec::new(),
            statistics: None,
        }
    }

    fn detect_prerelease(version: &str) -> bool {
        let prerelease_markers = ["-alpha", "-beta", "-rc", "-dev", "-pre", "-snapshot"];
        let version_lower = version.to_lowercase();
        prerelease_markers
            .iter()
            .any(|marker| version_lower.contains(marker))
    }

    pub fn with_compare_url(mut self, base_url: &str) -> Self {
        if let Some(prev_tag) = &self.previous_tag {
            self.compare_url = Some(format!(
                "{}/compare/{}...{}",
                base_url.trim_end_matches('/'),
                prev_tag,
                self.tag_name
            ));
        }
        self
    }

    pub fn with_contributors(mut self, contributors: Vec<String>) -> Self {
        self.contributors = contributors;
        self
    }

    pub fn with_first_time_contributors(mut self, contributors: Vec<String>) -> Self {
        self.first_time_contributors = contributors;
        self
    }

    pub fn with_statistics(mut self, statistics: ReleaseStatistics) -> Self {
        self.statistics = Some(statistics);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_new_has_correct_schema_version() {
        let manifest = ReleaseManifest::new("main".to_string(), "test-user".to_string());
        assert_eq!(manifest.schema_version, "1.2");
    }

    #[test]
    fn test_manifest_new_has_rfc3339_timestamp() {
        let manifest = ReleaseManifest::new("main".to_string(), "test-user".to_string());
        assert!(manifest.created_at.contains('T'));
        assert!(manifest.created_at.contains('Z') || manifest.created_at.contains('+'));
    }

    #[test]
    fn test_manifest_serialization_roundtrip() {
        let mut manifest = ReleaseManifest::new("main".to_string(), "github-actions".to_string());
        manifest.add_release(ProjectRelease::new(
            "my-crate".to_string(),
            "cargo".to_string(),
            "1.0.0".to_string(),
            "1.1.0".to_string(),
            "minor".to_string(),
            "## Changes\n- Added feature".to_string(),
            "".to_string(),
        ));

        let json = manifest.to_json().expect("serialization should succeed");
        let deserialized =
            ReleaseManifest::from_json(&json).expect("deserialization should succeed");

        assert_eq!(deserialized.schema_version, manifest.schema_version);
        assert_eq!(deserialized.base_branch, manifest.base_branch);
        assert_eq!(deserialized.created_by, manifest.created_by);
        assert_eq!(deserialized.releases.len(), 1);
        assert_eq!(deserialized.releases[0].name, "my-crate");
        assert_eq!(deserialized.releases[0].new_version, "1.1.0");
    }

    #[test]
    fn test_manifest_json_contains_expected_fields() {
        let mut manifest = ReleaseManifest::new("develop".to_string(), "ci-bot".to_string());
        manifest.add_release(ProjectRelease::new(
            "test-pkg".to_string(),
            "npm".to_string(),
            "2.0.0".to_string(),
            "3.0.0".to_string(),
            "major".to_string(),
            "Breaking changes".to_string(),
            "packages/test".to_string(),
        ));

        let json = manifest.to_json().expect("serialization should succeed");

        assert!(json.contains("\"schema_version\": \"1.2\""));
        assert!(json.contains("\"base_branch\": \"develop\""));
        assert!(json.contains("\"created_by\": \"ci-bot\""));
        assert!(json.contains("\"name\": \"test-pkg\""));
        assert!(json.contains("\"ecosystem\": \"npm\""));
        assert!(json.contains("\"previous_version\": \"2.0.0\""));
        assert!(json.contains("\"new_version\": \"3.0.0\""));
        assert!(json.contains("\"bump_type\": \"major\""));
    }

    #[test]
    fn test_project_release_tag_name_without_prefix() {
        let release = ProjectRelease::new(
            "crate".to_string(),
            "cargo".to_string(),
            "1.0.0".to_string(),
            "1.0.1".to_string(),
            "patch".to_string(),
            "Changelog".to_string(),
            "".to_string(),
        );
        assert_eq!(release.tag_name, "v1.0.1");
    }

    #[test]
    fn test_project_release_tag_name_with_prefix() {
        let release = ProjectRelease::new(
            "core".to_string(),
            "cargo".to_string(),
            "1.0.0".to_string(),
            "2.0.0".to_string(),
            "major".to_string(),
            "Changelog".to_string(),
            "packages/core".to_string(),
        );
        assert_eq!(release.tag_name, "packages/core/v2.0.0");
    }

    #[test]
    fn test_generate_filename_format() {
        let filename = ReleaseManifest::generate_filename();
        assert!(filename.starts_with("release-"));
        assert!(filename.ends_with(".json"));

        let without_ext = filename.strip_suffix(".json").unwrap();
        let parts: Vec<&str> = without_ext.splitn(3, '-').collect();
        assert_eq!(parts.len(), 3, "expected release-TIMESTAMP-UUID format");
        assert_eq!(parts[0], "release");

        let uuid_suffix = parts[2].split('-').next_back().unwrap();
        assert_eq!(uuid_suffix.len(), 8, "UUID suffix should be 8 chars");
        assert!(
            uuid_suffix.chars().all(|c| c.is_ascii_hexdigit()),
            "UUID suffix should be hex"
        );
    }

    #[test]
    fn test_manifest_multiple_releases() {
        let mut manifest = ReleaseManifest::new("main".to_string(), "bot".to_string());

        manifest.add_release(ProjectRelease::new(
            "pkg-a".to_string(),
            "cargo".to_string(),
            "1.0.0".to_string(),
            "1.1.0".to_string(),
            "minor".to_string(),
            "Changelog A".to_string(),
            "".to_string(),
        ));

        manifest.add_release(ProjectRelease::new(
            "pkg-b".to_string(),
            "npm".to_string(),
            "2.0.0".to_string(),
            "2.0.1".to_string(),
            "patch".to_string(),
            "Changelog B".to_string(),
            "packages/b".to_string(),
        ));

        assert_eq!(manifest.releases.len(), 2);
        assert_eq!(manifest.releases[0].name, "pkg-a");
        assert_eq!(manifest.releases[1].name, "pkg-b");
    }

    #[test]
    fn test_manifest_save_and_load_file() {
        let temp_dir = std::env::temp_dir().join("belaf_test_manifest");
        let manifest_path = temp_dir.join("releases").join("test-release.json");

        let mut manifest = ReleaseManifest::new("main".to_string(), "test".to_string());
        manifest.add_release(ProjectRelease::new(
            "test".to_string(),
            "cargo".to_string(),
            "0.1.0".to_string(),
            "0.2.0".to_string(),
            "minor".to_string(),
            "Test changelog".to_string(),
            "".to_string(),
        ));

        manifest
            .save_to_file(&manifest_path)
            .expect("save should succeed");

        let loaded_json = std::fs::read_to_string(&manifest_path).expect("read should succeed");
        let loaded = ReleaseManifest::from_json(&loaded_json).expect("parse should succeed");

        assert_eq!(loaded.releases.len(), 1);
        assert_eq!(loaded.releases[0].name, "test");

        std::fs::remove_dir_all(temp_dir).ok();
    }
}
