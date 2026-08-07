//! Release workflow orchestration for PR-based releases.
//!
//! This module implements the [`ReleasePipeline`] which coordinates the complete
//! release workflow:
//!
//! 1. Version bumping across all selected projects
//! 2. Changelog generation using git-cliff style templates
//! 3. Release manifest creation in `belaf/releases/`
//! 4. Git branch management (create, commit, push)
//! 5. GitHub Pull Request creation
//!
//! The workflow is designed for CI/CD environments where releases go through
//! a PR review process before being finalized by a GitHub App.

use anyhow::{Context, Result};
use std::collections::HashMap;
use tracing::info;

use crate::core::{
    bump::{self, BumpConfig, BumpRecommendation},
    changelog::Commit,
    config::syntax::{BumpConfiguration, ChangelogConfiguration},
    ecosystem::format_handler::FormatHandlerRegistry,
    git::repository::RepoPathBuf,
    graph::GraphQueryBuilder,
    group::GroupSet,
    resolved_release_unit::ReleaseUnitId,
    session::AppSession,
    tag_format::{format_tag, split_maven_coords, TagFormatInputs},
    wire::known::Ecosystem,
};

#[derive(Debug, Clone)]
pub struct ReleaseUnitCandidate {
    pub ident: ReleaseUnitId,
    pub name: String,
    pub prefix: String,
    pub current_version: String,
    pub commits: Vec<Commit>,
    pub commit_count: usize,
    pub suggested_bump: BumpRecommendation,
    /// Fully-computed prerelease version (F11b), e.g. `0.6.0-beta.3`, when the
    /// unit has a `prerelease` override. `None` = stable release (apply
    /// `suggested_bump` normally).
    pub prerelease_version: Option<String>,
    pub ecosystem: Ecosystem,
    /// `[cascade_inputs]` provenance: the declared inputs whose changes pulled
    /// this unit into the release, sorted by name. Empty when the unit was
    /// released on its own changes alone. Carried through
    /// [`SelectedReleaseUnit`] into the manifest because manifest emission
    /// only ever sees release candidates — the input nodes themselves are
    /// `Internal` and never appear there.
    pub cascade_inputs: Vec<crate::core::wire::domain::CascadeInputRefWire>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BumpChoice {
    Auto,
    Major,
    Minor,
    Patch,
}

impl BumpChoice {
    pub fn resolve(&self, suggested: BumpRecommendation) -> &'static str {
        match self {
            Self::Auto => suggested.as_str(),
            Self::Major => "major",
            Self::Minor => "minor",
            Self::Patch => "patch",
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Major => "major",
            Self::Minor => "minor",
            Self::Patch => "patch",
        }
    }

    pub fn all() -> Vec<Self> {
        vec![Self::Auto, Self::Major, Self::Minor, Self::Patch]
    }
}

#[derive(Debug, Clone)]
pub struct ReleaseUnitSelection {
    pub candidate: ReleaseUnitCandidate,
    pub bump_choice: BumpChoice,
    pub cached_changelog: Option<String>,
}

type ChangelogGenerationResult = (
    Vec<RepoPathBuf>,
    HashMap<String, String>,
    HashMap<String, Vec<Commit>>,
);

