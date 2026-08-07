//! Graph-construction helpers for [`AppBuilder`].
//!
//! Turns resolved `[release_unit.X]` blocks, auto-discovered units and
//! `[cascade_inputs.X]` entries into nodes and dependency edges on the
//! [`ReleaseUnitGraphBuilder`].

use anyhow::{anyhow, Context};

use crate::core::{errors::Result, graph::ReleaseUnitGraphBuilder, version::Version};

use super::AppBuilder;

/// Whether a `paths = [...]` entry is a glob pattern (Tier-3, F4-Glob) rather
/// than a literal directory prefix. Conservative: any of `*`, `?`, `[`.
fn path_is_glob(s: &str) -> bool {
    s.contains(['*', '?', '['])
}

impl AppBuilder {
    /// Add a configured `[release_unit.X]` block to the graph as a
    /// primary node. Reads the version from the canonical manifest
    /// (or runs the external `read_command`), constructs rewriters
    /// for every manifest in the unit's `manifests = [...]`, and
    /// registers them on the graph builder.
    pub(super) fn add_configured_unit_to_graph(
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

        let (version, prefix, manifests_for_rewriter, extra_includes): (
            Version,
            _,
            _,
            Vec<crate::core::git::repository::RepoPathBuf>,
        ) = match &unit.source {
            VersionSource::PathsOnly(paths) => {
                // Manifest-less internal/ignore unit (F1): no version to read.
                // Use a placeholder that is never emitted (these units are
                // excluded from candidacy). Literal paths become prefix includes
                // so commits in any of them attribute to this unit (for the
                // cascade closure); glob paths (Tier-3) are handled post-match.
                let version = parse_version_for_ecosystem("0.0.0", unit.ecosystem.as_str())
                    .with_context(|| {
                        format!(
                            "building placeholder version for paths-only unit `{}`",
                            unit.name
                        )
                    })?;
                let literals: Vec<crate::core::git::repository::RepoPathBuf> = paths
                    .iter()
                    .filter(|p| !path_is_glob(&p.escaped()))
                    .cloned()
                    .collect();
                let prefix = literals
                    .first()
                    .cloned()
                    .unwrap_or_else(|| crate::core::git::repository::RepoPathBuf::new(b""));
                let extra = literals.iter().skip(1).cloned().collect();
                (version, prefix, Vec::new(), extra)
            }
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
                (version, prefix_path.to_owned(), ms.clone(), Vec::new())
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
                (version, prefix, Vec::new(), Vec::new())
            }
        };

        let id = self.graph.add_project(qnames);
        let unit_node = self.graph.lookup_mut(id);
        unit_node.version = Some(version);
        unit_node.prefix = Some(prefix);
        unit_node.extra_includes = extra_includes;
        unit_node.kind = unit.kind;
        unit_node.bump_override = unit.bump_override.clone();
        unit_node.baseline = unit.baseline.clone();

        // Tier-3 (F4-Glob) — glob-shaped `paths` become residual glob matchers.
        // A unit with no literal paths skips the prefix `Include` (it would
        // over-match the repo root).
        if let VersionSource::PathsOnly(paths) = &unit.source {
            let globs: Vec<String> = paths
                .iter()
                .map(|p| p.escaped().to_string())
                .filter(|s| path_is_glob(s))
                .collect();
            if !globs.is_empty() {
                let has_literals = paths.iter().any(|p| !path_is_glob(&p.escaped()));
                unit_node.repo_paths_no_prefix_include = !has_literals;
                unit_node.extra_globs = globs;
            }
        }

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

