//! Release availability and release-commit bookkeeping.
//!
//! [`ReleaseAvailability`] answers "is this commit already released?",
//! [`ReleaseCommitInfo`] / [`ReleasedProjectInfo`] record the per-project state
//! captured at a release commit, and [`ChangeList`] accumulates the paths
//! touched while rewriting manifests.

use serde::{Deserialize, Serialize};

use crate::core::{
    errors::Result,
    git::repository::{CommitId, RepoPath, RepoPathBuf, Repository},
    resolved_release_unit::ResolvedReleaseUnit,
    tag_format::TagMatcher,
    version::Version,
};

/// Describes the availability of a given commit in the release of a project.
/// Note that because different projects are released at different times, the
/// availability for the same commit might vary depending on which project we're
/// considering.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReleaseAvailability {
    /// The commit has already been released. The earliest release containing it
    /// has the given version.
    ExistingRelease(Version),

    /// The commit has not been released, but is an ancestor of HEAD, so it
    /// would be available if a new release of the target project were to be
    /// created. We need to pay attention to this case to allow people to stage
    /// and release multiple projects in one batch.
    NewRelease,

    /// Neither of the above applies.
    NotAvailable,
}

impl Repository {
    pub fn find_earliest_release_containing(
        &self,
        unit: &ResolvedReleaseUnit,
        matcher: &TagMatcher,
        cid: &CommitId,
    ) -> Result<ReleaseAvailability> {
        if let Some((tag_oid, _tag_name, version)) = self.find_latest_tag_for_project(matcher)? {
            if self.repo.graph_descendant_of(tag_oid, cid.0)? || tag_oid == cid.0 {
                let v = Version::parse_like(&unit.version, version.to_string())?;
                return Ok(ReleaseAvailability::ExistingRelease(v));
            }
        }

        let head_ref = self.repo.head()?;
        let head_commit = head_ref.peel_to_commit()?;
        let head_id = head_commit.id();

        if head_id == cid.0 || self.repo.graph_descendant_of(head_id, cid.0)? {
            Ok(ReleaseAvailability::NewRelease)
        } else {
            Ok(ReleaseAvailability::NotAvailable)
        }
    }
}

/// Information about the state of the projects in the repository corresponding
/// to a "release" commit where all of the projects have been assigned version
/// numbers, and the commit should have made it out into the wild only if all of
/// the CI tests passed.
#[derive(Clone, Debug, Default)]
pub struct ReleaseCommitInfo {
    /// The Git commit-ish that this object describes. May be None when there is
    /// no upstream `release` branch, in which case this struct will contain no
    /// genuine information.
    pub commit: Option<CommitId>,

    /// A list of projects and their release information as of this commit. This
    /// list includes every tracked project in this commit. Not all of those
    /// projects necessarily were released with this commit, if they were
    /// unchanged from a previous release commit.
    pub projects: Vec<ReleasedProjectInfo>,
}

impl ReleaseCommitInfo {
    /// Attempt to find info for a prior release of the named project.
    ///
    /// Information may be missing if the project was only added to the
    /// repository after this information was recorded.
    pub fn lookup_project(&self, unit: &ResolvedReleaseUnit) -> Option<&ReleasedProjectInfo> {
        self.projects
            .iter()
            .find(|&rpi| rpi.qnames == *unit.qualified_names())
    }

    /// Find information about a project release if it occurred at this moment.
    ///
    /// This function is like `lookup_project()`, but also returns None if the
    /// "age" of any identified release is not zero.
    pub fn lookup_if_released(&self, unit: &ResolvedReleaseUnit) -> Option<&ReleasedProjectInfo> {
        self.lookup_project(unit).filter(|rel| rel.age == 0)
    }
}

/// Serializable state information about a single project in a release commit.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReleasedProjectInfo {
    /// The qualified names of this project, equivalent to the same-named
    /// property of the ResolvedReleaseUnit struct.
    pub qnames: Vec<String>,

    /// The version of the project in this commit, as text.
    pub version: String,

    /// The number of consecutive release commits for which this project
    /// has had the assigned version string. If zero, that means that the
    /// specified version was first released with this commit.
    pub age: usize,
}

/// A data structure recording changes made when rewriting files
/// in the repository.
#[derive(Debug, Default)]
pub struct ChangeList {
    pub(super) paths: Vec<RepoPathBuf>,
}

impl ChangeList {
    /// Mark the file at this path as having been updated.
    pub fn add_path(&mut self, p: &RepoPath) {
        self.paths.push(p.to_owned());
    }

    /// Get the paths in this changelist.
    pub fn paths(&self) -> impl Iterator<Item = &RepoPath> {
        self.paths[..].iter().map(|p| p.as_ref())
    }
}
