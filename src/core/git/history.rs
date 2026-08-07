//! Commit-history analysis.
//!
//! Holds [`RepoHistory`] — the set of commits attributed to one release unit
//! since its boundary (last release tag or baseline) — and
//! [`Repository::analyze_histories`], which walks the commit graph once and
//! fills in a history for every unit.

use anyhow::{anyhow, bail, Context as _};
use tracing::{info, warn};

use crate::core::{
    errors::Result,
    git::{
        path_matcher::is_binary_affecting,
        repository::{CommitId, ReleaseCommitInfo, RepoPath, Repository},
    },
    release_unit::BaselineSpec,
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
    /// The last *prerelease* tag of a prerelease unit that has never had a
    /// stable release.
    ///
    /// Separate from [`Self::ReleaseTag`] on purpose. It plays only one of
    /// that variant's two roles: it bounds the commit window, but it is not a
    /// stable anchor to compute a new base version from — so
    /// [`RepoHistory::release_version`] deliberately reports `None` for it.
    /// Feeding a prerelease version in as the "last stable" would make the
    /// base creep on every run and reset the prerelease counter each time.
    PrereleaseTag {
        commit: CommitId,
        tag_name: String,
        version: semver::Version,
    },
    Baseline {
        commit: CommitId,
    },
}

/// A `Deploy` unit with no matching release tag in a repo that already has
/// version-shaped tags.
///
/// Analyzing such a unit from repo start would over-count old commits and
/// inflate its bump, so the run refuses — but the refusal has to name
/// **every** offender at once. Collecting them into values instead of
/// `bail!`ing on the first is what lets `belaf baseline` report and fix the
/// whole set in one pass rather than one unit per run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UntaggedDeployUnit {
    /// Disambiguated, user-facing unit name — for display.
    pub name: String,

    /// The `[release_unit.<key>]` key a `baseline` has to be written under.
    /// This is the unit's narrow (unqualified) name, which is what both an
    /// explicit block and a partial-override block key on. It equals
    /// [`Self::name`] unless two units of different ecosystems share a name.
    pub config_key: String,

    /// The tag template the lookup tried and missed.
    pub tag_template: String,
}

/// Per-unit history boundaries plus the units that could not get one.
///
/// Produced by [`Repository::resolve_history_boundaries`], consumed both by
/// [`Repository::analyze_histories`] (which turns a non-empty `untagged`
/// into a hard error) and by `belaf baseline` (which reports/fixes it
/// without running a release). One implementation, two callers.
#[derive(Clone, Debug)]
pub struct HistoryBoundaries {
    /// Parallel to the `projects` slice passed in. `None` = analyze from
    /// repo start.
    pub boundaries: Vec<Option<HistoryBoundary>>,

    /// Every offending unit, in `projects` order.
    pub untagged: Vec<UntaggedDeployUnit>,
}