/// F11b — compute a prerelease version (`<base>-<label>.<N>`).
///
/// `stable` is the last stable release version (the prerelease unit's
/// boundary); `level` is the bump computed from commits since stable; `current`
/// is the unit's current version (possibly already a prerelease of this
/// base/label). The counter resets to 1 when the base or label changes,
/// otherwise increments. Semver-only — non-semver current versions error.
fn compute_prerelease_version(
    label: &str,
    stable: Option<&semver::Version>,
    level: BumpRecommendation,
    current: &str,
) -> Result<String> {
    // Validate the label is a usable semver prerelease identifier.
    semver::Prerelease::new(&format!("{label}.1"))
        .with_context(|| format!("invalid prerelease label `{label}`"))?;

    let current_sv = semver::Version::parse(current).with_context(|| {
        format!("prerelease requires a semver version; `{current}` is not semver")
    })?;
    let current_base = semver::Version::new(current_sv.major, current_sv.minor, current_sv.patch);

    let new_base = match stable {
        // Anchored on the last stable release, so changes accumulated across
        // several betas are counted once.
        Some(stable) => match level {
            BumpRecommendation::Major => semver::Version::new(stable.major + 1, 0, 0),
            BumpRecommendation::Minor => semver::Version::new(stable.major, stable.minor + 1, 0),
            // `None` shouldn't reach here (F5 floors to patch); treat as patch.
            BumpRecommendation::Patch | BumpRecommendation::None => {
                semver::Version::new(stable.major, stable.minor, stable.patch + 1)
            }
        },
        // No stable release has ever shipped — a package kept permanently in
        // beta. There is nothing to bump *from*: `0.6.0-beta.N` already means
        // "heading for 0.6.0", so further commits before 0.6.0 ships move the
        // counter, not the target. Bumping here instead would walk the base
        // forward on every run and reset the counter each time.
        //
        // Mirrors release-please's prerelease strategy: hold the base while
        // the components below the bump level are still zero, and only break
        // out when they are not.
        None => match level {
            BumpRecommendation::Patch | BumpRecommendation::None => current_base.clone(),
            BumpRecommendation::Minor if current_base.major == 0 && current_base.patch == 0 => {
                current_base.clone()
            }
            BumpRecommendation::Minor => {
                semver::Version::new(current_base.major, current_base.minor + 1, 0)
            }
            BumpRecommendation::Major if current_base.minor == 0 && current_base.patch == 0 => {
                current_base.clone()
            }
            BumpRecommendation::Major => semver::Version::new(current_base.major + 1, 0, 0),
        },
    };

    // Counter: continue iff current is a prerelease of the same base + label.
    let counter = if current_base == new_base && !current_sv.pre.is_empty() {
        prerelease_counter(current_sv.pre.as_str(), label)
            .map(|n| n + 1)
            .unwrap_or(1)
    } else {
        1
    };

    Ok(format!(
        "{}.{}.{}-{label}.{counter}",
        new_base.major, new_base.minor, new_base.patch
    ))
}

/// Extract the numeric counter from a `<label>.<N>` prerelease string.
fn prerelease_counter(pre: &str, label: &str) -> Option<u64> {
    pre.strip_prefix(label)?.strip_prefix('.')?.parse().ok()
}

/// The outcome of a completed `prepare` run.
pub struct PreparedRelease {
    pub pr_url: String,
    /// Whether the run opened the release PR or refreshed an existing one.
    pub pr_action: crate::core::github::client::PrAction,
}

pub struct PrepareContext<'a> {
    pub sess: &'a mut AppSession,
    pub base_branch: String,
    pub release_branch: String,
    pub candidates: Vec<ReleaseUnitCandidate>,
    pub allow_dirty: bool,
    pub changelog_config: ChangelogConfiguration,
    pub bump_config: BumpConfiguration,
}

impl<'a> PrepareContext<'a> {
    pub fn initialize(sess: &'a mut AppSession, allow_dirty: bool) -> Result<Self> {
        if !allow_dirty {
            if let Some(dirty) = sess
                .repo
                .check_if_dirty(&[])
                .context("failed to check repository for modified files")?
            {
                return Err(anyhow::anyhow!(
                    "requires a clean working directory. Found uncommitted changes: {}",
                    dirty.escaped()
                ));
            }
        } else if let Some(dirty) = sess
            .repo
            .check_if_dirty(&[])
            .context("failed to check repository for modified files")?
        {
            info!(
                "preparing release with uncommitted changes in the repository (e.g.: `{}`)",
                dirty.escaped()
            );
        }

        let (base_branch, release_branch) = create_release_branch(sess, false)?;
        let changelog_config = sess.changelog_config.clone();
        let bump_config = sess.bump_config.clone();

        Ok(Self {
            sess,
            base_branch,
            release_branch,
            candidates: Vec::new(),
            allow_dirty,
            changelog_config,
            bump_config,
        })
    }

