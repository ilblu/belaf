//! Project groups — sets of projects that release together.
//!
//! A group is a logical bundle of projects that must share one bump and one
//! release moment. The canonical use case is a GraphQL schema published as
//! both an npm package (`@org/schema`) and a Maven artifact
//! (`com.org:schema`): they describe the same surface, must move in lockstep,
//! and breaking the lockstep would mean a server-client contract break.
//!
//! # Plan §5 — first-class graph primitive
//!
//! The plan envisions `Group` as an enum variant of a unified `GraphNode`
//! type, sharing toposort/bump/cycle logic with `Project`. The current
//! implementation takes a more incremental path: groups live as a sibling
//! collection on [`crate::core::graph::ProjectGraph`] rather than as a real
//! `petgraph` node type. Practically this gives us:
//!
//! - shared-bump-state semantics (CLI propagates the highest member bump
//!   to the whole group before manifest emission),
//! - dep-filtering (deps between same-group members are skipped during
//!   bump propagation),
//! - manifest population (`groups[]` + `releases[].group_id`).
//!
//! Going to a true unified node type is an additive change later — no
//! manifest schema bump needed because the wire format already speaks in
//! terms of `groups[]` + `group_id`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error as ThisError;

use crate::core::project::ProjectId;

/// Wire-format group identifier. Pattern: `^[a-z0-9][a-z0-9-]*$`, max 64
/// chars (validated by the JSON schema; we re-validate here to fail fast at
/// config-load time).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GroupId(String);

#[derive(Debug, ThisError)]
pub enum GroupIdError {
    #[error("group id `{0}` must match pattern ^[a-z0-9][a-z0-9-]*$ (lowercase, digits, dashes)")]
    InvalidPattern(String),
    #[error("group id `{0}` exceeds 64 character limit (got {1})")]
    TooLong(String, usize),
    #[error("group id cannot be empty")]
    Empty,
}

impl GroupId {
    pub fn new(s: impl Into<String>) -> Result<Self, GroupIdError> {
        let s: String = s.into();
        if s.is_empty() {
            return Err(GroupIdError::Empty);
        }
        if s.len() > 64 {
            return Err(GroupIdError::TooLong(s.clone(), s.len()));
        }
        let mut chars = s.chars();
        let first = chars.next().expect("non-empty checked above");
        if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
            return Err(GroupIdError::InvalidPattern(s));
        }
        for c in chars {
            if !c.is_ascii_lowercase() && !c.is_ascii_digit() && c != '-' {
                return Err(GroupIdError::InvalidPattern(s));
            }
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for GroupId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A bundle of projects that release together. Member project IDs are
/// resolved at config-binding time from their user-facing names.
#[derive(Clone, Debug)]
pub struct Group {
    pub id: GroupId,
    pub members: Vec<ProjectId>,
    /// Optional group-level `tag_format` override (B10). When present
    /// every member release uses this template instead of the per-
    /// ecosystem default. Per-project overrides still win over this.
    pub tag_format: Option<String>,
}

/// Index of every group in the repo, plus the reverse map `ProjectId →
/// GroupId` so per-project lookups are O(1).
#[derive(Clone, Debug, Default)]
pub struct GroupSet {
    groups: Vec<Group>,
    member_of: HashMap<ProjectId, usize>,
}

impl GroupSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.groups.len()
    }

    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Group> {
        self.groups.iter()
    }

    /// Look up the group that contains the given project, if any.
    pub fn group_of(&self, pid: ProjectId) -> Option<&Group> {
        self.member_of.get(&pid).map(|&idx| &self.groups[idx])
    }

    /// Look up a group by id.
    pub fn by_id(&self, id: &GroupId) -> Option<&Group> {
        self.groups.iter().find(|g| &g.id == id)
    }

    /// Register a group. Returns `Err` if any member is already in another
    /// group (one project, one group — overlapping membership is rejected
    /// because shared-bump-state would be ambiguous).
    pub fn add(&mut self, group: Group) -> Result<(), GroupSetError> {
        for &pid in &group.members {
            if let Some(&existing) = self.member_of.get(&pid) {
                return Err(GroupSetError::MemberInMultipleGroups {
                    project: pid,
                    first: self.groups[existing].id.clone(),
                    second: group.id,
                });
            }
        }
        if self.groups.iter().any(|g| g.id == group.id) {
            return Err(GroupSetError::DuplicateId(group.id));
        }
        let idx = self.groups.len();
        for &pid in &group.members {
            self.member_of.insert(pid, idx);
        }
        self.groups.push(group);
        Ok(())
    }
}

#[derive(Debug, ThisError)]
pub enum GroupSetError {
    #[error(
        "project {project} cannot be a member of both group `{first}` and group `{second}` — overlapping group membership is unsupported"
    )]
    MemberInMultipleGroups {
        project: ProjectId,
        first: GroupId,
        second: GroupId,
    },
    #[error("duplicate group id `{0}`")]
    DuplicateId(GroupId),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_id_accepts_valid_patterns() {
        assert!(GroupId::new("graphql-schema").is_ok());
        assert!(GroupId::new("a").is_ok());
        assert!(GroupId::new("0abc").is_ok());
        assert!(GroupId::new("schema-v1").is_ok());
    }

    #[test]
    fn group_id_rejects_uppercase() {
        let err = GroupId::new("GraphQL").unwrap_err();
        assert!(matches!(err, GroupIdError::InvalidPattern(_)));
    }

    #[test]
    fn group_id_rejects_underscores() {
        let err = GroupId::new("my_group").unwrap_err();
        assert!(matches!(err, GroupIdError::InvalidPattern(_)));
    }

    #[test]
    fn group_id_rejects_leading_dash() {
        let err = GroupId::new("-foo").unwrap_err();
        assert!(matches!(err, GroupIdError::InvalidPattern(_)));
    }

    #[test]
    fn group_id_rejects_too_long() {
        let s = "a".repeat(65);
        let err = GroupId::new(&s).unwrap_err();
        assert!(matches!(err, GroupIdError::TooLong(_, 65)));
    }

    #[test]
    fn group_id_rejects_empty() {
        let err = GroupId::new("").unwrap_err();
        assert!(matches!(err, GroupIdError::Empty));
    }

    fn mkgroup(id: &str, members: Vec<ProjectId>) -> Group {
        Group {
            id: GroupId::new(id).unwrap(),
            members,
            tag_format: None,
        }
    }

    #[test]
    fn group_set_indexes_member_to_group() {
        let mut s = GroupSet::new();
        s.add(mkgroup("schema", vec![0, 2, 5])).unwrap();
        assert_eq!(s.group_of(0).unwrap().id.as_str(), "schema");
        assert_eq!(s.group_of(5).unwrap().id.as_str(), "schema");
        assert!(s.group_of(1).is_none());
    }

    #[test]
    fn group_set_rejects_overlapping_members() {
        let mut s = GroupSet::new();
        s.add(mkgroup("a", vec![0, 1])).unwrap();
        let err = s.add(mkgroup("b", vec![1, 2])).unwrap_err();
        assert!(matches!(
            err,
            GroupSetError::MemberInMultipleGroups { project: 1, .. }
        ));
    }

    #[test]
    fn group_set_rejects_duplicate_id() {
        let mut s = GroupSet::new();
        s.add(mkgroup("a", vec![0])).unwrap();
        let err = s.add(mkgroup("a", vec![1])).unwrap_err();
        assert!(matches!(err, GroupSetError::DuplicateId(_)));
    }
}
