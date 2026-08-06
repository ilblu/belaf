//! Commit-history analysis.
//!
//! Holds [`RepoHistory`] — the set of commits attributed to one release unit
//! since its boundary (last release tag or baseline) — and
//! [`Repository::analyze_histories`], which walks the commit graph once and
//! fills in a history for every unit.

use anyhow::bail;
use tracing::{info, warn};

use crate::core::{
    errors::Result,
    git::{
        path_matcher::is_binary_affecting,
        repository::{CommitId, ReleaseCommitInfo, RepoPath, Repository},
    },
    resolved_release_unit::{ReleaseUnitId, ResolvedReleaseUnit, UnitKind},
    tag_format::TagMatcher,
};

#[derive(Clone, Debug)]
pub enum HistoryBoundary {
    ReleaseTag {
        commit: CommitId,
        tag_name: String,
        version: semver::Version,
    },
    Baseline {
        commit: CommitId,
    },
}

#[derive(Clone, Debug)]
pub struct RepoHistory {
    pub(super) commits: Vec<CommitId>,
    pub(super) boundary: Option<HistoryBoundary>,
    /// F7 — for commits collected via the dependency closure (not the unit's
    /// own paths), records which closure member (an internal crate) the commit
    /// is attributed to. Drives the `via <crate>` changelog prefix. Commits
    /// hitting the unit's own paths are absent (self wins → no `via`).
    pub(super) provenance: std::collections::HashMap<CommitId, ReleaseUnitId>,
    /// Which `[cascade_inputs]` nodes contributed to the commits this unit
    /// collected.
    ///
    /// Deliberately separate from `provenance`: that map is only written when
    /// the unit itself was *not* hit and holds at most one contributor per
    /// commit, so it cannot express "the unit changed on its own **and** a
    /// declared input was patched" — which is precisely the case the bump
    /// floor and the manifest provenance must handle. Stays empty (and costs
    /// nothing) when no inputs are configured.
    pub(super) input_hits: std::collections::BTreeSet<ReleaseUnitId>,
}

impl RepoHistory {
    pub fn boundary_commit(&self) -> Option<CommitId> {
        match &self.boundary {
            Some(HistoryBoundary::ReleaseTag { commit, .. }) => Some(*commit),
            Some(HistoryBoundary::Baseline { commit }) => Some(*commit),
            None => None,
        }
    }

    pub fn release_version(&self) -> Option<&semver::Version> {
        match &self.boundary {
            Some(HistoryBoundary::ReleaseTag { version, .. }) => Some(version),
            _ => None,
        }
    }

    pub fn has_release_tag(&self) -> bool {
        matches!(&self.boundary, Some(HistoryBoundary::ReleaseTag { .. }))
    }

    pub fn release_info(&self, repo: &Repository) -> Result<ReleaseCommitInfo> {
        let mut info = repo.get_bootstrap_release_info();

        if let Some(HistoryBoundary::ReleaseTag { version, .. }) = &self.boundary {
            for proj_info in &mut info.projects {
                if proj_info.version == version.to_string() {
                    proj_info.age = 0;
                }
            }
        }

        Ok(info)
    }

    pub fn n_commits(&self) -> usize {
        self.commits.len()
    }

    pub fn commits(&self) -> impl IntoIterator<Item = &CommitId> {
        &self.commits[..]
    }

    /// F7 — closure-provenance: which internal crate (by `ReleaseUnitId`) a
    /// collected commit is attributed to, when it came from a dependency-closure
    /// member rather than the unit's own paths. Absent = the unit's own change.
    pub fn provenance_for(&self, commit: CommitId) -> Option<ReleaseUnitId> {
        self.provenance.get(&commit).copied()
    }

    /// The `[cascade_inputs]` nodes that contributed to this unit's collected
    /// commits, in ascending id order. Empty for every unit in a repo with no
    /// `[cascade_inputs]` configured.
    pub fn input_hits(&self) -> impl Iterator<Item = ReleaseUnitId> + '_ {
        self.input_hits.iter().copied()
    }
}