    /// Turn `[cascade_inputs.<name>]` blocks into synthetic `Internal` graph
    /// nodes plus dependency edges.
    ///
    /// Each block becomes one manifest-less node named after the table key,
    /// owning the block's `paths`. Every affected unit gets an edge to it, so a
    /// change under one of those paths enters that unit's — and its
    /// dependents' — dependency closure. The node is `Internal`, so it is never
    /// versioned, tagged or released.
    ///
    /// Path handling matches [`Self::add_configured_unit_to_graph`] exactly:
    /// literal entries become prefix `Include` terms, glob-shaped entries
    /// become residual glob matchers. That is what makes `apko/*.lock` work —
    /// the predecessor (`[codegen_edges]`) turned every entry into a prefix and
    /// so produced the never-matching prefix `apko/*.lock/`.
    /// `glob_expansions` maps a glob-form `[release_unit.<key>]` config key to
    /// the unit names it expanded into, so `affects` can name the key instead
    /// of restating every unit — a list that would otherwise drift out of sync
    /// with the glob it duplicates.
    pub(super) fn materialize_cascade_inputs(
        &mut self,
        inputs: &[crate::core::config::ResolvedCascadeInput],
        binary_affecting: &crate::core::config::syntax::BinaryAffectingConfiguration,
        glob_expansions: &std::collections::BTreeMap<String, Vec<String>>,
    ) -> Result<()> {
        use crate::core::config::CascadeInputTargets;
        use crate::core::git::{path_matcher::is_binary_affecting, repository::RepoPathBuf};
        use crate::core::resolved_release_unit::{DepRequirement, DependencyTarget, UnitKind};

        if inputs.is_empty() {
            return Ok(());
        }

        /// Namespace slot (`qnames[1]`) for cascade-input nodes. User-visible
        /// through qualified names and error messages.
        const NAMESPACE: &str = "cascade_input";

        // `all-deploy-units` is resolved once, before any input node exists —
        // input nodes are `Internal` and would be filtered out anyway, but
        // computing it up front keeps the set independent of iteration order.
        let deploy_ids: Vec<_> = self
            .graph
            .project_ids()
            .filter(|&id| self.graph.lookup(id).kind == UnitKind::Deploy)
            .collect();

        for input in inputs {
            let name = &input.name;

            // A cascade-input node whose name collides with a real unit would
            // otherwise surface as a baffling "unrecognized project name" from
            // the text-dependency resolution in `complete_loading_with_groups`.
            if self.graph.id_for_qname(name).is_some() {
                return Err(anyhow!(
                    "[cascade_inputs.{name}] collides with an existing release unit named \
                     `{name}`. The table key becomes the input's node name, so it must be \
                     unique across the repo — rename the `[cascade_inputs.{name}]` block \
                     (e.g. `{name}-input`)."
                ));
            }

            // Warn about paths that `[binary_affecting]` can never let through:
            // the input would exist but silently never fire.
            for path in &input.paths {
                if !is_binary_affecting(path.as_bytes(), binary_affecting) {
                    tracing::warn!(
                        "[cascade_inputs.{name}] declares path `{path}`, which `[binary_affecting]` \
                         excludes from bump-relevant changes — this path can never trigger the \
                         input. Adjust the path or the `[binary_affecting]` exclusion lists."
                    );
                }
            }

            let literals: Vec<RepoPathBuf> = input
                .paths
                .iter()
                .filter(|p| !path_is_glob(p.as_str()))
                .map(|p| RepoPathBuf::new(p.as_bytes()))
                .collect();
            let globs: Vec<String> = input
                .paths
                .iter()
                .filter(|p| path_is_glob(p.as_str()))
                .cloned()
                .collect();

            let prefix = literals
                .first()
                .cloned()
                .unwrap_or_else(|| RepoPathBuf::new(b""));
            let extra_includes: Vec<RepoPathBuf> = literals.iter().skip(1).cloned().collect();

            let placeholder = parse_version_for_ecosystem("0.0.0", "cargo")
                .context("building placeholder version for cascade_input node")?;
            let id = self
                .graph
                .add_project(vec![name.clone(), NAMESPACE.to_string()]);
            let node = self.graph.lookup_mut(id);
            node.version = Some(placeholder);
            node.prefix = Some(prefix);
            node.extra_includes = extra_includes;
            node.kind = UnitKind::Internal;
            node.is_cascade_input = true;
            if !globs.is_empty() {
                // Same rule as glob-shaped `paths = [...]` units: skip the
                // prefix `Include` when there is no literal path, because the
                // empty prefix would match the whole repo.
                node.repo_paths_no_prefix_include = literals.is_empty();
                node.extra_globs = globs;
            }

            let targets: Vec<String> = match &input.affects {
                CascadeInputTargets::AllDeployUnits => deploy_ids
                    .iter()
                    .filter_map(|&did| self.graph.lookup(did).qnames.first().cloned())
                    .collect(),
                CascadeInputTargets::Units(names) => {
                    let mut resolved = Vec::new();
                    for entry in names {
                        let is_unit = self.graph.id_for_qname(entry).is_some();
                        let expanded = glob_expansions.get(entry);

                        match (is_unit, expanded) {
                            // A release unit and a glob key of the same name:
                            // guessing either way would silently release the
                            // wrong set.
                            (true, Some(units)) => anyhow::bail!(
                                "[cascade_inputs.{name}] `affects` lists `{entry}`, which is \
                                 ambiguous: it is both a release unit and the key of a \
                                 glob-form `[release_unit.{entry}]` covering {}. Rename one \
                                 of them.",
                                units.join(", ")
                            ),
                            (true, None) => resolved.push(entry.clone()),
                            (false, Some(units)) => resolved.extend(units.iter().cloned()),
                            // Previously a warning, which meant a typo here
                            // released nothing and said so only in a log line
                            // nobody reads — the same silent no-op this whole
                            // section exists to eliminate.
                            (false, None) => anyhow::bail!(
                                "[cascade_inputs.{name}] `affects` lists `{entry}`, which is \
                                 neither a known release unit nor the key of a glob-form \
                                 `[release_unit]`. Fix the name, or use \
                                 `affects = \"all-deploy-units\"`."
                            ),
                        }
                    }
                    resolved
                }
            };

            for target in &targets {
                match self.graph.id_for_qname(target) {
                    Some(target_id) => self.graph.add_dependency(
                        target_id,
                        DependencyTarget::Text(name.clone()),
                        NAMESPACE.to_string(),
                        DepRequirement::Unavailable,
                    ),
                    None => tracing::warn!(
                        "[cascade_inputs.{name}] lists `{target}` under `affects`, which is not a \
                         known release unit — skipping that edge"
                    ),
                }
            }
        }
        Ok(())
    }

    /// Register a `DiscoveredUnit` (from auto-discovery) as a graph
    /// node. Closures capturing rewriter logic run here once the
    /// unit's `ReleaseUnitId` is assigned.
    pub(super) fn register_discovered_unit(
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