    pub fn resolve_workdir(
        &self,
        path: &crate::core::git::repository::RepoPath,
    ) -> std::path::PathBuf {
        self.sess.repo.resolve_workdir(path)
    }

    pub fn discover_projects(&mut self) -> Result<()> {
        let q = GraphQueryBuilder::default();
        let idents = self
            .sess
            .graph()
            .query(q)
            .context("could not select projects")?;

        if idents.is_empty() {
            info!("no projects found in repository");
            return Ok(());
        }

        let histories = self
            .sess
            .analyze_histories()
            .context("failed to analyze project histories")?;

        for ident in &idents {
            let unit = self.sess.graph().lookup(*ident);

            // F1 — only `deploy` units are release candidates. `internal`
            // (cascade-only) and `ignore` units are never versioned/tagged/
            // released; they participate in the graph for the closure cascade
            // (WS-3) but must never produce a manifest entry.
            if unit.kind != crate::core::resolved_release_unit::UnitKind::Deploy {
                continue;
            }

            let history = histories.lookup(*ident);
            let n_commits = history.n_commits();

            if n_commits == 0 {
                info!(
                    "{}: no changes since last release, skipping",
                    unit.user_facing_name
                );
                continue;
            }

            let commits: Vec<Commit> = history
                .commits()
                .into_iter()
                .filter_map(|cid| {
                    let mut commit = self.sess.repo.get_commit_details(*cid).ok()?;
                    // F7 — annotate closure-propagated commits with the internal
                    // crate they came from, for the `via <crate>` changelog prefix.
                    if let Some(member) = history.provenance_for(*cid) {
                        commit.via =
                            Some(self.sess.graph().lookup(member).user_facing_name.clone());
                    }
                    Some(commit)
                })
                .collect();

            let current_version = unit.version.to_string();

            let analysis = bump::analyze_commits(&commits).with_context(|| {
                format!(
                    "failed to analyze commit messages for {}",
                    unit.user_facing_name
                )
            })?;

            // F11a — effective bump policy = global `[bump]` ⊕ the per-unit
            // `[release_unit.<name>.bump]` override (field-wise, per-unit wins).
            let mut bump_config = BumpConfig::from_user_config(&self.bump_config);
            let mut prerelease: Option<String> = None;
            if let Some(ov) = &unit.bump_override {
                if let Some(v) = ov.features_always_bump_minor {
                    bump_config.features_always_bump_minor = v;
                }
                if let Some(v) = ov.breaking_always_bump_major {
                    bump_config.breaking_always_bump_major = v;
                }
                prerelease = ov.prerelease.clone();
            }
            // F5 — every commit collected here is binary-affecting (the closure
            // collection only keeps path-hits through the binary filter), so a
            // unit that reached this point must ship at least a patch even if no
            // commit carried a bumpable type (all `chore`/`refactor`). Floor
            // before `apply_config` so the pre-1.0 downgrade can't drop it.
            let mut suggested_bump = analysis
                .recommendation
                .with_patch_floor()
                .apply_config(&bump_config, Some(&current_version));

            // `[cascade_inputs]` — raise the level to the highest floor any
            // contributing input declares.
            //
            // Note this can only *raise* an existing bump, never create one:
            // `input_hits` is filled from the commits the unit collected, and a
            // unit with zero commits was already skipped above. A declared
            // input therefore cannot resurrect an untouched unit.
            //
            // Also note `mirror` and `floor_patch` are no-ops in practice: the
            // F5 patch floor two lines up already lifts any collected commit to
            // at least Patch. The setting only changes anything from
            // `floor_minor` upwards.
            let cascade_inputs = self.collect_cascade_input_refs(history, &mut suggested_bump);

            // F11a — cap the level at `max_bump` (never exceed it, even on
            // feat/breaking). Combined with the F5 floor → interval [patch, cap].
            if let Some(max) = unit
                .bump_override
                .as_ref()
                .and_then(|o| o.max_bump.as_deref())
            {
                if let Some(cap) = BumpRecommendation::from_string(max) {
                    suggested_bump = suggested_bump.cap_at(cap);
                }
            }

            // F11b — compute the prerelease version (`<base>-<label>.<N>`). The
            // base comes from the last STABLE tag (the unit's boundary is the
            // last stable tag for prerelease units, see analyze_histories), so
            // changes accumulated across betas are counted once; the counter
            // continues iff the current version is already a prerelease of the
            // same base + label.
            let prerelease_version = match &prerelease {
                Some(label) => Some(compute_prerelease_version(
                    label,
                    history.release_version(),
                    suggested_bump,
                    &current_version,
                )?),
                None => None,
            };

            info!("{}: {}", unit.user_facing_name, analysis.summary());

            let qnames = unit.qualified_names();
            let ecosystem = qnames
                .get(1)
                .map(|s| Ecosystem::classify(s))
                .unwrap_or_else(|| Ecosystem::classify("cargo"));

            self.candidates.push(ReleaseUnitCandidate {
                ident: *ident,
                name: unit.user_facing_name.clone(),
                prefix: unit.prefix().escaped(),
                current_version,
                commits,
                commit_count: n_commits,
                suggested_bump,
                prerelease_version,
                ecosystem,
                cascade_inputs,
            });
        }

        Ok(())
    }

