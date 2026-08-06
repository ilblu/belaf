//! Release tag lookup and creation.
//!
//! Finding the newest (or newest stable) tag matching a project's tag-format
//! template, parsing a semver out of a tag name, and managing the
//! `belaf-baseline` tag that anchors a repo with no releases yet.

use anyhow::Context;
use tracing::{info, warn};

use crate::core::{errors::Result, git::repository::Repository, tag_format::TagMatcher};

impl Repository {
    /// Find the latest release tag for a project.
    ///
    /// For single-project repos (`is_single_project = true`), matches both:
    /// - Plain version tags: `v1.2.3`
    /// - Prefixed tags: `project-name-v1.2.3`
    ///
    /// For multi-project repos, only matches prefixed tags to avoid ambiguity.
    ///
    /// Returns the commit OID, tag name, and parsed version of the
    /// latest matching tag, sorted by semantic version (highest first).
    ///
    /// The caller passes a pre-compiled [`TagMatcher`] built from the
    /// project's effective tag-format template (unit override > group
    /// override > ecosystem default). See
    /// [`crate::core::tag_format::build_tag_matcher`].
    ///
    /// Pre-fix history note: this used to take `(project_name,
    /// is_single_project)` and hard-coded `{name}-v{version}` (cargo) +
    /// bare-`v{version}` (single-project). Every ecosystem with a
    /// different default — npm, maven, pypa, go — silently missed its
    /// own tags and triggered the "walk every commit since repo start"
    /// fallback in [`Repository::analyze_histories`], inflating bumps.
    pub fn find_latest_tag_for_project(
        &self,
        matcher: &TagMatcher,
    ) -> Result<Option<(git2::Oid, String, semver::Version)>> {
        let tags = self.repo.tag_names(None)?;

        let mut matching_tags: Vec<(git2::Oid, String, semver::Version)> = Vec::new();

        for tag_name in tags.iter().flatten() {
            let Some(version) = matcher.match_version(tag_name) else {
                continue;
            };
            let Ok(tag_ref) = self.repo.find_reference(&format!("refs/tags/{}", tag_name)) else {
                continue;
            };
            let oid = if let Some(target_oid) = tag_ref.target() {
                target_oid
            } else if let Ok(tag_obj) = tag_ref.peel_to_tag() {
                tag_obj.target_id()
            } else {
                continue;
            };
            matching_tags.push((oid, tag_name.to_string(), version));
        }

        if matching_tags.is_empty() {
            return Ok(None);
        }

        matching_tags.sort_by(|a, b| b.2.cmp(&a.2));
        Ok(matching_tags.into_iter().next())
    }

    /// Like [`Self::find_latest_tag_for_project`] but ignores prerelease tags
    /// (F11b). The base level for a prerelease unit must be computed from
    /// commits since the last **stable** release, not the last prerelease —
    /// otherwise accumulated changes across betas get under-counted.
    pub fn find_latest_stable_tag_for_project(
        &self,
        matcher: &TagMatcher,
    ) -> Result<Option<(git2::Oid, String, semver::Version)>> {
        let tags = self.repo.tag_names(None)?;
        let mut matching: Vec<(git2::Oid, String, semver::Version)> = Vec::new();
        for tag_name in tags.iter().flatten() {
            let Some(version) = matcher.match_version(tag_name) else {
                continue;
            };
            if !version.pre.is_empty() {
                continue; // skip prereleases
            }
            let Ok(tag_ref) = self.repo.find_reference(&format!("refs/tags/{}", tag_name)) else {
                continue;
            };
            let oid = if let Some(t) = tag_ref.target() {
                t
            } else if let Ok(tag_obj) = tag_ref.peel_to_tag() {
                tag_obj.target_id()
            } else {
                continue;
            };
            matching.push((oid, tag_name.to_string(), version));
        }
        matching.sort_by(|a, b| b.2.cmp(&a.2));
        Ok(matching.into_iter().next())
    }

    /// Parse a semantic version from a tag name.
    ///
    /// Supports two formats:
    /// - Plain version tags: `v1.2.3` → `1.2.3`
    /// - Prefixed tags: `project-name-v1.2.3` → `1.2.3`
    ///
    /// Returns `0.0.0` if the tag cannot be parsed as a valid semver.
    pub fn parse_version_from_tag(tag_name: &str) -> semver::Version {
        if let Some(version_str) = tag_name.strip_prefix('v') {
            if let Ok(version) = semver::Version::parse(version_str) {
                return version;
            }
        }
        if let Some(version_str) = tag_name.rsplit("-v").next() {
            if let Ok(version) = semver::Version::parse(version_str) {
                return version;
            }
        }
        semver::Version::new(0, 0, 0)
    }

    /// `true` if the repo has **any** tag whose name looks like a version
    /// tag (`v1.2.3`, `name-v1.2.3`, `name@v1.2.3`, `name/v1.2.3`,
    /// `name-1.2.3`). Used by [`Self::analyze_histories`] to decide
    /// whether a tag-lookup miss is "first release ever" (no version
    /// tags at all) vs. "the configured template doesn't match the
    /// existing tags" (the bug class). Uses a permissive regex by
    /// design — false positives just relax the safety net.
    pub(super) fn repo_has_any_version_tags(&self) -> Result<bool> {
        let tags = self.repo.tag_names(None)?;
        let re = regex::Regex::new(r"\d+\.\d+\.\d+").expect("BUG: literal regex compiles");
        for tag_name in tags.iter().flatten() {
            if re.is_match(tag_name) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(super) fn find_baseline_tag(&self) -> Result<Option<git2::Oid>> {
        match self.repo.find_reference("refs/tags/belaf-baseline") {
            Ok(tag_ref) => {
                if let Some(target_oid) = tag_ref.target() {
                    Ok(Some(target_oid))
                } else if let Ok(tag_obj) = tag_ref.peel_to_tag() {
                    Ok(Some(tag_obj.target_id()))
                } else {
                    Ok(None)
                }
            }
            Err(_) => Ok(None),
        }
    }

    pub fn tag_exists(&self, tag_name: &str) -> bool {
        let refs_tag = format!("refs/tags/{}", tag_name);
        self.repo.find_reference(&refs_tag).is_ok()
    }

    pub fn create_baseline_tag(&self) -> Result<()> {
        let head = self.repo.head()?;
        let target_oid = head.target().context("HEAD has no target")?;

        match self.repo.tag_lightweight(
            "belaf-baseline",
            &self.repo.find_object(target_oid, None)?,
            false,
        ) {
            Ok(_) => {
                info!("created baseline tag 'belaf-baseline' at HEAD");
                Ok(())
            }
            Err(e) if e.code() == git2::ErrorCode::Exists => {
                warn!("baseline tag 'belaf-baseline' already exists, not creating");
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }
}
