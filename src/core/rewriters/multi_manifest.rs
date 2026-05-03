//! `MultiManifestRewriter` — writes the same `new_version` into every
//! file of a [`crate::core::release_unit::VersionSource::Manifests`]
//! source in lockstep.
//!
//! Phase D.6 of `BELAF_MASTER_PLAN.md`. Heal-forward semantics: if
//! some manifests are already at `new_version` (e.g. partial-state
//! recovery from a previous failed run), the remaining ones are
//! still written and a `warn!` is emitted. **Never reject a partial
//! state** — that breaks CI re-runs.

use std::path::Path;

use thiserror::Error;
use tracing::warn;

use crate::core::git::repository::{RepoPathBuf, Repository};
use crate::core::release_unit::ManifestFile;
use crate::core::version_field::{self, VersionFieldError};

/// Per-call summary so callers can log / surface what happened.
#[derive(Debug, Default)]
pub struct MultiManifestReport {
    /// Manifests that were successfully written to `new_version`.
    pub wrote: Vec<RepoPathBuf>,
    /// Manifests that were already at `new_version` (idempotent skip).
    pub already_at_target: Vec<RepoPathBuf>,
}

#[derive(Debug, Error)]
pub enum MultiManifestError {
    #[error("rewriting `{path}` failed: {source}")]
    Single {
        path: String,
        #[source]
        source: VersionFieldError,
    },
}

/// Write `new_version` into every manifest in `manifests`. Files
/// already at `new_version` are skipped silently. Files that miss
/// the version field error out in the dispatcher's
/// [`crate::core::version_field::read`] / `write` step.
///
/// Returns aggregate [`MultiManifestReport`] on success; the first
/// hard error short-circuits and is wrapped in a [`MultiManifestError`].
pub fn write_all(
    manifests: &[ManifestFile],
    new_version: &str,
    repo: &Repository,
) -> Result<MultiManifestReport, MultiManifestError> {
    let mut report = MultiManifestReport::default();

    for m in manifests {
        let abs = repo.resolve_workdir(&m.path);

        // Read first so we can decide idempotent vs write. Read
        // failure (file missing, malformed) is a hard error — we
        // would otherwise silently skip half a Tauri triplet.
        let current = read_or_warn(m, &abs)?;

        if current.as_deref() == Some(new_version) {
            report.already_at_target.push(m.path.clone());
            continue;
        }

        version_field::write(&m.version_field, &abs, new_version).map_err(|e| {
            MultiManifestError::Single {
                path: abs.display().to_string(),
                source: e,
            }
        })?;
        report.wrote.push(m.path.clone());
    }

    if !report.already_at_target.is_empty() && !report.wrote.is_empty() {
        // Partial-state recovery — log so users see why some files
        // got "no diff" while others moved.
        warn!(
            "MultiManifestRewriter heal-forward: {} of {} manifests were already at `{}` (skipped); {} written",
            report.already_at_target.len(),
            manifests.len(),
            new_version,
            report.wrote.len(),
        );
    }

    Ok(report)
}

/// Wrapper that surfaces the read result as `Option<String>`. None
/// means "no version field present" — only valid if write would
/// surface that as an error itself; otherwise we propagate.
fn read_or_warn(m: &ManifestFile, abs: &Path) -> Result<Option<String>, MultiManifestError> {
    match version_field::read(&m.version_field, abs) {
        Ok(v) => Ok(Some(v)),
        Err(VersionFieldError::VersionFieldMissing { .. }) => Ok(None),
        Err(e) => Err(MultiManifestError::Single {
            path: abs.display().to_string(),
            source: e,
        }),
    }
}

// ---------------------------------------------------------------------------
// Rewriter trait integration.
// ---------------------------------------------------------------------------

use crate::core::errors::Result as BelafResult;
use crate::core::git::repository::ChangeList;
use crate::core::resolved_release_unit::ReleaseUnitId;
use crate::core::rewriters::Rewriter;
use crate::core::session::AppSession;

/// `Rewriter` that drives [`write_all`] for configured
/// `[release_unit.X]` entries with `manifests = [...]`. The list of
/// `ManifestFile`s is captured at session-build time and frozen.
#[derive(Debug)]
pub struct MultiManifestRewriter {
    unit_id: ReleaseUnitId,
    manifests: Vec<ManifestFile>,
}

impl MultiManifestRewriter {
    pub fn new(unit_id: ReleaseUnitId, manifests: Vec<ManifestFile>) -> Self {
        Self { unit_id, manifests }
    }
}

impl Rewriter for MultiManifestRewriter {
    fn rewrite(&self, app: &AppSession, changes: &mut ChangeList) -> BelafResult<()> {
        let unit = app.graph().lookup(self.unit_id);
        let new_version = unit.version.to_string();
        let report = write_all(&self.manifests, &new_version, &app.repo)
            .map_err(|e| anyhow::anyhow!("multi-manifest rewrite failed: {e}"))?;
        for path in report.wrote {
            changes.add_path(&path);
        }
        Ok(())
    }
}
