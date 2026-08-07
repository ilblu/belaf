//! Session construction — [`AppBuilder`].
//!
//! Opens the repository, loads `belaf/config.toml`, resolves and discovers
//! release units, populates the dependency graph and recovers versions from
//! existing tags, finally yielding a ready-to-use
//! [`AppSession`](super::AppSession). The graph-node registration step lives
//! in the sibling [`graph_build`](super::graph_build) module.

use anyhow::Context;
use tracing::{info, warn};

use crate::{
    core::{
        config::ConfigurationFile, ecosystem::format_handler::FormatHandlerRegistry,
        errors::Result, git::repository::Repository, graph::ReleaseUnitGraphBuilder,
        version::Version,
    },
    utils::theme::ReleaseProgressBar,
};

use super::{build_tag_matcher_for, AppSession, NpmConfig};

/// Setting up a Belaf application session.
pub struct AppBuilder {
    pub repo: Repository,
    pub graph: ReleaseUnitGraphBuilder,

    is_ci: bool,
    populate_graph: bool,
    show_progress: bool,
    fetch_tags_first: bool,
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
            fetch_tags_first: false,
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

    /// Opt into a `git fetch --tags` against the resolved upstream
    /// before project discovery. Required for commands that read tag
    /// state to make release decisions (`prepare`), because the
    /// release tags are created server-side by the belaf GitHub App
    /// after merging the manifest PR, and `git pull --ff-only` does
    /// not pull tags. Set `BELAF_NO_FETCH=1` to bypass at runtime —
    /// useful in tests and offline scenarios.
    pub fn fetch_tags_first(mut self, do_fetch: bool) -> Self {
        self.fetch_tags_first = do_fetch;
        self
    }

    /// Refresh tags from the upstream before deciding anything.
    ///
    /// Authenticated when it can be: a private repo over HTTPS rejects an
    /// anonymous fetch, and this runs before any other network call, so an
    /// unauthenticated attempt used to kill the run before it started. The
    /// credential is the same short-lived installation token the release push
    /// uses; obtaining it is best-effort, because public repos and SSH remotes
    /// work fine without one and a pre-install checkout has no token at all.
    ///
    /// Fail-soft: this is a refresh, not a prerequisite. If it fails while the
    /// repo already has version tags locally, warn and continue with what we
    /// have — a stale-by-one-release view is far better than a dead run, and
    /// in CI `actions/checkout` with `fetch-depth: 0` has already brought the
    /// tags anyway. Only a repo with *no* tags at all still errors: there the
    /// baseline decision would be made blind.
    fn fetch_tags_before_release_prep(&mut self) -> Result<()> {
        let git_token = match crate::core::github::client::fetch_git_credentials(&self.repo) {
            Ok(token) => Some(token),
            Err(e) => {
                info!("continuing tag fetch without credentials: {e}");
                None
            }
        };

        let Err(e) = self.repo.fetch_tags(git_token.as_deref()) else {
            return Ok(());
        };

        if self.repo.repo_has_any_version_tags().unwrap_or(false) {
            warn!(
                "could not refresh tags from the upstream ({e}) — continuing with the tags \
                 already in this clone. Release decisions may be based on a stale view if a \
                 release landed since the last fetch. Set BELAF_NO_FETCH=1 to skip this step."
            );
            return Ok(());
        }

        Err(e).context(
            "failed to fetch upstream tags before release prep, and this clone has no version \
             tags to fall back on — every unit would look brand-new and the computed bumps \
             would be wrong",
        )
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

            if let Some((_, tag_name, version)) = self.repo.find_latest_tag_for_project(&matcher)? {
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

        if self.fetch_tags_first && std::env::var_os("BELAF_NO_FETCH").is_none() {
            self.fetch_tags_before_release_prep()?;
        }

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

            // Partial-override `kind`s are normally applied after the graph is
            // built (see below), but `affects = "all-deploy-units"` needs the
            // true kinds *now* — otherwise a crate marked `kind = "internal"`
            // through a partial override would still count as a deploy unit.
            // The post-build pass stays: it also carries `bump_override`, and
            // it matches on the disambiguated user-facing name, which
            // `id_for_qname` (narrow name only) cannot always resolve.
            for ru in &resolved_units {
                if matches!(
                    ru.origin,
                    crate::core::release_unit::ResolveOrigin::PartialOverride { .. }
                ) {
                    if let Some(id) = self.graph.id_for_qname(&ru.unit.name) {
                        self.graph.lookup_mut(id).kind = ru.unit.kind;
                    }
                }
            }

            // Materialize `[cascade_inputs]` as synthetic internal nodes +
            // dependency edges, now that all real units are registered (so their
            // ids are resolvable). Must run before `complete_loading_with_groups`,
            // which resolves the `Text` dependency targets and builds the petgraph.
            // `affects` may name a glob-form `[release_unit.<key>]` instead of
            // listing every unit it expands to. The config key is not stored
            // on the resolved unit — `ResolveOrigin::Glob` keeps the index of
            // the entry it came from — so recover it from the same slice the
            // resolver indexed into.
            let mut glob_expansions: std::collections::BTreeMap<String, Vec<String>> =
                std::collections::BTreeMap::new();
            for r in &resolved_units {
                if let crate::core::release_unit::ResolveOrigin::Glob { glob_index, .. } = &r.origin
                {
                    if let Some(named) = config.release_units.get(*glob_index) {
                        glob_expansions
                            .entry(named.name.clone())
                            .or_default()
                            .push(r.unit.name.clone());
                    }
                }
            }

            self.materialize_cascade_inputs(
                &config.cascade_inputs,
                &config.binary_affecting,
                &glob_expansions,
            )?;

            self.resolve_versions_from_tags(&resolved_units)?;
        }

        // Apply project config and compile the graph.

        let mut graph = self.graph.complete_loading_with_groups(&config.groups)?;

        // F1 — apply `kind` overrides from partial-override blocks onto their
        // (discovered) graph nodes. Explicit/glob/paths-only units already had
        // `kind` set in `add_configured_unit_to_graph`; partial overrides
        // decorate a discovered node, so the kind is applied here by name.
        for ru in &resolved_units {
            if matches!(
                ru.origin,
                crate::core::release_unit::ResolveOrigin::PartialOverride { .. }
            ) {
                if let Some(id) = graph.lookup_ident(&ru.unit.name) {
                    let node = graph.lookup_mut(id);
                    node.kind = ru.unit.kind;
                    node.bump_override = ru.unit.bump_override.clone();
                    node.baseline = ru.unit.baseline.clone();
                }
            }
        }

        Ok(AppSession {
            repo: self.repo,
            graph,
            npm_config: NpmConfig::default(),
            changelog_config: config.changelog,
            bump_config: config.bump,
            commit_attribution: config.commit_attribution,
            binary_affecting: config.binary_affecting,
            cascade_inputs: config.cascade_inputs,
            bump_sources: config.bump_sources,
            resolved_release_units: resolved_units,
            ignore_paths,
            allow_uncovered,
            detection_cache: std::sync::OnceLock::new(),
            is_ci: self.is_ci,
        })
    }
}
