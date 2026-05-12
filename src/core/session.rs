// Copyright 2020 Peter Williams <peter@newton.cx> and collaborators
// Licensed under the MIT License.

//! State for the Belaf CLI application.

use anyhow::{anyhow, Context};
use std::collections::HashMap;
use thiserror::Error as ThisError;
use tracing::{error, info, warn};

use crate::{
    atry,
    core::{
        config::{syntax::ChangelogConfiguration, ConfigurationFile},
        ecosystem::format_handler::FormatHandlerRegistry,
        errors::Result,
        git::repository::{ChangeList, ReleaseAvailability, Repository},
        graph::{ReleaseUnitGraph, ReleaseUnitGraphBuilder, RepoHistories},
        group::GroupSet,
        resolved_release_unit::{DepRequirement, ReleaseUnitId, ResolvedReleaseUnit},
        tag_format::{
            build_tag_matcher, split_maven_coords, TagMatcher, TagPatternInputs,
        },
        version::Version,
    },
    utils::theme::ReleaseProgressBar,
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

/// Setting up a Belaf application session.
pub struct AppBuilder {
    pub repo: Repository,
    pub graph: ReleaseUnitGraphBuilder,

    is_ci: bool,
    populate_graph: bool,
    show_progress: bool,
}

fn detect_ci_environment() -> bool {
    std::env::var("CI").is_ok()
        || std::env::var("GITHUB_ACTIONS").is_ok()
        || std::env::var("GITLAB_CI").is_ok()
        || std::env::var("CIRCLECI").is_ok()
        || std::env::var("TRAVIS").is_ok()
        || std::env::var("JENKINS_URL").is_ok()
}

impl AppBuilder {
    /// Start initializing an application session.
    ///
    /// This first phase of initialization may fail if the environment doesn't
    /// associate the process with a proper Git repository with a work tree.
    pub fn new() -> Result<AppBuilder> {
        let repo = Repository::open_from_env()?;
        let graph = ReleaseUnitGraphBuilder::new();
        let is_ci = detect_ci_environment();

        Ok(AppBuilder {
            graph,
            repo,
            is_ci,
            populate_graph: true,
            show_progress: false,
        })
    }

    pub fn with_progress(mut self, show_progress: bool) -> Self {
        self.show_progress = show_progress;
        self
    }

    pub fn populate_graph(mut self, do_populate: bool) -> Self {
        self.populate_graph = do_populate;
        self
    }

    /// Walk every project whose manifest reported version `0.0.0` and
    /// try to recover the real current version from an existing git
    /// tag. Uses the same template-driven lookup as the post-init code
    /// path, honouring per-`[release_unit]` `tag_format` overrides.
    ///
    /// Group-level (`[group.X].tag_format`) overrides are not consulted
    /// here because the `GroupSet` is only assembled inside
    /// [`ReleaseUnitGraphBuilder::complete_loading_with_groups`], which
    /// runs *after* this pass. That's acceptable: a group-only override
    /// only matters if the unit's manifest also reads as 0.0.0 (rare),
    /// and the post-init `analyze_histories` path applies the full
    /// precedence chain again before any bump decision is made.
    fn resolve_versions_from_tags(
        &mut self,
        resolved_units: &[crate::core::release_unit::ResolvedReleaseUnit],
    ) -> Result<()> {
        let is_single_project = self.graph.unit_count() == 1;
        let registry = FormatHandlerRegistry::with_defaults();

        for ident in self.graph.project_ids() {
            let unit = self.graph.lookup_mut(ident);

            let is_zero_version = matches!(
                &unit.version,
                Some(Version::Semver(v)) if v.major == 0
                    && v.minor == 0
                    && v.patch == 0
                    && v.pre.is_empty()
                    && v.build.is_empty()
            );

            if !is_zero_version {
                continue;
            }

            let Some(project_name) = unit.qnames.first().cloned() else {
                warn!(
                    "project at index {} has no qualified names, skipping version resolution",
                    ident
                );
                continue;
            };
            let ecosystem_name = unit
                .qnames
                .get(1)
                .cloned()
                .unwrap_or_else(|| "cargo".to_string());
            let tag_format_override = resolved_units
                .iter()
                .find(|r| r.unit.name == project_name)
                .and_then(|r| r.unit.tag_format.as_deref())
                .map(|s| s.to_string());

            let matcher = build_tag_matcher_for(
                &project_name,
                &ecosystem_name,
                tag_format_override.as_deref(),
                None, // groups not assembled yet — see fn docstring
                &registry,
                is_single_project,
            )?;

            if let Some((_, tag_name, version)) =
                self.repo.find_latest_tag_for_project(&matcher)?
            {
                if version.major != 0 || version.minor != 0 || version.patch != 0 {
                    info!(
                        "resolved version {} from tag '{}' for project '{}'",
                        version, tag_name, project_name
                    );
                    unit.version = Some(Version::Semver(version));
                }
            }
        }

        Ok(())
    }

    /// Finish app initialization, yielding a full AppSession object.
    pub fn initialize(mut self) -> Result<AppSession> {
        // Start by loading the configuration file, if it exists. If it doesn't
        // we'll get a sensible default.

        let mut cfg_path = self.repo.resolve_config_dir();
        cfg_path.push("config.toml");
        let config = ConfigurationFile::get(&cfg_path).with_context(|| {
            format!(
                "failed to load repository config file `{}`",
                cfg_path.display()
            )
        })?;

        self.repo
            .apply_config(config.repo)
            .with_context(|| "failed to finalize repository setup")?;

        let ignore_paths = config.ignore_paths.paths.clone();
        let allow_uncovered = config.allow_uncovered.paths.clone();
        let mut resolved_units: Vec<crate::core::release_unit::ResolvedReleaseUnit> = Vec::new();

        // Now auto-detect everything in the repo index.

        if self.populate_graph {
            use crate::core::ecosystem::format_handler::{
                FormatHandlerRegistry, WorkspaceDiscovererRegistry,
            };
            use crate::core::release_unit::discovery::discover_implicit_release_units;
            use crate::core::release_unit::VersionSource;

            let registry = FormatHandlerRegistry::with_defaults();
            let discoverers = WorkspaceDiscovererRegistry::with_defaults();

            // Resolve `[release_unit.<name>]` entries first so we can
            // (a) add full-explicit / glob units to the graph as primary
            // nodes, (b) feed their manifest+satellite paths to discovery
            // as a skip-list, and (c) match partial-override blocks
            // (those without `ecosystem`) against the auto-detected set
            // after discovery returns.
            let resolve_output =
                crate::core::release_unit::resolver::resolve(&self.repo, &config.release_units)
                    .map_err(|e| {
                        crate::core::errors::Error::msg(format!("release_unit resolution: {e}"))
                    })?;
            resolved_units = resolve_output.resolved;

            let mut configured_skip_paths: Vec<crate::core::git::repository::RepoPathBuf> =
                Vec::new();
            for r in &resolved_units {
                if let VersionSource::Manifests(ms) = &r.unit.source {
                    for m in ms {
                        let escaped = m.path.escaped().to_string();
                        if let Some(parent) = std::path::Path::new(&escaped).parent() {
                            let parent_str = parent.to_string_lossy().to_string();
                            if !parent_str.is_empty() {
                                configured_skip_paths.push(
                                    crate::core::git::repository::RepoPathBuf::new(
                                        parent_str.as_bytes(),
                                    ),
                                );
                            }
                        }
                    }
                }
                for sat in &r.unit.satellites {
                    configured_skip_paths.push(sat.clone());
                }
            }
            for p in &config.ignore_paths.paths {
                configured_skip_paths.push(crate::core::git::repository::RepoPathBuf::new(
                    p.trim_end_matches('/').as_bytes(),
                ));
            }

            for resolved in &resolved_units {
                self.add_configured_unit_to_graph(&registry, resolved)?;
            }

            // The skip-list keeps auto-discovery from claiming the
            // same manifest paths that a `[release_unit.X]` block
            // already covers.
            let discovered = discover_implicit_release_units(
                &self.repo,
                &registry,
                &discoverers,
                &configured_skip_paths,
            )?;

            // Match partial-override specs against the discovered set
            // and synthesize ResolvedReleaseUnits whose override fields
            // (tag_format, visibility, satellites, cascade_from) take
            // effect at workflow time. These are NOT registered via
            // `add_configured_unit_to_graph` — graph registration goes
            // through the discovered unit's already-built rewriters.
            let partial_resolved =
                crate::core::release_unit::resolver::resolve_partial_against_discovered(
                    &resolve_output.partial_overrides,
                    &discovered,
                )
                .map_err(|e| {
                    crate::core::errors::Error::msg(format!("partial-override resolution: {e}"))
                })?;
            resolved_units.extend(partial_resolved);

            if self.show_progress {
                let total = discovered.len();
                let mut progress = ReleaseProgressBar::new(total, "Loading release units");
                for (idx, du) in discovered.into_iter().enumerate() {
                    progress.update(idx);
                    Self::register_discovered_unit(&mut self.graph, du);
                }
                progress.finish();
            } else {
                for du in discovered {
                    Self::register_discovered_unit(&mut self.graph, du);
                }
            }

            self.resolve_versions_from_tags(&resolved_units)?;
        }

        // Apply project config and compile the graph.

        let graph = self.graph.complete_loading_with_groups(&config.groups)?;

        Ok(AppSession {
            repo: self.repo,
            graph,
            npm_config: NpmConfig::default(),
            changelog_config: config.changelog,
            bump_config: config.bump,
            bump_sources: config.bump_sources,
            resolved_release_units: resolved_units,
            ignore_paths,
            allow_uncovered,
            detection_cache: std::sync::OnceLock::new(),
            is_ci: self.is_ci,
        })
    }

    /// Add a configured `[release_unit.X]` block to the graph as a
    /// primary node. Reads the version from the canonical manifest
    /// (or runs the external `read_command`), constructs rewriters
    /// for every manifest in the unit's `manifests = [...]`, and
    /// registers them on the graph builder.
    fn add_configured_unit_to_graph(
        &mut self,
        registry: &crate::core::ecosystem::format_handler::FormatHandlerRegistry,
        resolved: &crate::core::release_unit::ResolvedReleaseUnit,
    ) -> Result<()> {
        use crate::core::release_unit::{ResolveOrigin, VersionSource};

        // Partial-override units are synthesized AFTER discovery from the
        // matching DiscoveredUnit. Graph registration (version read,
        // rewriter setup) happens through `register_discovered_unit`;
        // re-registering here would duplicate rewriters.
        if matches!(resolved.origin, ResolveOrigin::PartialOverride { .. }) {
            return Ok(());
        }

        let unit = &resolved.unit;
        let qnames = vec![unit.name.clone(), unit.ecosystem.as_str().to_string()];

        let (version, prefix, manifests_for_rewriter): (Version, _, _) = match &unit.source {
            VersionSource::Manifests(ms) => {
                let first = ms.first().ok_or_else(|| {
                    anyhow!("release_unit `{}` has empty manifests = []", unit.name)
                })?;
                let abs = self.repo.resolve_workdir(&first.path);
                let version_str = crate::core::version_field::read(&first.version_field, &abs)
                    .with_context(|| {
                        format!(
                            "reading version for release_unit `{}` from `{}`",
                            unit.name,
                            first.path.escaped()
                        )
                    })?;
                let version = parse_version_for_ecosystem(&version_str, unit.ecosystem.as_str())
                    .with_context(|| {
                        format!(
                            "parsing version `{}` for release_unit `{}`",
                            version_str, unit.name
                        )
                    })?;
                let (prefix_path, _) = first.path.split_basename();
                (version, prefix_path.to_owned(), ms.clone())
            }
            VersionSource::External(ext) => {
                let version_str = crate::core::rewriters::external::read_current(ext, &self.repo)
                    .map_err(|e| {
                    anyhow!(
                        "reading external versioner for release_unit `{}`: {}",
                        unit.name,
                        e
                    )
                })?;
                let version = parse_version_for_ecosystem(&version_str, unit.ecosystem.as_str())
                    .with_context(|| {
                        format!(
                            "parsing version `{}` from external read_command for `{}`",
                            version_str, unit.name
                        )
                    })?;
                let prefix = unit
                    .satellites
                    .first()
                    .cloned()
                    .unwrap_or_else(|| crate::core::git::repository::RepoPathBuf::new(b""));
                (version, prefix, Vec::new())
            }
        };

        let id = self.graph.add_project(qnames);
        let unit_node = self.graph.lookup_mut(id);
        unit_node.version = Some(version);
        unit_node.prefix = Some(prefix);

        if !manifests_for_rewriter.is_empty() {
            unit_node.rewriters.push(Box::new(
                crate::core::rewriters::multi_manifest::MultiManifestRewriter::new(
                    id,
                    manifests_for_rewriter,
                ),
            ));
        }
        let _ = registry; // FormatHandler-specific rewriters are deferred
                          // to the auto-discovered units.

        Ok(())
    }

    /// Register a `DiscoveredUnit` (from auto-discovery) as a graph
    /// node. Closures capturing rewriter logic run here once the
    /// unit's `ReleaseUnitId` is assigned.
    fn register_discovered_unit(
        graph: &mut ReleaseUnitGraphBuilder,
        du: crate::core::ecosystem::format_handler::DiscoveredUnit,
    ) {
        use crate::core::resolved_release_unit::DependencyTarget;

        let id = graph.add_project(du.qnames);
        let node = graph.lookup_mut(id);
        node.version = Some(du.version);
        node.prefix = Some(du.prefix);
        for factory in du.rewriter_factories {
            node.rewriters.push(factory(id));
        }
        for dep in du.internal_deps {
            graph.add_dependency(
                id,
                DependencyTarget::Text(dep.target_package_name),
                dep.literal,
                dep.requirement,
            );
        }
    }
}

fn parse_version_for_ecosystem(version_str: &str, ecosystem: &str) -> Result<Version> {
    let trimmed = version_str.trim();
    if ecosystem == "pypa" {
        Ok(Version::Pep440(
            trimmed.parse().map_err(|e| anyhow!("not PEP 440: {e}"))?,
        ))
    } else {
        Ok(Version::Semver(
            semver::Version::parse(trimmed).map_err(|e| anyhow!("not semver: {e}"))?,
        ))
    }
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
        let registry = FormatHandlerRegistry::with_defaults();
        let project_refs: Vec<&ResolvedReleaseUnit> = self.graph.projects_slice().iter().collect();
        let matchers = build_matchers_for_runtime_units(
            &project_refs,
            &self.resolved_release_units,
            self.graph.groups(),
            &registry,
        )?;
        self.graph.analyze_histories(&self.repo, &matchers)
    }
}

pub enum ExecutionEnvironment {
    Ci,
    NotCi,
}
