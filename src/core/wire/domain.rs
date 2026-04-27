//! Domain model: ergonomic API over the typify-generated wire types.
//!
//! The rest of the CLI talks in terms of [`Manifest`], [`Release`], and
//! [`Group`] from this module. The structs are thin wrappers around the
//! `codegen::*` types but expose plain `String`/`bool`/`Vec<...>` fields
//! and the [`Ecosystem`]/[`BumpType`] discriminated unions instead of the
//! generated newtype wrappers.
//!
//! Conversion: `From<Manifest> for codegen::BelafReleaseManifest` writes the
//! wire format; `TryFrom<codegen::BelafReleaseManifest> for Manifest` reads
//! it. This is where the discriminated-union classification happens
//! (`Ecosystem::classify`, `BumpType::classify`).

use serde_json::{Map, Value};
use time::OffsetDateTime;
use uuid::Uuid;

use super::codegen::{self, BelafReleaseManifest, Release as WireRelease};
use super::known::{BumpType, Ecosystem};

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Manifest {
    pub schema_version: String,
    pub manifest_id: String,
    pub created_at: String,
    pub created_by: String,
    pub base_branch: String,
    pub groups: Vec<Group>,
    pub releases: Vec<Release>,
    pub x: Map<String, Value>,
}

impl Manifest {
    /// Build a fresh empty manifest. `manifest_id` is a UUID v7 string;
    /// `created_at` is the current UTC time as RFC 3339.
    pub fn new(base_branch: String, created_by: String) -> Self {
        let now = OffsetDateTime::now_utc();
        let format = time::format_description::well_known::Rfc3339;
        let created_at = now.format(&format).unwrap_or_else(|_| now.to_string());
        Self {
            schema_version: "2.0".to_string(),
            manifest_id: Uuid::now_v7().to_string(),
            created_at,
            created_by,
            base_branch,
            groups: Vec::new(),
            releases: Vec::new(),
            x: Map::new(),
        }
    }

    /// Filename for on-disk storage. Always `belaf/releases/<manifest_id>.json`.
    pub fn filename(&self) -> String {
        format!("{}.json", self.manifest_id)
    }

    pub fn add_release(&mut self, release: Release) {
        self.releases.push(release);
    }

    pub fn add_group(&mut self, group: Group) {
        self.groups.push(group);
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        let wire: BelafReleaseManifest = self.clone().into();
        serde_json::to_string_pretty(&wire)
    }