    /// Resolve the `[cascade_inputs]` nodes that contributed to a unit's
    /// commits into manifest-ready references, raising `suggested_bump` to the
    /// highest floor they declare.
    ///
    /// Runs *before* the `max_bump` cap so an explicit per-unit cap still wins
    /// over an input's floor. Costs nothing when no inputs are configured —
    /// `input_hits` is then empty for every unit.
    fn collect_cascade_input_refs(
        &self,
        history: &crate::core::git::repository::RepoHistory,
        suggested_bump: &mut BumpRecommendation,
    ) -> Vec<crate::core::wire::domain::CascadeInputRefWire> {
        use crate::core::release_unit::cascade::{cascaded, BumpKind};
        use crate::core::release_unit::CascadeBumpStrategy;

        let mut refs: Vec<crate::core::wire::domain::CascadeInputRefWire> = Vec::new();

        for input_id in history.input_hits() {
            let name = self
                .sess
                .graph()
                .lookup(input_id)
                .user_facing_name
                .to_string();
            // Node names equal the config key (collisions are rejected at
            // graph build), so this lookup always resolves in practice.
            let Some(cfg) = self.sess.cascade_inputs().iter().find(|c| c.name == name) else {
                continue;
            };

            let floor = cascaded(
                cfg.bump.unwrap_or(CascadeBumpStrategy::Mirror),
                BumpKind::from_recommendation(*suggested_bump),
            )
            .to_recommendation();
            *suggested_bump = suggested_bump.merge(floor);

            refs.push(crate::core::wire::domain::CascadeInputRefWire {
                name,
                bump: cfg.bump.map(|b| b.wire_key().to_string()),
            });
        }

        refs.sort_by(|a, b| a.name.cmp(&b.name));
        refs
    }

    pub fn has_candidates(&self) -> bool {
        !self.candidates.is_empty()
    }

    pub fn cleanup(self) {
        cleanup_release_branch(self.sess, &self.base_branch, &self.release_branch);
    }

    /// The branch this run will push to.
    pub fn release_branch(&self) -> &str {
        &self.release_branch
    }

    /// The branch this run started from, and the base of its pull request.
    pub fn base_branch(&self) -> &str {
        &self.base_branch
    }

