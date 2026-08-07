// Copyright 2020 Peter Williams <peter@newton.cx> and collaborators
// Licensed under the MIT License.

//! State for the Belaf CLI application.

mod builder;
mod graph_build;

pub use builder::AppBuilder;

use anyhow::{anyhow, Context};
use std::collections::HashMap;
use thiserror::Error as ThisError;
use tracing::{error, info, warn};

use crate::{
    atry,
    core::{
        config::syntax::ChangelogConfiguration,
        ecosystem::format_handler::FormatHandlerRegistry,
        errors::Result,
        git::repository::{ChangeList, ReleaseAvailability, Repository},
        graph::{ReleaseUnitGraph, RepoHistories},
        group::GroupSet,
        resolved_release_unit::{DepRequirement, ReleaseUnitId, ResolvedReleaseUnit},
        tag_format::{build_tag_matcher, split_maven_coords, TagMatcher, TagPatternInputs},
        version::Version,
    },
};

/// Build a [`TagMatcher`] for one project, honouring the same
/// precedence chain as [`crate::core::workflow::build_tag_name`] —
/// `[release_unit.<name>].tag_format` > `[group.<id>].tag_format` >
/// the ecosystem trait's `tag_format_default()`. The matcher is the
/// inverse of the tag the github-app would create on PR-merge, so any
/// previously-published tag will be recognised.
///
/// Used by every place that walks `git tag` looking for "this
/// project's last release" — `analyze_histories`,
/// `find_earliest_release_containing`, the initial `resolve_versions_from_tags`
/// pass during graph build.
fn build_tag_matcher_for(
    project_name: &str,
    ecosystem_name: &str,
    tag_format_override: Option<&str>,
    group_tag_format: Option<&str>,
    registry: &FormatHandlerRegistry,
    allow_bare_v_fallback: bool,
) -> Result<TagMatcher> {
    let template_override = tag_format_override.or(group_tag_format);
    let (eco_default_tag, eco_allowed_vars): (&'static str, &'static [&'static str]) =
        match registry.lookup(ecosystem_name) {
            Some(h) => (h.tag_format_default(), h.tag_template_vars()),
            None => ("{name}@v{version}", &["name", "version", "ecosystem"]),
        };
    let maven_coords = if ecosystem_name == "maven" {
        split_maven_coords(project_name)
    } else {
        None
    };
    let module_path = if ecosystem_name == "go" {
        Some(project_name)
    } else {
        None
    };
    let inputs = TagPatternInputs {
        project_name,
        ecosystem: ecosystem_name,
        ecosystem_default: eco_default_tag,
        allowed_vars: eco_allowed_vars,
        override_template: template_override,
        maven_coords,
        module_path,
        allow_bare_v_fallback,
    };
    build_tag_matcher(&inputs)
}

/// Build a matcher per project for the *runtime* graph — used by
/// `AppSession::analyze_histories` and by `find_earliest_release_containing`.
fn build_matchers_for_runtime_units(
    units: &[&ResolvedReleaseUnit],
    cfg_units: &[crate::core::release_unit::ResolvedReleaseUnit],
    groups: &GroupSet,
    registry: &FormatHandlerRegistry,
) -> Result<Vec<TagMatcher>> {
    let allow_bare_v_fallback = units.len() == 1;
    units
        .iter()
        .map(|unit| {
            let project_name = &unit.user_facing_name;
            // qnames[1] is the ecosystem string by convention (per
            // FormatHandler::name docs). Fall back to "cargo" for the
            // legacy single-qname case to preserve the previous
            // bare-v matching behaviour.
            let ecosystem_name = unit
                .qualified_names()
                .get(1)
                .cloned()
                .unwrap_or_else(|| "cargo".to_string());
            let tag_format_override = cfg_units
                .iter()
                .find(|r| r.unit.name == *project_name)
                .and_then(|r| r.unit.tag_format.as_deref())
                .map(|s| s.to_string());
            let group_tag_format = groups
                .group_of(unit.ident())
                .and_then(|g| g.tag_format.clone());
            build_tag_matcher_for(
                project_name,
                &ecosystem_name,
                tag_format_override.as_deref(),
                group_tag_format.as_deref(),
                registry,
                allow_bare_v_fallback,
            )
        })
        .collect()
}

#[derive(Clone, Debug, Default)]
pub struct NpmConfig {
    pub internal_dep_protocol: Option<String>,
}

/// An error returned when one project in the repository needs a newer release
/// of another project. The inner values are the user-facing names of the two
/// projects: the first named project depends on the second one.
#[derive(Debug, ThisError)]
#[error("unsatisfied internal requirement: `{0}` needs newer `{1}`")]
pub struct UnsatisfiedInternalRequirementError(pub String, pub String);

pub struct AppSession {
    pub repo: Repository,
    pub npm_config: NpmConfig,
    pub changelog_config: ChangelogConfiguration,
    pub bump_config: super::config::syntax::BumpConfiguration,
    /// `[commit_attribution]` — scope-matching config consumed when
    /// attributing commits to units in [`Self::analyze_histories`]. Held
    /// so the `ScopeMatcher` is built from the user's settings instead of
    /// the hardcoded default (F10).
    commit_attribution: super::config::syntax::CommitAttributionConfiguration,
    /// `[binary_affecting]` — which changed paths count toward a bump (F3).
    /// Consumed by `analyze_histories`.
    binary_affecting: super::config::syntax::BinaryAffectingConfiguration,
    /// `[cascade_inputs.<name>]` — declared path inputs, already validated and
    /// sorted. Held so the bump-floor step in
    /// [`crate::core::workflow::PrepareContext::discover_projects`] can map a
    /// contributing input node back to its declared `bump` strategy.
    cascade_inputs: Vec<super::config::ResolvedCascadeInput>,
    /// `[[bump_source]]` entries from `belaf/config.toml`. Resolved at
    /// CI/wizard entry by [`crate::cmd::prepare`].
    bump_sources: Vec<super::config::syntax::BumpSourceConfig>,
    /// Resolved `[release_unit.<name>]` / glob-form `[release_unit.<name>]` entries.
    /// Held so [`Self::pre_prepare_drift_check`] can compare detected
    /// bundles against the configured coverage set without re-running
    /// the resolver.
    resolved_release_units: Vec<crate::core::release_unit::ResolvedReleaseUnit>,
    /// `[ignore_paths] paths` from `belaf/config.toml` — paths the
    /// drift check should silence even though a detector matches.
    ignore_paths: Vec<String>,
    /// `[allow_uncovered] paths` from `belaf/config.toml` — explicit
    /// "yes I see this is uncovered, leave it alone" list.
    allow_uncovered: Vec<String>,
    /// Cached output of [`crate::core::release_unit::detector::detect_all`]
    /// — first call materialises it, subsequent calls reuse. Avoids
    /// the full filesystem walk on every `belaf prepare` invocation
    /// (the wizard + drift-check would otherwise traverse the same
    /// tree twice).
    detection_cache: std::sync::OnceLock<crate::core::release_unit::detector::DetectionReport>,
    graph: ReleaseUnitGraph,
    is_ci: bool,
}

impl AppSession {
    /// Create a new app session with totally default parameters
    pub fn initialize_default() -> Result<Self> {
        AppBuilder::new()?.initialize()
    }

    pub fn execution_environment(&self) -> Result<ExecutionEnvironment> {
        if self.is_ci {
            Ok(ExecutionEnvironment::Ci)
        } else {
            Ok(ExecutionEnvironment::NotCi)
        }
    }

    /// The parsed `[commit_attribution]` config — consumed by `belaf check`
    /// (F8) to build the scope matcher for commit-label validation.
    pub fn commit_attribution(&self) -> &super::config::syntax::CommitAttributionConfiguration {
        &self.commit_attribution
    }

    /// The parsed `[binary_affecting]` config (F3) — exposed so `belaf check`
    /// can ignore non-binary-affecting paths when validating scope/path
    /// consistency.
    pub fn binary_affecting(&self) -> &super::config::syntax::BinaryAffectingConfiguration {
        &self.binary_affecting
    }

    /// The validated `[cascade_inputs.<name>]` blocks, sorted by name.
    pub fn cascade_inputs(&self) -> &[super::config::ResolvedCascadeInput] {
        &self.cascade_inputs
    }

    /// Check that the current process is running *outside* of a CI environment.
    pub fn ensure_not_ci(&self, force: bool) -> Result<()> {
        match self.execution_environment()? {
            ExecutionEnvironment::NotCi => Ok(()),

            _ => {
                warn!("CI environment detected; this is unexpected for this command");
                if force {
                    Ok(())
                } else {
                    Err(anyhow!(
                        "refusing to proceed (use \"force\" mode to override)",
                    ))
                }
            }
        }
    }

    /// Check that the working tree is completely clean. We allow untracked and
    /// ignored files but otherwise don't want any modifications, etc. Returns
    /// Ok if clean, an Err downcastable to DirtyRepositoryError if not. The
    /// error may have a different cause if, e.g., there is an I/O failure.
    pub fn ensure_fully_clean(&self) -> Result<()> {
        use crate::core::git::repository::DirtyRepositoryError;

        if let Some(changed_path) = self.repo.check_if_dirty(&[])? {
            Err(DirtyRepositoryError(changed_path).into())
        } else {
            Ok(())
        }
    }

    /// Get the graph of projects inside this app session.
    /// `[[bump_source]]` entries declared in `belaf/config.toml`.
    pub fn config_bump_sources(&self) -> &[super::config::syntax::BumpSourceConfig] {
        &self.bump_sources
    }

    /// Resolved `[release_unit.<name>]` / glob-form `[release_unit.<name>]` entries.
    pub fn resolved_release_units(&self) -> &[crate::core::release_unit::ResolvedReleaseUnit] {
        &self.resolved_release_units
    }

    /// Phase H — run the drift detector against the working tree and
    /// return an `Err(message)` when an uncovered detector hit exists.
    /// Wired into [`crate::cmd::prepare::run`] so every prepare run
    /// (CI or interactive) catches new bundles that aren't claimed by
    /// any `[release_unit.<name>]` / `[ignore_paths]` / `[allow_uncovered]`.
    /// Reuses [`Self::detection_report`] to avoid walking the
    /// filesystem twice within one process.
    pub fn pre_prepare_drift_check(&self) -> std::result::Result<(), String> {
        let report = self.detection_report();
        let drift = crate::core::release_unit::detector::detect_drift_from_report(
            report,
            &self.resolved_release_units,
            &self.ignore_paths,
            &self.allow_uncovered,
        );
        if drift.is_empty() {
            Ok(())
        } else {
            Err(drift.format_error())
        }
    }

    /// Compute the current uncovered-path list. Returns `[]` when the
    /// drift detector finds nothing — distinct from `pre_prepare_drift_check`
    /// in that it never errors. Used by the CLI to telemetry-report
    /// drift state to the dashboard regardless of pass/fail.
    pub fn drift_uncovered_paths(&self) -> Vec<String> {
        let report = self.detection_report();
        let drift = crate::core::release_unit::detector::detect_drift_from_report(
            report,
            &self.resolved_release_units,
            &self.ignore_paths,
            &self.allow_uncovered,
        );
        drift
            .uncovered
            .iter()
            .map(|h| h.path.escaped().to_string())
            .collect()
    }

    /// Cached [`detect_all`](crate::core::release_unit::detector::detect_all)
    /// output. The first caller pays the filesystem-walk cost; the
    /// rest reuse the materialised report.
    pub fn detection_report(&self) -> &crate::core::release_unit::detector::DetectionReport {
        self.detection_cache
            .get_or_init(|| crate::core::release_unit::detector::detect_all(&self.repo))
    }

    pub fn graph(&self) -> &ReleaseUnitGraph {
        &self.graph
    }

    /// Get the graph of projects inside this app session, mutably.
    pub fn graph_mut(&mut self) -> &mut ReleaseUnitGraph {
        &mut self.graph
    }

    /// Walk the project graph and solve internal dependencies.
    ///
    /// This method walks the graph in topologically-sorted order. For each
    /// project, the callback `process` is called, which should return true if a
    /// new release of the project is being scheduled. By the time the callback
    /// is called, the project's internal dependency information will have been
    /// updated: for DepRequirement::Commit deps, `resolved_version` will be a
    /// Some value containing the required version. It is possible that this
    /// version will be being released "right now".
    ///
    /// By the time the callback returns, the project's `version` field should
    /// have been updated with its reference version for this release process --
    /// which should be a new value, if the callback returns true.
    ///
    /// After processing all projects, the function will return an error if
    /// there are unsatisfiable internal dependencies. This can happen either
    /// because no sufficiently new release of the dependee exists (and it's not
    /// being released now), or the internal version requirement information
    /// hasn't been annotated.
    pub fn solve_internal_deps<F>(&mut self, mut process: F) -> Result<()>
    where
        F: FnMut(&mut Repository, &mut ReleaseUnitGraph, ReleaseUnitId) -> Result<bool>,
    {
        let mut new_versions: HashMap<ReleaseUnitId, Version> = HashMap::new();
        let toposorted_idents: Vec<_> = self.graph.toposorted().collect();
        let mut unsatisfied_deps = Vec::new();

        for ident in (toposorted_idents[..]).iter().copied() {
            // We can't conveniently navigate the deps while holding a mutable
            // ref to depending project, so do some lifetime futzing and buffer
            // up modifications to its dep info.

            unsatisfied_deps.clear();

            let mut resolved_versions = {
                let unit = self.graph.lookup(ident);
                let mut resolved_versions = Vec::new();

                for (idx, dep) in unit.internal_deps.iter().enumerate() {
                    match dep.belaf_requirement {
                        // If the requirement is of a specific commit, we need
                        // to resolve its corresponding release and/or make sure
                        // that the dependee project is also being released in
                        // this batch.
                        DepRequirement::Commit(ref cid) => {
                            let dependee_proj = self.graph.lookup(dep.ident);
                            let registry = FormatHandlerRegistry::with_defaults();
                            let matchers = build_matchers_for_runtime_units(
                                &[dependee_proj],
                                &self.resolved_release_units,
                                self.graph.groups(),
                                &registry,
                            )?;
                            let avail = self.repo.find_earliest_release_containing(
                                dependee_proj,
                                &matchers[0],
                                cid,
                            )?;

                            let resolved = match avail {
                                ReleaseAvailability::NotAvailable => {
                                    unsatisfied_deps
                                        .push(dependee_proj.user_facing_name.to_string());
                                    dependee_proj.version.clone()
                                }

                                ReleaseAvailability::ExistingRelease(ref v) => v.clone(),

                                ReleaseAvailability::NewRelease => {
                                    if let Some(v) = new_versions.get(&dep.ident) {
                                        v.clone()
                                    } else {
                                        unsatisfied_deps
                                            .push(dependee_proj.user_facing_name.to_string());
                                        dependee_proj.version.clone()
                                    }
                                }
                            };

                            resolved_versions.push((idx, resolved));
                        }

                        DepRequirement::Manual(_) => {}

                        DepRequirement::Unavailable => {
                            let dependee_proj = self.graph.lookup(dep.ident);
                            unsatisfied_deps.push(dependee_proj.user_facing_name.to_string());
                            resolved_versions.push((idx, dependee_proj.version.clone()));
                        }
                    }
                }

                resolved_versions
            };

            {
                let unit = self.graph.lookup_mut(ident);

                for (idx, resolved) in resolved_versions.drain(..) {
                    unit.internal_deps[idx].resolved_version = Some(resolved);
                }
            }

            // Now, let the callback do its thing with the project, and tell us
            // if it gets a new release.

            let updated_version = atry!(
                process(&mut self.repo, &mut self.graph, ident);
                ["failed to solve internal dependencies of project `{}`", self.graph.lookup(ident).user_facing_name]
            );

            let unit = self.graph.lookup(ident);

            if updated_version {
                if !unsatisfied_deps.is_empty() {
                    return Err(UnsatisfiedInternalRequirementError(
                        unit.user_facing_name.to_string(),
                        unsatisfied_deps.join(", "),
                    )
                    .into());
                }

                new_versions.insert(ident, unit.version.clone());
            } else if !unsatisfied_deps.is_empty() {
                warn!(
                    "project `{}` has internal requirements that won't be satisfiable in the wild, \
                     but that's OK since it's not going to be released",
                    unit.user_facing_name
                );
            }
        }

        Ok(())
    }

    /// A fake version of `solve_internal_deps`. Rather than properly expressing
    /// internal version requirements, this manually assigns each internal
    /// dependency to match exactly the version of the depended-upon package.
    /// This functionality is needed for Lerna, which otherwise isn't clever
    /// enough to correctly detect the internal dependency.
    pub fn fake_internal_deps(&mut self) {
        let toposorted_idents: Vec<_> = self.graph.toposorted().collect();

        for ident in (toposorted_idents[..]).iter().copied() {
            let mut resolved_versions = {
                let unit = self.graph.lookup(ident);
                let mut resolved_versions = Vec::new();

                for (idx, dep) in unit.internal_deps.iter().enumerate() {
                    let dependee_proj = self.graph.lookup(dep.ident);
                    resolved_versions.push((idx, dependee_proj.version.clone()));
                }

                resolved_versions
            };

            {
                let unit = self.graph.lookup_mut(ident);

                for (idx, resolved) in resolved_versions.drain(..) {
                    unit.internal_deps[idx].belaf_requirement =
                        DepRequirement::Manual(resolved.to_string());
                    unit.internal_deps[idx].resolved_version = Some(resolved);
                }
            }
        }
    }

    pub fn apply_versions(&mut self, bump_specs: &HashMap<String, String>) -> Result<()> {
        let histories = self.analyze_histories()?;

        self.solve_internal_deps(|_repo, graph, ident| {
            let unit = graph.lookup_mut(ident);
            let history = histories.lookup(ident);

            if let Some(tag_version) = history.release_version() {
                unit.version = unit.version.parse_like(tag_version.to_string())?;
            }

            let baseline_version = unit.version.clone();

            Ok(
                if let Some(bump_spec) = bump_specs.get(&unit.user_facing_name) {
                    let scheme = unit.version.parse_bump_scheme(bump_spec)?;
                    scheme.apply(&mut unit.version)?;
                    info!(
                        "{}: {} => {}",
                        unit.user_facing_name, baseline_version, unit.version
                    );
                    true
                } else {
                    info!(
                        "{}: unchanged from {}",
                        unit.user_facing_name, baseline_version
                    );
                    false
                },
            )
        })
        .with_context(|| "failed to solve internal dependencies")?;

        Ok(())
    }

    /// Rewrite everyone's metadata to match our internal state.
    pub fn rewrite(&self) -> Result<ChangeList> {
        let mut changes = ChangeList::default();

        for ident in self.graph.toposorted() {
            let unit = self.graph.lookup(ident);

            for rw in &unit.rewriters {
                rw.rewrite(self, &mut changes)?;
            }
        }

        // `Cargo.lock` records the versions the manifests just moved, so it is
        // stale the moment any of them is written — and a release commit that
        // carries bumped manifests against a stale lock leaves every later
        // checkout with a dirty working tree, which `prepare` refuses to run
        // in. Syncing here rather than inside a rewriter covers both the
        // auto-discovered (`CargoRewriter`) and the configured
        // (`MultiManifestRewriter`) paths, and lets one `cargo update
        // --workspace` settle however many crates bumped.
        let written: Vec<crate::core::git::repository::RepoPathBuf> =
            changes.paths().map(|p| p.to_owned()).collect();
        for lockfile in crate::core::cargo_lock::sync_after_rewrite(
            &self.repo,
            written.iter().map(|p| p.as_ref()),
        )? {
            changes.add_path(&lockfile);
        }

        Ok(changes)
    }

    /// Like rewrite(), but only for the special Belaf requirements metadata.
    /// This is convenience functionality not needed for the main workflows.
    pub fn rewrite_belaf_requirements(&self) -> Result<ChangeList> {
        let mut changes = ChangeList::default();

        for ident in self.graph.toposorted() {
            let unit = self.graph.lookup(ident);

            for rw in &unit.rewriters {
                rw.rewrite_belaf_requirements(self, &mut changes)?;
            }
        }

        Ok(changes)
    }

    pub fn analyze_histories(&self) -> Result<RepoHistories> {
        let matchers = self.build_runtime_tag_matchers()?;
        // F-decouple — `analyze_histories` no longer consumes the scope matcher;
        // WHETHER is path-based only. The `[commit_attribution]` config is still
        // held on the session and consumed by `belaf check` (F8).
        self.graph
            .analyze_histories(&self.repo, &matchers, &self.binary_affecting)
    }

    /// One [`TagMatcher`] per graph unit, in graph order — the exact input
    /// [`Self::analyze_histories`] feeds the boundary resolution.
    fn build_runtime_tag_matchers(&self) -> Result<Vec<TagMatcher>> {
        let registry = FormatHandlerRegistry::with_defaults();
        let project_refs: Vec<&ResolvedReleaseUnit> = self.graph.projects_slice().iter().collect();
        build_matchers_for_runtime_units(
            &project_refs,
            &self.resolved_release_units,
            self.graph.groups(),
            &registry,
        )
    }

    /// The `Deploy` units that `belaf prepare` would refuse to analyze:
    /// no matching release tag, no per-unit `baseline`, no repo-wide
    /// `belaf-baseline`, in a repo that already carries version-shaped tags.
    ///
    /// Exactly the set [`Self::analyze_histories`] turns into an error —
    /// same call, same precedence — but returned as data so `belaf baseline`
    /// can report or fix it without running a release.
    pub fn untagged_deploy_units(
        &self,
    ) -> Result<Vec<crate::core::git::history::UntaggedDeployUnit>> {
        let matchers = self.build_runtime_tag_matchers()?;
        Ok(self
            .repo
            .resolve_history_boundaries(self.graph.projects_slice(), &matchers)?
            .untagged)
    }
}

pub enum ExecutionEnvironment {
    Ci,
    NotCi,
}