    pub fn from_json(json: &str) -> Result<Self, ManifestParseError> {
        let wire: BelafReleaseManifest = serde_json::from_str(json)?;
        Ok(wire.into())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestParseError {
    #[error("failed to parse manifest JSON: {0}")]
    Json(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Group
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Group {
    pub id: String,
    pub members: Vec<String>,
    pub x: Map<String, Value>,
}

// ---------------------------------------------------------------------------
// Release
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Release {
    pub name: String,
    pub ecosystem: Ecosystem,
    pub group_id: Option<String>,
    pub previous_version: String,
    pub new_version: String,
    pub bump_type: BumpType,
    pub tag_name: String,
    pub previous_tag: Option<String>,
    pub compare_url: Option<String>,
    pub is_prerelease: bool,
    pub changelog: String,
    pub contributors: Vec<String>,
    pub first_time_contributors: Vec<String>,
    pub statistics: Option<ReleaseStatistics>,
    pub x: Map<String, Value>,
}

#[derive(Debug, Clone, Default)]
pub struct ReleaseStatistics {
    pub commit_count: u64,
    pub days_since_last_release: Option<i64>,
    pub breaking_changes_count: u64,
    pub features_count: u64,
    pub fixes_count: u64,
    pub pr_count: Option<u64>,
}

// ---------------------------------------------------------------------------
// Wire <-> Domain conversions
// ---------------------------------------------------------------------------

impl From<Manifest> for BelafReleaseManifest {
    fn from(m: Manifest) -> Self {
        BelafReleaseManifest {
            schema_version: m.schema_version,
            manifest_id: m
                .manifest_id
                .parse()
                .expect("manifest_id must be valid UUID v7 (validated upstream)"),
            created_at: m.created_at.parse().expect("created_at must be non-empty"),
            created_by: m.created_by.parse().expect("created_by must be non-empty"),
            base_branch: m
                .base_branch
                .parse()
                .expect("base_branch must be non-empty"),
            groups: m.groups.into_iter().map(Into::into).collect(),
            releases: m.releases.into_iter().map(Into::into).collect(),
            x: m.x,
        }
    }
}

impl From<BelafReleaseManifest> for Manifest {
    fn from(wire: BelafReleaseManifest) -> Self {
        Self {
            schema_version: "2.0".to_string(),
            manifest_id: wire.manifest_id.into(),
            created_at: wire.created_at.into(),
            created_by: wire.created_by.into(),
            base_branch: wire.base_branch.into(),
            groups: wire.groups.into_iter().map(Into::into).collect(),
            releases: wire.releases.into_iter().map(Into::into).collect(),
            x: wire.x,
        }
    }
}

impl From<Group> for codegen::Group {
    fn from(g: Group) -> Self {
        codegen::Group {
            id: g
                .id
                .parse()
                .expect("group.id must match pattern (validated by belaf upstream)"),
            members: g
                .members
                .into_iter()
                .map(|m| m.parse().expect("group member name must be non-empty"))
                .collect(),
            x: g.x,
        }
    }
}

impl From<codegen::Group> for Group {
    fn from(g: codegen::Group) -> Self {
        Self {
            id: g.id.into(),
            members: g.members.into_iter().map(|m| m.into()).collect(),
            x: g.x,
        }
    }
}

impl From<Release> for WireRelease {
    fn from(r: Release) -> Self {
        WireRelease {
            name: r.name.parse().expect("release.name must be non-empty"),
            ecosystem: r
                .ecosystem
                .as_str()
                .parse()
                .expect("ecosystem string must be non-empty"),
            group_id: r
                .group_id
                .map(|g| g.parse().expect("group_id pattern (validated upstream)")),
            previous_version: r.previous_version,
            new_version: r
                .new_version
                .parse()
                .expect("new_version must be non-empty"),
            bump_type: r
                .bump_type
                .as_str()
                .parse()
                .expect("bump_type must be non-empty"),
            tag_name: r.tag_name.parse().expect("tag_name must be non-empty"),
            previous_tag: r.previous_tag,
            compare_url: r.compare_url,
            is_prerelease: r.is_prerelease,
            changelog: r.changelog,
            contributors: r.contributors,
            first_time_contributors: r.first_time_contributors,
            statistics: r.statistics.map(Into::into),
            x: r.x,
        }
    }
}

impl From<WireRelease> for Release {
    fn from(r: WireRelease) -> Self {
        Self {
            name: r.name.into(),
            ecosystem: Ecosystem::classify(&String::from(r.ecosystem)),
            group_id: r.group_id.map(|g| g.into()),
            previous_version: r.previous_version,
            new_version: r.new_version.into(),
            bump_type: BumpType::classify(&String::from(r.bump_type)),
            tag_name: r.tag_name.into(),
            previous_tag: r.previous_tag,
            compare_url: r.compare_url,
            is_prerelease: r.is_prerelease,
            changelog: r.changelog,
            contributors: r.contributors,
            first_time_contributors: r.first_time_contributors,
            statistics: r.statistics.map(Into::into),
            x: r.x,
        }
    }
}

impl From<ReleaseStatistics> for codegen::ReleaseStatistics {
    fn from(s: ReleaseStatistics) -> Self {
        codegen::ReleaseStatistics {
            commit_count: Some(s.commit_count),
            days_since_last_release: s.days_since_last_release,
            breaking_changes_count: s.breaking_changes_count,
            features_count: s.features_count,
            fixes_count: s.fixes_count,
            pr_count: s.pr_count.and_then(|n| i64::try_from(n).ok()),
            x: Map::new(),
        }
    }
}

impl From<codegen::ReleaseStatistics> for ReleaseStatistics {
    fn from(s: codegen::ReleaseStatistics) -> Self {
        Self {
            commit_count: s.commit_count.unwrap_or(0),
            days_since_last_release: s.days_since_last_release,
            breaking_changes_count: s.breaking_changes_count,
            features_count: s.features_count,
            fixes_count: s.fixes_count,
            pr_count: s.pr_count.map(|n| n.max(0) as u64),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_filename_uses_manifest_id() {
        let m = Manifest::new("main".to_string(), "alice".to_string());
        assert!(m.filename().ends_with(".json"));
        assert!(m.filename().starts_with(&m.manifest_id));
    }

    #[test]
    fn manifest_id_is_uuid_v7_format() {
        let m = Manifest::new("main".to_string(), "alice".to_string());
        // UUID v7: third group starts with 7
        let parts: Vec<&str> = m.manifest_id.split('-').collect();
        assert_eq!(parts.len(), 5);
        assert!(parts[2].starts_with('7'));
    }

    #[test]
    fn manifest_roundtrip_minimal() {
        let mut m = Manifest::new("main".to_string(), "alice".to_string());
        m.add_release(Release {
            name: "@org/foo".to_string(),
            ecosystem: Ecosystem::classify("npm"),
            group_id: None,
            previous_version: "0.1.0".to_string(),
            new_version: "0.2.0".to_string(),
            bump_type: BumpType::classify("minor"),
            tag_name: "@org/foo@v0.2.0".to_string(),
            previous_tag: Some("@org/foo@v0.1.0".to_string()),
            compare_url: None,
            is_prerelease: false,
            changelog: String::new(),
            contributors: vec![],
            first_time_contributors: vec![],
            statistics: None,
            x: Map::new(),
        });
        let json = m.to_json().expect("serialise");
        let m2 = Manifest::from_json(&json).expect("deserialise");
        assert_eq!(m2.releases.len(), 1);
        assert_eq!(m2.releases[0].name, "@org/foo");
        assert_eq!(m2.releases[0].ecosystem.as_str(), "npm");
    }

    #[test]
    fn unknown_ecosystem_survives_roundtrip() {
        let mut m = Manifest::new("main".to_string(), "alice".to_string());
        m.add_release(Release {
            name: "weird-thing".to_string(),
            ecosystem: Ecosystem::Unknown("gradle".to_string()),
            group_id: None,
            previous_version: "0.1.0".to_string(),
            new_version: "0.2.0".to_string(),
            bump_type: BumpType::classify("minor"),
            tag_name: "weird-thing@v0.2.0".to_string(),
            previous_tag: None,
            compare_url: None,
            is_prerelease: false,
            changelog: String::new(),
            contributors: vec![],
            first_time_contributors: vec![],
            statistics: None,
            x: Map::new(),
        });
        let json = m.to_json().expect("serialise");
        assert!(json.contains("\"gradle\""));
        let m2 = Manifest::from_json(&json).expect("deserialise");
        assert_eq!(m2.releases[0].ecosystem.as_str(), "gradle");
    }
}