/// Render the one error that names every untagged deploy unit.
///
/// The diagnostic (why belaf refuses, the likely causes, the fixes) appears
/// once at the top; the units are a plain list underneath. Repeating the
/// paragraph per unit would bury the list, which is the part the user has to
/// act on.
pub fn untagged_deploy_units_error(units: &[UntaggedDeployUnit]) -> crate::core::errors::Error {
    let listing = units
        .iter()
        .map(|u| format!("  • `{}` (tried template `{}`)", u.name, u.tag_template))
        .collect::<Vec<_>>()
        .join("\n");

    let first_key = units
        .first()
        .map(|u| u.config_key.as_str())
        .unwrap_or("<name>");

    anyhow!(
        "could not locate a previous-release tag for {n} release unit{plural}, \
         but this repo already has version-shaped tags. \
         Refusing to analyze the full history — that would over-count old commits and inflate the bump.\n\
         \n{listing}\n\
         \n\
         Likely causes: (1) the unit's `tag_format` in `belaf/config.toml` doesn't match how previous \
         tags were written; (2) the unit is genuinely new and has never been released.\n\
         \n\
         Fixes, per unit, in `belaf/config.toml`:\n\
         \x20 • never released    → [release_unit.{first_key}]\n\
         \x20                        baseline = \"first-release\"\n\
         \x20 • released before   → [release_unit.{first_key}]\n\
         \x20                        baseline = \"<commit-sha>\"   # start the window here\n\
         \x20 • tags exist, template is wrong → set `tag_format = \"...\"` to match them\n\
         \n\
         `belaf baseline` lists exactly these units; `belaf baseline --fix` writes \
         `baseline = \"first-release\"` for all of them.",
        n = units.len(),
        plural = if units.len() == 1 { "" } else { "s" },
    )
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
            Some(HistoryBoundary::ReleaseTag { commit, .. })
            | Some(HistoryBoundary::PrereleaseTag { commit, .. })
            | Some(HistoryBoundary::Baseline { commit }) => Some(*commit),
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
    /// Decide where each unit's commit window starts, and collect the units
    /// that cannot get a window at all.
    ///
    /// Split out of [`Self::analyze_histories`] so `belaf baseline` can ask
    /// the same question without running a release. It walks tags only — no
    /// revwalk, no diffing — so it is cheap enough to call on its own.
    ///
    /// Precedence, highest first:
    /// 1. the unit's latest matching release tag (stable only, for a
    ///    prerelease unit),
    /// 2. for a prerelease unit with no stable tag: its latest prerelease tag,
    /// 3. the unit's own `baseline` key,
    /// 4. the repo-wide `belaf-baseline` git tag,
    /// 5. nothing — which is a hard stop for a `Deploy` unit in a repo that
    ///    already has version-shaped tags, and simply "analyze from repo
    ///    start" otherwise.
    pub fn resolve_history_boundaries(
        &self,
        projects: &[ResolvedReleaseUnit],
        matchers: &[TagMatcher],
    ) -> Result<HistoryBoundaries> {
        if projects.len() != matchers.len() {
            bail!(
                "internal error: resolve_history_boundaries got {} projects and {} matchers; \
                 lengths must match",
                projects.len(),
                matchers.len()
            );
        }

        let baseline_tag_oid = self.find_baseline_tag()?;
        let repo_has_any_version_tags = self.repo_has_any_version_tags()?;

        let mut boundaries: Vec<Option<HistoryBoundary>> = vec![None; projects.len()];
        let mut untagged: Vec<UntaggedDeployUnit> = Vec::new();

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
                boundaries[i] = Some(HistoryBoundary::ReleaseTag {
                    commit: CommitId(tag_oid),
                    tag_name,
                    version,
                });
            } else if let Some((tag_oid, tag_name, version)) = is_prerelease_unit
                .then(|| self.find_latest_tag_for_project(matcher))
                .transpose()?
                .flatten()
            {
                // A prerelease unit with no stable tag at all — a package kept
                // permanently in beta until its 1.0. Its last prerelease tag is
                // a perfectly good commit boundary; without this the run would
                // fall through to the guard below and one such unit would take
                // the whole repo's release down with it.
                info!(
                    "no stable tag for prerelease unit {}, using last prerelease tag {} (v{})",
                    unit.user_facing_name, tag_name, version
                );
                boundaries[i] = Some(HistoryBoundary::PrereleaseTag {
                    commit: CommitId(tag_oid),
                    tag_name,
                    version,
                });
            } else if let Some(spec) = unit.baseline.as_ref() {
                // Per-unit `baseline` — the reviewed, one-unit-wide form of
                // what `belaf-baseline` does repo-wide. Only consulted after
                // the tag lookups above: a real tag always wins, so leaving
                // the key in place after the first release is harmless.
                match spec {
                    BaselineSpec::FirstRelease => {
                        info!(
                            "no release tag for {}, but `baseline = \"first-release\"` is set — \
                             analyzing from repo start as explicitly accepted",
                            unit.user_facing_name
                        );
                        // boundary stays None; explicitly NOT a violation.
                    }
                    BaselineSpec::Commit(commit_ish) => {
                        let oid =
                            self.resolve_baseline_commit(&unit.user_facing_name, commit_ish)?;
                        info!(
                            "no release tag for {}, using `baseline = \"{}\"` → {}",
                            unit.user_facing_name, commit_ish, oid
                        );
                        boundaries[i] = Some(HistoryBoundary::Baseline {
                            commit: CommitId(oid),
                        });
                    }
                }
            } else if let Some(baseline_oid) = baseline_tag_oid {
                info!(
                    "no release tag for {}, using baseline tag belaf-baseline",
                    unit.user_facing_name
                );
                boundaries[i] = Some(HistoryBoundary::Baseline {
                    commit: CommitId(baseline_oid),
                });
            } else if repo_has_any_version_tags && unit.kind == UnitKind::Deploy {
                // Defensive guard. If the repo already has version-shaped
                // tags but none matched THIS unit's template, falling back to
                // "all commits since repo start" is almost certainly going to
                // inflate the recommended bump.
                //
                // Collected rather than raised: bailing on the first offender
                // makes adoption a whack-a-mole loop, one unit surfaced per
                // run. The caller raises one error naming all of them.
                //
                // F1 — only `Deploy` units count: `Internal`/`Ignore` units have
                // no tags by design. Their own history is never used for
                // candidacy (deploy units collect their commits within the
                // *deploy* unit's tag window via the closure), so analyzing
                // from repo start for them is harmless.
                untagged.push(UntaggedDeployUnit {
                    name: unit.user_facing_name.clone(),
                    config_key: unit
                        .qualified_names()
                        .first()
                        .cloned()
                        .unwrap_or_else(|| unit.user_facing_name.clone()),
                    tag_template: matcher.template().to_string(),
                });
            } else if unit.kind == UnitKind::Deploy {
                warn!(
                    "no release tag or baseline found for {}, and the repo has no version tags at all — analyzing all commits since repo start. This is correct only for a brand-new repo.",
                    unit.user_facing_name
                );
            }
            // else: Internal/Ignore unit with no tag — expected, analyze from
            // repo start silently (boundary stays None).
        }

        Ok(HistoryBoundaries {
            boundaries,
            untagged,
        })
    }

    /// Resolve a `baseline = "<commit-ish>"` value against this repo.
    ///
    /// `revparse_single` accepts short shas, full shas, tags and branch
    /// names. A value that resolves to something that is not a commit (a
    /// blob, say) is as much a config error as one that resolves to nothing,
    /// so both land in the same message.
    fn resolve_baseline_commit(&self, unit_name: &str, commit_ish: &str) -> Result<git2::Oid> {
        self.repo
            .revparse_single(commit_ish)
            .and_then(|obj| obj.peel_to_commit())
            .map(|c| c.id())
            .with_context(|| {
                format!(
                    "release_unit `{unit_name}`: `baseline = \"{commit_ish}\"` does not resolve \
                     to a commit in this repository. Use a commit sha that exists on the \
                     analyzed branch (short shas are fine), or `baseline = \"first-release\"` \
                     to analyze from repo start."
                )
            })
    }

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

        let resolved = self.resolve_history_boundaries(projects, matchers)?;
        if !resolved.untagged.is_empty() {
            return Err(untagged_deploy_units_error(&resolved.untagged));
        }
        for (history, boundary) in histories.iter_mut().zip(resolved.boundaries) {
            history.boundary = boundary;
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
