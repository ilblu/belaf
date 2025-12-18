// Copyright 2020 Peter Williams <peter@newton.cx> and collaborators
// Licensed under the MIT License.

//! Updating project versioning metadata in the repository.

use crate::core::release::{errors::Result, repository::ChangeList, session::AppSession};

/// A trait for something that can perform some kind of metadata rewriting.
pub trait Rewriter: std::fmt::Debug {
    /// Rewrite the metafiles to embed the project versions and internal
    /// dependency specifications captured in the current runtime state.
    fn rewrite(&self, app: &AppSession, changes: &mut ChangeList) -> Result<()>;

    /// Rewrite the metafiles to embed the Belaf-specific internal dependency
    /// version requirement metadata. This should not be done as part of the
    /// mainline rewriting process, but it is convenient to be able to do this
    /// on behalf of the user (e.g. generating `thiscommit:` strings and
    /// bootstrapping).
    fn rewrite_belaf_requirements(
        &self,
        _app: &AppSession,
        _changes: &mut ChangeList,
    ) -> Result<()> {
        Ok(())
    }
}
