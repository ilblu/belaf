//! Auto-discovery of release units from the repo working tree.
//!
//! Walks the git index, dispatches each path to either a
//! [`WorkspaceDiscoverer`] (for ecosystems with native workspace
//! protocols — cargo metadata, npm `workspaces`, maven `<modules>`)
//! or to a [`FormatHandler`]'s single-package
//! [`FormatHandler::discover_single`].
//!
//! Two consumers feed [`crate::core::session::AppBuilder`]: this
//! orchestrator (auto-discovered units) and the explicit
//! `[release_unit.X]` resolver. The session then merges both into
//! the graph.

use std::collections::HashSet;

use crate::core::{
    ecosystem::format_handler::{
        is_path_inside_any, DiscoveryBatch, FormatHandlerRegistry, UnitOwnership,
        WorkspaceDiscovererRegistry,
    },
    errors::Result,
    git::repository::{RepoPathBuf, Repository},
};

/// Walk the repo for every unconfigured manifest.
///
/// `ownership` carries every path a `[release_unit.X]` block already covers
/// (manifest directories, satellites, literal `paths`) plus the
/// `[ignore_paths]` list. It filters the walk *and* is handed to the workspace
/// discoverers, which enumerate members from a single root call and therefore
/// cannot be filtered by the walk alone.
pub fn discover_implicit_release_units(
    repo: &Repository,
    handlers: &FormatHandlerRegistry,
    discoverers: &WorkspaceDiscovererRegistry,
    ownership: &UnitOwnership,
) -> Result<DiscoveryBatch> {
    let skip_paths: Vec<RepoPathBuf> = ownership.paths().cloned().collect();

    // Collect index paths once. We can't easily do per-path dispatch
    // inline because workspace discoverers consume multiple paths in
    // one call (cargo metadata enumerates every workspace member from
    // one Cargo.toml).
    let mut paths: Vec<RepoPathBuf> = Vec::new();
    repo.scan_paths(|p| {
        if !is_path_inside_any(p, &skip_paths) {
            paths.push(p.to_owned());
        }
        Ok(())
    })?;

    let mut batch = DiscoveryBatch::default();
    let mut consumed: HashSet<RepoPathBuf> = HashSet::new();

    // First pass: workspace discoverers. They get the chance to claim
    // a workspace-root path and emit a batch of units; their anchor
    // paths plus the workspace-root itself are added to `consumed` so
    // the second pass doesn't re-emit them as single-package units
    // (virtual workspaces contribute a root but no unit-of-its-own).
    for path in &paths {
        if consumed.contains(path) {
            continue;
        }
        for ws in discoverers.discoverers() {
            if ws.claims(repo, path) {
                let new_batch = ws.discover(repo, path, ownership)?;
                consumed.insert(path.clone());
                for u in &new_batch.units {
                    consumed.insert(u.anchor_manifest.clone());
                }
                batch.extend(new_batch);
                break;
            }
        }
    }

    // Second pass: per-path single-manifest discovery for every path
    // a workspace discoverer didn't claim.
    let mut claimed_dirs: HashSet<RepoPathBuf> = HashSet::new();
    for path in &paths {
        if consumed.contains(path) {
            continue;
        }
        let Some(handler) = handlers.handler_for(path) else {
            continue;
        };
        // Dedup at the directory level — pypa matches multiple files
        // (setup.py / setup.cfg / pyproject.toml) per project; we
        // only want one DiscoveredUnit out of any given directory.
        let (parent, _) = path.split_basename();
        let parent_buf = parent.to_owned();
        if claimed_dirs.contains(&parent_buf) {
            continue;
        }
        if let Some(unit) = handler.discover_single(repo, path)? {
            claimed_dirs.insert(parent_buf);
            batch.units.push(unit);
        }
    }

    Ok(batch)
}