    /// Switch from the repo's stable release branch to a throwaway
    /// timestamped one, so this run opens a release PR of its own instead of
    /// updating the open one.
    ///
    /// Safe to call any time before `finalize`: the branch created during
    /// initialization sits on the base commit and carries no work yet.
    pub fn use_separate_branch(&mut self) -> Result<()> {
        let previous = std::mem::take(&mut self.release_branch);
        let (_, separate) = create_release_branch(self.sess, true)?;
        self.release_branch = separate;

        // The stable branch was reset to the base commit during
        // initialization and holds nothing we need. Only local state — the
        // remote branch and its open PR are untouched.
        if let Err(e) = self.sess.repo.delete_branch(&previous) {
            tracing::warn!(
                "failed to remove unused release branch '{}': {}",
                previous,
                e
            );
        }

        Ok(())
    }

    pub fn finalize(self, selections: Vec<ReleaseUnitSelection>) -> Result<PreparedRelease> {
        if selections.is_empty() {
            return Err(anyhow::anyhow!("no projects selected for release"));
        }

        let mut prepared: Vec<SelectedReleaseUnit> = Vec::new();

        for selection in &selections {
            let unit = self.sess.graph().lookup(selection.candidate.ident);

            let bump_scheme_text = selection
                .bump_choice
                .resolve(selection.candidate.suggested_bump);

            if bump_scheme_text == "no bump" {
                info!("{}: no version bump needed", unit.user_facing_name);
                continue;
            }

            let bump_scheme = unit
                .version
                .parse_bump_scheme(bump_scheme_text)
                .with_context(|| {
                    format!(
                        "invalid bump scheme \"{}\" for project {}",
                        bump_scheme_text, unit.user_facing_name
                    )
                })?;

            let old_version = selection.candidate.current_version.clone();

            let proj_mut = self.sess.graph_mut().lookup_mut(selection.candidate.ident);

            if let Some(pre_ver) = &selection.candidate.prerelease_version {
                // F11b — apply the precomputed prerelease version directly.
                let new = proj_mut.version.parse_like(pre_ver).with_context(|| {
                    format!(
                        "invalid prerelease version `{}` for {}",
                        pre_ver, proj_mut.user_facing_name
                    )
                })?;
                // Monotonicity guard (per-ecosystem ordering, incl. prerelease):
                // the new version must be strictly greater than the current one.
                if new.partial_cmp(&proj_mut.version) != Some(std::cmp::Ordering::Greater) {
                    return Err(anyhow::anyhow!(
                        "prerelease version `{}` for {} is not greater than current `{}` \
                         — check the prerelease label/counter",
                        pre_ver,
                        proj_mut.user_facing_name,
                        old_version
                    ));
                }
                proj_mut.version = new;
            } else {
                bump_scheme.apply(&mut proj_mut.version).with_context(|| {
                    format!(
                        "failed to apply version bump to {}",
                        proj_mut.user_facing_name
                    )
                })?;
            }

            // F7/F11b — compute prerelease from the STRUCTURED version (not a
            // string-marker check), so arbitrary labels classify correctly.
            let is_prerelease = proj_mut.version.is_prerelease();
            let new_version = proj_mut.version.to_string();

            info!(
                "{}: {} -> {} ({} commit{})",
                proj_mut.user_facing_name,
                old_version,
                new_version,
                selection.candidate.commit_count,
                if selection.candidate.commit_count == 1 {
                    ""
                } else {
                    "s"
                }
            );

            prepared.push(SelectedReleaseUnit {
                ident: selection.candidate.ident,
                name: proj_mut.user_facing_name.clone(),
                prefix: selection.candidate.prefix.clone(),
                old_version,
                new_version,
                bump_type: bump_scheme_text.to_string(),
                is_prerelease,
                commits: selection.candidate.commits.clone(),
                ecosystem: selection.candidate.ecosystem.clone(),
                cached_changelog: selection.cached_changelog.clone(),
                cascade_inputs: selection.candidate.cascade_inputs.clone(),
            });
        }

        if prepared.is_empty() {
            return Err(anyhow::anyhow!("no projects needed version bumps"));
        }

        let pipeline = ReleasePipeline::new(self.sess, self.base_branch, self.release_branch)?;
        pipeline.execute(prepared)
    }
}