impl Repository {
    /// Figure out which commits in the history affect each project since its
    /// last release.
    ///
    /// This gets a little tricky since not all projects in the repo are
    /// released in lockstep. For each individiual project, we need to analyze
    /// the history from HEAD to its most recent release commit. I worry about
    /// the efficiency of this so we trace all the histories at once to try to
    /// improve that.
    pub fn analyze_histories(
        &self,
        projects: &[ResolvedReleaseUnit],
        matchers: &[TagMatcher],
        closures: &[Vec<usize>],
        binary_affecting: &crate::core::config::syntax::BinaryAffectingConfiguration,
    ) -> Result<Vec<RepoHistory>> {
        if projects.len() != matchers.len() {
            bail!(
                "internal error: analyze_histories got {} projects and {} matchers; lengths must match",
                projects.len(),
                matchers.len()
            );
        }

        let mut histories = vec![
            RepoHistory {
                commits: Vec::new(),
                boundary: None,
                provenance: std::collections::HashMap::new(),
                input_hits: std::collections::BTreeSet::new(),
            };
            projects.len()
        ];

        let baseline_tag_oid = self.find_baseline_tag()?;
        let repo_has_any_version_tags = self.repo_has_any_version_tags()?;

        for (i, unit) in projects.iter().enumerate() {
            let matcher = &matchers[i];
            // F11b — a prerelease unit's boundary is its last STABLE tag, so the
            // base level is computed from all commits since stable (across any
            // intervening prereleases), not just since the last prerelease.
            let is_prerelease_unit = unit
                .bump_override
                .as_ref()
                .and_then(|o| o.prerelease.as_ref())
                .is_some();
            let latest = if is_prerelease_unit {
                self.find_latest_stable_tag_for_project(matcher)?
            } else {
                self.find_latest_tag_for_project(matcher)?
            };
            if let Some((tag_oid, tag_name, version)) = latest {
                info!(
                    "found release tag for {}: {} (v{}) via template `{}`",
                    unit.user_facing_name,
                    tag_name,
                    version,
                    matcher.template()
                );
                histories[i].boundary = Some(HistoryBoundary::ReleaseTag {
                    commit: CommitId(tag_oid),
                    tag_name,
                    version,
                });
            } else if let Some(baseline_oid) = baseline_tag_oid {
                info!(
                    "no release tag for {}, using baseline tag belaf-baseline",
                    unit.user_facing_name
                );
                histories[i].boundary = Some(HistoryBoundary::Baseline {
                    commit: CommitId(baseline_oid),
                });
            } else if repo_has_any_version_tags && unit.kind == UnitKind::Deploy {
                // Defensive guard. If the repo already has version-shaped
                // tags but none matched THIS project's template, falling
                // back to "all commits since repo start" is almost
                // certainly going to inflate the recommended bump. The
                // legacy code path hit this bug for every npm/maven/pypa/go
                // project. Surface the diagnostic loudly, do NOT fall
                // through silently.
                //
                // F1 — only `Deploy` units bail: `Internal`/`Ignore` units have
                // no tags by design. Their own history is never used for
                // candidacy (deploy units collect their commits within the
                // *deploy* unit's tag window via the closure), so analyzing
                // from repo start for them is harmless.
                bail!(
                    "could not locate a previous-release tag for `{name}` (tried template `{tmpl}`), \
                     but this repo already has version-shaped tags. \
                     Refusing to analyze the full history — that would over-count old commits and inflate the bump. \
                     Likely causes: (1) the project's `tag_format` in `belaf/config.toml` doesn't match how previous tags were written; \
                     (2) the project is genuinely new — in that case, create a baseline with `git tag belaf-baseline <commit>` to mark the starting point. \
                     Override-only path: set `tag_format = \"...\"` on the `[release_unit.<name>]` block to match the existing tag shape.",
                    name = unit.user_facing_name,
                    tmpl = matcher.template(),
                );
            } else if unit.kind == UnitKind::Deploy {
                warn!(
                    "no release tag or baseline found for {}, and the repo has no version tags at all — analyzing all commits since repo start. This is correct only for a brand-new repo.",
                    unit.user_facing_name
                );
            }
            // else: Internal/Ignore unit with no tag — expected, analyze from
            // repo start silently (boundary stays None).
        }

        let commit_cache_size = std::num::NonZeroUsize::new(self.analysis_config.commit_cache_size)
            .unwrap_or(std::num::NonZeroUsize::new(512).expect("BUG: 512 is non-zero"));
        let tree_cache_size = std::num::NonZeroUsize::new(self.analysis_config.tree_cache_size)
            .unwrap_or(std::num::NonZeroUsize::new(3).expect("BUG: 3 is non-zero"));

        let mut commit_data = lru::LruCache::new(commit_cache_size);
        let mut trees = lru::LruCache::new(tree_cache_size);

        let mut dopts = git2::DiffOptions::new();
        dopts.include_typechange(true);

        // F-decouple — WHETHER is path-based only: the per-commit hit map is
        // populated solely from binary-affecting path matches. Scope no longer
        // writes it (scope/type drive changelog + `belaf check`, never the bump
        // decision — see F6).
        debug_assert_eq!(closures.len(), projects.len());

        // Tier-3 (F4-Glob) overlap guard prep: which units own residual globs.
        // `any_globs` is false for every all-prefix repo (e.g. clikd), so the
        // per-path overlap check below is entirely skipped — zero overhead.
        //
        // `[cascade_inputs]` nodes are excluded from the guard entirely. For a
        // real unit, a path claimed by two owners is a partition break; for an
        // input the overlap is the whole point (a path can belong to a service
        // *and* feed a declared input). The guard stays fully strict between
        // real units.
        let unit_has_globs: Vec<bool> = projects
            .iter()
            .map(|u| u.repo_paths.has_globs() && !u.is_cascade_input)
            .collect();
        let any_globs = unit_has_globs.iter().any(|&b| b);
        let any_inputs = projects.iter().any(|u| u.is_cascade_input);

        // note that we don't "know" that unit_idx = project.ident
        for unit_idx in 0..projects.len() {
            let mut walk = self.repo.revwalk()?;
            walk.push_head()?;

            if let Some(boundary_commit) = histories[unit_idx].boundary_commit() {
                walk.hide(boundary_commit.0)?;
            }

            // Walk through the history, finding relevant commits. The full
            // codepath loads up trees for each commit and its parents, computes
            // the diff, and compares that against the path-matchers for each
            // project to decide if a given commit affects a given project. The
            // intention is that the LRU caches will make it so that little
            // redundant work is performed.

            for maybe_oid in walk {
                let oid = maybe_oid?;

                // Hopefully this commit is already in the cache, but if not ...
                if !commit_data.contains(&oid) {
                    // Get the two relevant trees and compute their diff. We have to
                    // jump through some hoops to support the root commit (with no
                    // parents) but it's not really that bad. We also have to pop() the
                    // trees out of the LRU because get() holds a mutable reference to
                    // the cache, which prevents us from looking at two trees
                    // simultaneously.

                    let commit = self.repo.find_commit(oid)?;
                    let ctid = commit.tree_id();
                    let cur_tree = match trees.pop(&ctid) {
                        Some(t) => t,
                        None => self.repo.find_tree(ctid)?,
                    };

                    let (maybe_ptid, maybe_parent_tree) = if commit.parent_count() == 0 {
                        (None, None) // this is the first commit in the history!
                    } else {
                        let parent = commit.parent(0)?;
                        let ptid = parent.tree_id();
                        let parent_tree = match trees.pop(&ptid) {
                            Some(t) => t,
                            None => self.repo.find_tree(ptid)?,
                        };
                        (Some(ptid), Some(parent_tree))
                    };

                    let diff = self.repo.diff_tree_to_tree(
                        maybe_parent_tree.as_ref(),
                        Some(&cur_tree),
                        Some(&mut dopts),
                    )?;

                    trees.put(ctid, cur_tree);
                    if let (Some(ptid), Some(pt)) = (maybe_ptid, maybe_parent_tree) {
                        trees.put(ptid, pt);
                    }

                    let mut hit_buf = vec![false; projects.len()];

                    // Skip merge commits (>=2 parents): the per-unit, no-squash
                    // commit convention means non-merge commits carry all the
                    // signal.
                    if commit.parent_count() < 2 {
                        for delta in diff.deltas() {
                            for file in &[delta.old_file(), delta.new_file()] {
                                if let Some(path_bytes) = file.path_bytes() {
                                    // F3 — only binary-affecting paths count
                                    // toward WHETHER (skip tests/docs/… either
                                    // side of a rename is conservative).
                                    if !is_binary_affecting(path_bytes, binary_affecting) {
                                        continue;
                                    }
                                    let path = RepoPath::new(path_bytes);
                                    let mut matched: Vec<usize> = Vec::new();
                                    for (idx, unit) in projects.iter().enumerate() {
                                        if unit.repo_paths.repo_path_matches(path) {
                                            hit_buf[idx] = true;
                                            if !unit.is_cascade_input {
                                                matched.push(idx);
                                            }
                                        }
                                    }
                                    // Tier-3 overlap guard — a path claimed by a
                                    // residual-glob unit AND any other unit is a
                                    // partition break. Never silently co-own:
                                    // hard-error so the human resolves it (narrow
                                    // the glob). Prefix↔prefix overlaps can't
                                    // happen (make_disjoint), so this only fires
                                    // when a glob is involved.
                                    if any_globs
                                        && matched.len() > 1
                                        && matched.iter().any(|&i| unit_has_globs[i])
                                    {
                                        let owners: Vec<String> = matched
                                            .iter()
                                            .map(|&i| projects[i].user_facing_name.clone())
                                            .collect();
                                        bail!(
                                            "path `{}` is claimed by multiple release units {:?} \
                                             via a residual glob — narrow the glob so it doesn't \
                                             overlap build-owned territory; belaf will not silently \
                                             pick an owner",
                                            path.escaped(),
                                            owners,
                                        );
                                    }
                                }
                            }
                        }
                    }

                    commit_data.put(oid, hit_buf);
                }

                let hits = commit_data
                    .get(&oid)
                    .expect("BUG: commit data should be in cache after put()");

                // F2 — a unit collects every commit that touched ANY member of
                // its dependency closure (itself + transitive in-repo deps),
                // bounded to its own tag window (this walk hid its boundary). A
                // leaf unit's closure is just `{unit}`, reducing to the historical
                // per-unit behaviour.
                if closures[unit_idx].iter().any(|&c| hits[c]) {
                    let cid = CommitId(oid);
                    histories[unit_idx].commits.push(cid);
                    // F7 — provenance: if the commit didn't touch the unit's OWN
                    // paths, attribute it to the closure member it did touch
                    // (self wins → no `via`; diamond → lowest id, deterministic).
                    if !hits[unit_idx] {
                        if let Some(&member) = closures[unit_idx]
                            .iter()
                            .filter(|&&c| c != unit_idx && hits[c])
                            .min()
                        {
                            histories[unit_idx].provenance.insert(cid, member);
                        }
                    }
                    // `[cascade_inputs]` — record every declared input in the
                    // closure the commit touched. Unlike `provenance` this is
                    // additive and independent of whether the unit was hit on
                    // its own paths, so "changed itself AND the base was
                    // patched" is representable. Drives the bump floor and the
                    // manifest's `cascade_inputs` provenance.
                    if any_inputs {
                        for &member in &closures[unit_idx] {
                            if hits[member] && projects[member].is_cascade_input {
                                histories[unit_idx].input_hits.insert(member);
                            }
                        }
                    }
                }
            }
        }

        Ok(histories)
    }
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod history_tests;