#[derive(Debug, Clone)]
pub struct SelectedReleaseUnit {
    pub ident: ReleaseUnitId,
    pub name: String,
    pub prefix: String,
    pub old_version: String,
    pub new_version: String,
    pub bump_type: String,
    /// Whether `new_version` carries a prerelease marker, computed from the
    /// structured version (F7/F11b) rather than a string-marker heuristic.
    pub is_prerelease: bool,
    pub commits: Vec<Commit>,
    pub ecosystem: Ecosystem,
    pub cached_changelog: Option<String>,
    /// See [`ReleaseUnitCandidate::cascade_inputs`].
    pub cascade_inputs: Vec<crate::core::wire::domain::CascadeInputRefWire>,
}

/// Resolve the per-release tag name using the precedence chain:
/// `[release_unit.<name>].tag_format` > `[group.<id>].tag_format` > the
/// ecosystem trait's `tag_format_default()`. The ecosystem registry
/// is instantiated fresh here
/// (the Loader instances are stateless once `finalize` has run).
fn build_tag_name(
    sess: &AppSession,
    project: &SelectedReleaseUnit,
    groups: &GroupSet,
) -> Result<String> {
    let registry = FormatHandlerRegistry::with_defaults();
    let eco_name = project.ecosystem.as_str();

    // Bundle / synthetic ecosystems (`tauri`, `hexagonal-cargo`,
    // `jvm-library`) aren't `FormatHandler`-backed — they're
    // taxonomy labels on configured `[release_unit.X]` blocks.
    // For those we fall back to a generic tag template; the unit's
    // own `tag_format` override (set by auto-detect or by the user)
    // covers customisation, the default is a sensible last resort.
    let (eco_default_tag, eco_allowed_vars): (&'static str, &'static [&'static str]) =
        match registry.lookup(eco_name) {
            Some(h) => (h.tag_format_default(), h.tag_template_vars()),
            None => ("{name}@v{version}", &["name", "version", "ecosystem"]),
        };

    // tag-format precedence: explicit [release_unit.<name>] > [group.<id>]
    // > ecosystem default.
    let unit_override = sess
        .resolved_release_units()
        .iter()
        .find(|r| r.unit.name == project.name)
        .and_then(|r| r.unit.tag_format.as_deref());
    let group_override = groups
        .group_of(project.ident)
        .and_then(|g| g.tag_format.as_deref());
    let template = unit_override.or(group_override);

    let maven_coords = if eco_name == "maven" {
        split_maven_coords(&project.name)
    } else {
        None
    };

    let inputs = TagFormatInputs {
        project_name: &project.name,
        version: &project.new_version,
        ecosystem: eco_name,
        ecosystem_default: eco_default_tag,
        allowed_vars: eco_allowed_vars,
        override_template: template,
        maven_coords,
        module_path: if eco_name == "go" {
            Some(&project.name)
        } else {
            None
        },
    };
    format_tag(&inputs)
}

mod branch;
mod changelog_gen;
mod github;
mod pipeline;

pub use branch::{cleanup_release_branch, create_release_branch};
pub use changelog_gen::{
    generate_and_write_project_changelog, generate_changelog_entry, ChangelogGenerationParams,
    ChangelogResult,
};
pub use github::{extract_github_remote, load_github_token, GitHubRemoteInfo};
pub use pipeline::ReleasePipeline;

#[cfg(test)]
mod prerelease_tests {
    use super::{compute_prerelease_version, prerelease_counter};
    use crate::core::bump::BumpRecommendation;

    fn sv(s: &str) -> semver::Version {
        semver::Version::parse(s).unwrap()
    }

    // --- no stable release has ever shipped (permanent beta) ---------------
    //
    // There is nothing to bump from, so the base holds and the counter moves.
    // Mirrors release-please's prerelease strategy; without it the base walked
    // forward every run and the counter reset to 1 each time.

    #[test]
    fn no_stable_patch_holds_the_base() {
        let v = compute_prerelease_version("beta", None, BumpRecommendation::Patch, "0.6.0-beta.3")
            .unwrap();
        assert_eq!(v, "0.6.0-beta.4");
    }

    #[test]
    fn no_stable_minor_holds_the_base_while_pre_major() {
        // 0.x with patch == 0: the base already represents an unreleased
        // minor, so a feature does not move it again.
        let v = compute_prerelease_version("beta", None, BumpRecommendation::Minor, "0.6.0-beta.3")
            .unwrap();
        assert_eq!(v, "0.6.0-beta.4");
    }

    #[test]
    fn no_stable_minor_bumps_once_patch_is_nonzero() {
        // 0.6.1 is a patch line; a feature has to move to 0.7.0.
        let v = compute_prerelease_version("beta", None, BumpRecommendation::Minor, "0.6.1-beta.2")
            .unwrap();
        assert_eq!(v, "0.7.0-beta.1");
    }

    #[test]
    fn no_stable_major_breaks_out_when_minor_is_nonzero() {
        // A real break is the one thing that still shows up in the number.
        let v = compute_prerelease_version("beta", None, BumpRecommendation::Major, "0.6.0-beta.3")
            .unwrap();
        assert_eq!(v, "1.0.0-beta.1");
    }

    #[test]
    fn no_stable_major_holds_the_base_at_x_0_0() {
        // 1.0.0-beta.N is already the breaking release being prepared.
        let v = compute_prerelease_version("beta", None, BumpRecommendation::Major, "1.0.0-beta.2")
            .unwrap();
        assert_eq!(v, "1.0.0-beta.3");
    }

    #[test]
    fn no_stable_first_beta_from_a_plain_version() {
        // Never tagged at all: 0.6.0 in the manifest, no prerelease suffix.
        let v =
            compute_prerelease_version("beta", None, BumpRecommendation::Patch, "0.6.0").unwrap();
        assert_eq!(v, "0.6.0-beta.1");
    }

    #[test]
    fn first_beta_from_stable() {
        // F11b — base from the last stable tag + level; counter starts at 1.
        let v = compute_prerelease_version(
            "beta",
            Some(&sv("0.5.0")),
            BumpRecommendation::Minor,
            "0.5.0",
        )
        .unwrap();
        assert_eq!(v, "0.6.0-beta.1");
    }

    #[test]
    fn counter_continues_same_base_and_label() {
        // base 0.6.0 unchanged + same label → increment counter.
        let v = compute_prerelease_version(
            "beta",
            Some(&sv("0.5.0")),
            BumpRecommendation::Minor,
            "0.6.0-beta.2",
        )
        .unwrap();
        assert_eq!(v, "0.6.0-beta.3");
    }

    #[test]
    fn counter_resets_on_base_rise() {
        // A breaking change raises the base (1.x) → counter resets to 1.
        let v = compute_prerelease_version(
            "beta",
            Some(&sv("1.1.0")),
            BumpRecommendation::Major,
            "1.1.0-beta.4",
        )
        .unwrap();
        assert_eq!(v, "2.0.0-beta.1");
    }

    #[test]
    fn counter_resets_on_label_change() {
        // alpha → beta keeps the base but resets the counter (not alpha's).
        let v = compute_prerelease_version(
            "beta",
            Some(&sv("0.5.0")),
            BumpRecommendation::Minor,
            "0.6.0-alpha.2",
        )
        .unwrap();
        assert_eq!(v, "0.6.0-beta.1");
    }

    #[test]
    fn non_semver_current_errors() {
        assert!(
            compute_prerelease_version("beta", None, BumpRecommendation::Patch, "1.0.0a1").is_err()
        );
    }

    #[test]
    fn counter_parsing() {
        assert_eq!(prerelease_counter("beta.3", "beta"), Some(3));
        assert_eq!(prerelease_counter("alpha.1", "beta"), None);
        assert_eq!(prerelease_counter("beta", "beta"), None);
    }
}
