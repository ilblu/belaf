// Copyright 2020 Peter Williams <peter@newton.cx> and collaborators
// Licensed under the MIT License.

//! The graph of projects within the repository.
//!
//! A Belaf-enabled repository may adopt a “monorepo” model where it contains
//! multiple projects, each with their own independent versioning scheme. The
//! projects will likely all be managed in a single repository because they
//! depend on each other. In the general case, these intra-repository
//! dependencies have the structure of a directed acyclic graph (DAG).

use petgraph::{
    algo::toposort,
    graph::{DefaultIx, DiGraph, NodeIndex},
};
use std::collections::{HashMap, HashSet};

use crate::core::{
    config::syntax::ResolvedGroupConfig,
    errors::Result,
    git::repository::{RepoHistory, Repository},
    group::{Group, GroupId, GroupSet},
    resolved_release_unit::{
        DepRequirement, Dependency, DependencyBuilder, DependencyTarget, ReleaseUnitId,
        ResolvedReleaseUnit, ResolvedReleaseUnitBuilder, UnitKind,
    },
    tag_format::TagMatcher,
};
use crate::{a_ok_or, atry};

pub mod errors;
pub use errors::{DependencyCycleError, NamingClashError, NoSuchProjectError};

type OurNodeIndex = NodeIndex<DefaultIx>;

/// A DAG of projects expressing their dependencies.
#[derive(Debug, Default)]
pub struct ReleaseUnitGraph {
    /// The projects. Projects are uniquely identified by their index into this
    /// vector.
    projects: Vec<ResolvedReleaseUnit>,

    /// The `petgraph` state expressing the project graph.
    graph: DiGraph<ReleaseUnitId, ()>,

    /// Mapping from user-facing project name to project ID. This is calculated
    /// in the complete_loading() method.
    name_to_id: HashMap<String, ReleaseUnitId>,

    /// ResolvedReleaseUnit IDs in a topologically sorted order.
    toposorted_ids: Vec<ReleaseUnitId>,

    /// Groups: bundles of projects that release together. Sourced from
    /// `[[group]]` entries in `belaf/config.toml`. See `core::group`.
    groups: GroupSet,

    /// `ReleaseUnitId` → petgraph `NodeIndex`. Carried over from the builder
    /// (lockstep with `add_project`) so [`Self::closure`] can start a graph
    /// traversal from a unit id. `retain_edges` only drops edges, never nodes,
    /// so these indices stay valid after `complete_loading_with_groups`.
    node_ixs: Vec<OurNodeIndex>,
}

impl ReleaseUnitGraph {
    /// Get a reference to a project in the graph from its ID.
    pub fn lookup(&self, ident: ReleaseUnitId) -> &ResolvedReleaseUnit {
        &self.projects[ident]
    }

    /// Get a mutable reference to a project in the graph from its ID.
    pub fn lookup_mut(&mut self, ident: ReleaseUnitId) -> &mut ResolvedReleaseUnit {
        &mut self.projects[ident]
    }

    /// Get a project ID from its user-facing name.
    ///
    /// None indicates that the name is not found.
    pub fn lookup_ident<S: AsRef<str>>(&self, name: S) -> Option<ReleaseUnitId> {
        self.name_to_id.get(name.as_ref()).copied()
    }

    /// Iterate over all projects in the graph, in no particular order.
    ///
    /// In many cases [[`Self::toposorted`]] may be preferable.
    pub fn projects(&self) -> GraphIter<'_> {
        GraphIter {
            graph: self,
            node_idxs_iter: self
                .graph
                .node_indices()
                .collect::<Vec<OurNodeIndex>>()
                .into_iter(),
        }
    }

    /// Get an iterator to visit the project identifiers in the graph in
    /// topologically sorted order.
    ///
    /// That is, if project A in the repository depends on project B, project B
    /// will be visited before project A. This operation is fallible if the
    /// dependency graph contains cycles — i.e., if project B depends on project
    /// A and project A depends on project B. This shouldn't happen but isn't
    /// strictly impossible.
    pub fn toposorted(&self) -> TopoSortIdentIter<'_> {
        TopoSortIdentIter {
            graph: self,
            index: 0,
        }
    }

    pub fn query(&self, query: GraphQueryBuilder) -> Result<Vec<ReleaseUnitId>> {
        let mut matched_idents = Vec::new();
        let mut seen_ids = HashSet::new();

        let root_idents = if query.no_names() {
            self.toposorted_ids.clone()
        } else {
            let mut root_idents = Vec::new();

            for name in query.names {
                if let Some(id) = self.name_to_id.get(&name) {
                    root_idents.push(*id);
                } else {
                    return Err(NoSuchProjectError(name).into());
                }
            }

            root_idents
        };

        for id in root_idents {
            let unit = &self.projects[id];

            if let Some(ref ptype) = query.ecosystem_filter {
                let qnames = unit.qualified_names();
                let n = qnames.len();

                if n < 2 {
                    continue;
                }

                if &qnames[n - 1] != ptype {
                    continue;
                }
            }

            if seen_ids.insert(id) {
                matched_idents.push(id);
            }
        }

        Ok(matched_idents)
    }

    pub fn analyze_histories(
        &self,
        repo: &Repository,
        matchers: &[TagMatcher],
        binary_affecting: &crate::core::config::syntax::BinaryAffectingConfiguration,
    ) -> Result<RepoHistories> {
        // F2 — precompute each unit's dependency closure as positions in the
        // projects slice. For the full ordered slice, position == ReleaseUnitId,
        // so `closure(id)` yields the positions a unit's commits may come from.
        let closures: Vec<Vec<usize>> = (0..self.projects.len())
            .map(|id| self.closure(id))
            .collect();
        Ok(RepoHistories {
            histories: repo.analyze_histories(
                &self.projects[..],
                matchers,
                &closures,
                binary_affecting,
            )?,
        })
    }

    /// Slice access for callers that need to construct per-project
    /// metadata parallel to the graph's project order (e.g. building
    /// a `Vec<TagMatcher>` for [`Self::analyze_histories`]).
    pub fn projects_slice(&self) -> &[ResolvedReleaseUnit] {
        &self.projects
    }

    /// Read-only access to the configured project groups.
    pub fn groups(&self) -> &GroupSet {
        &self.groups
    }

    /// Compute the transitive dependency **closure** of `root` (F2): the unit
    /// itself plus every in-repo crate it depends on, transitively.
    ///
    /// Traverses the petgraph (NOT the raw `internal_deps`) so it inherits the
    /// graph's two guarantees for free: intra-group edges are already filtered
    /// out (`retain_edges`), and the graph is toposort-acyclic. A `visited` set
    /// makes the walk cycle-/diamond-safe regardless.
    ///
    /// Edges are stored `dependee → depender` (see `complete_loading_with_groups`),
    /// so a dependency-closure walk follows **incoming** edges from `root`.
    /// `Ignore` units are never roots and are never traversed into.
    pub fn closure(&self, root: ReleaseUnitId) -> Vec<ReleaseUnitId> {
        let mut visited: HashSet<ReleaseUnitId> = HashSet::new();
        let mut out: Vec<ReleaseUnitId> = Vec::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            if !visited.insert(id) {
                continue;
            }
            if self.projects[id].kind == UnitKind::Ignore {
                continue;
            }
            out.push(id);
            let nix = self.node_ixs[id];
            for dep_nix in self
                .graph
                .neighbors_directed(nix, petgraph::Direction::Incoming)
            {
                let dep_id = self.graph[dep_nix];
                if !visited.contains(&dep_id) {
                    stack.push(dep_id);
                }
            }
        }
        out
    }
}

/// This type is how we "launder" the knowledge that the vector that
/// comes out of repo.analyze_histories can be mapped into ReleaseUnitId values.
#[derive(Clone, Debug)]
pub struct RepoHistories {
    histories: Vec<RepoHistory>,
}

impl RepoHistories {
    /// Given a project ID, look up its history
    pub fn lookup(&self, projid: ReleaseUnitId) -> &RepoHistory {
        &self.histories[projid]
    }
}

/// Builder structure for querying projects in the graph.
#[derive(Debug, Default)]
pub struct GraphQueryBuilder {
    names: Vec<String>,
    ecosystem_filter: Option<String>,
}

impl GraphQueryBuilder {
    pub fn names<T: std::fmt::Display>(&mut self, names: impl IntoIterator<Item = T>) -> &mut Self {
        self.names = names.into_iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn only_ecosystem<T: std::fmt::Display>(&mut self, ptype: T) -> &mut Self {
        self.ecosystem_filter = Some(ptype.to_string());
        self
    }

    pub fn no_names(&self) -> bool {
        self.names.is_empty()
    }
}

/// A builder for the project graph upon app startup.
///
/// We do not impl Default even though we could, because the only way to
/// create one of these should be via the AppBuilder.
#[derive(Debug)]
pub struct ReleaseUnitGraphBuilder {
    /// The projects. Projects are uniquely identified by their index into this
    /// vector.
    projects: Vec<ResolvedReleaseUnitBuilder>,

    /// NodeIndex values for each project based on its identifier.
    node_ixs: Vec<OurNodeIndex>,

    /// The `petgraph` state expressing the project graph.
    graph: DiGraph<ReleaseUnitId, ()>,
}

impl ReleaseUnitGraphBuilder {
    pub(crate) fn new() -> ReleaseUnitGraphBuilder {
        ReleaseUnitGraphBuilder {
            projects: Vec::new(),
            node_ixs: Vec::new(),
            graph: DiGraph::default(),
        }
    }

    /// Register a new project with the graph and return its
    /// identifier.
    pub fn add_project(&mut self, qnames: Vec<String>) -> ReleaseUnitId {
        let mut pbuilder = ResolvedReleaseUnitBuilder::new();
        pbuilder.qnames = qnames;

        let id = self.projects.len();
        self.projects.push(pbuilder);
        self.node_ixs.push(self.graph.add_node(id));
        id
    }

    /// Get a reference to a project builder from its ID.
    pub fn lookup(&self, ident: ReleaseUnitId) -> &ResolvedReleaseUnitBuilder {
        &self.projects[ident]
    }

    /// Get a mutable reference to a project buider from its ID.
    pub fn lookup_mut(&mut self, ident: ReleaseUnitId) -> &mut ResolvedReleaseUnitBuilder {
        &mut self.projects[ident]
    }

    /// Get the number of projects in the graph.
    pub fn unit_count(&self) -> usize {
        self.projects.len()
    }

    /// Get an iterator over all project IDs.
    pub fn project_ids(&self) -> std::ops::Range<ReleaseUnitId> {
        0..self.projects.len()
    }

    /// Find a builder project by its primary qualified name (`qnames[0]`).
    /// Used by `[cascade_inputs]` materialization to wire an affected unit to a
    /// synthetic cascade-input node before `complete_loading` builds the
    /// name→id map.
    pub fn id_for_qname(&self, name: &str) -> Option<ReleaseUnitId> {
        self.projects
            .iter()
            .position(|p| p.qnames.first().map(String::as_str) == Some(name))
    }

    /// Resolve a dependency target read out of a manifest to a graph node.
    ///
    /// Prefers an exact `(qnames[0], qnames[1])` match so a cargo crate never
    /// binds to a same-named npm package. Falls back to a *unique* narrow-name
    /// match, which is what makes a configured unit resolvable when its
    /// declared `ecosystem` differs from the manifest the edge came from — a
    /// `tauri` unit holding a `Cargo.toml`, say.
    ///
    /// An ambiguous narrow name is an error rather than a first-match guess:
    /// picking the wrong node here silently wires a release to the wrong
    /// dependency.
    pub fn resolve_dep_target(&self, name: &str, ecosystem: &str) -> Result<ReleaseUnitId> {
        let exact = self.projects.iter().position(|p| {
            p.qnames.first().map(String::as_str) == Some(name)
                && p.qnames.get(1).map(String::as_str) == Some(ecosystem)
        });
        if let Some(id) = exact {
            return Ok(id);
        }

        let mut by_name = self
            .projects
            .iter()
            .enumerate()
            .filter(|(_, p)| p.qnames.first().map(String::as_str) == Some(name));
        match (by_name.next(), by_name.next()) {
            (Some((id, _)), None) => Ok(id),
            (Some((_, a)), Some((_, b))) => Err(anyhow::anyhow!(
                "dependency target `{name}` (from a {ecosystem} manifest) is ambiguous: it \
                 matches both `{}` and `{}`. Give the units distinct names.",
                a.qnames.join(":"),
                b.qnames.join(":"),
            )),
            (None, _) => Err(NoSuchProjectError(name.to_owned()).into()),
        }
    }

    /// Add a dependency between two projects in the graph.
    pub fn add_dependency(
        &mut self,
        depender_id: ReleaseUnitId,
        dependee_target: DependencyTarget,
        literal: String,
        req: DepRequirement,
    ) {
        self.projects[depender_id]
            .internal_deps
            .push(DependencyBuilder {
                target: dependee_target,
                literal,
                belaf_requirement: req,
                resolved_version: None,
            });
    }

    /// Complete construction of the graph.
    ///
    /// In particular, this function calculates unique, user-facing names for
    /// every project in the graph. After this function is called, new projects
    /// may not be added to the graph.
    ///
    /// If the internal project graph turns out to have a dependecy cycle, an
    /// error downcastable to DependencyCycleError.
    pub fn complete_loading(self) -> Result<ReleaseUnitGraph> {
        self.complete_loading_with_groups(&[])
    }

    /// Like [`complete_loading`], but binds `[[group]]` config entries to
    /// the resulting `ReleaseUnitGraph`. Member names are resolved against the
    /// graph's user-facing names; an unknown name is a hard error.
    pub fn complete_loading_with_groups(
        mut self,
        group_configs: &[ResolvedGroupConfig],
    ) -> Result<ReleaseUnitGraph> {
        // The first order of business is to determine every project's
        // user-facing name using progressive disambiguation with qualified names.

        let mut name_to_id = HashMap::new();

        // Each project has a vector of "qualified names" [n1, n2, ..., nN] that
        // should be unique. Here n1 is the "narrowest" name and probably
        // corresponds to what the user naively thinks of as the project names.
        // Farther-out names help us disambiguate, e.g. in a monorepo containing
        // a Python project and an NPM project with the same name. Our
        // disambiguation simply strings together n_narrow items from the narrow
        // end of the list. If qnames is [foo, bar, bax, quux] and n_narrow is
        // 2, the rendered name is "bar:foo".
        #[derive(Copy, Clone, Debug, Eq, PartialEq)]
        struct NamingState {
            pub n_narrow: usize,
        }

        impl Default for NamingState {
            fn default() -> Self {
                NamingState { n_narrow: 1 }
            }
        }

        impl NamingState {
            fn compute_name(&self, unit: &ResolvedReleaseUnitBuilder) -> String {
                let mut s = String::new();
                const SEP: char = ':';

                for i in 0..self.n_narrow {
                    if i != 0 {
                        s.push(SEP);
                    }

                    s.push_str(&unit.qnames[self.n_narrow - 1 - i]);
                }

                s
            }
        }

        let mut states = vec![NamingState::default(); self.projects.len()];
        let mut need_another_pass = true;

        while need_another_pass {
            name_to_id.clear();
            need_another_pass = false;

            for node_ix in &self.node_ixs {
                use std::collections::hash_map::Entry;
                let ident1 = self.graph[*node_ix];
                let proj1 = &self.projects[ident1];
                let candidate_name = states[ident1].compute_name(proj1);

                let ident2: ReleaseUnitId = match name_to_id.entry(candidate_name) {
                    Entry::Vacant(o) => {
                        // Great. No conflict.
                        o.insert(ident1);
                        continue;
                    }

                    Entry::Occupied(o) => o.remove(),
                };

                // If we're still here, we have a name conflict that needs
                // solving. We've removed the conflicting project from the map.
                //
                // We'd like to disambiguate both of the conflicting entries
                // equally. I.e., if the qnames are [pywwt, npm] and [pywwt,
                // python] we want to end up with "python:pywwt" and
                // "npm:pywwt", not "python:pywwt" and "pywwt".

                let proj2 = &self.projects[ident2];
                let qn1 = &proj1.qnames;
                let qn2 = &proj2.qnames;
                let n1 = qn1.len();
                let n2 = qn2.len();
                let mut success = false;

                for i in 0..std::cmp::min(n1, n2) {
                    if qn1[i] != qn2[i] {
                        success = true;
                        states[ident1].n_narrow = std::cmp::max(states[ident1].n_narrow, i + 1);
                        states[ident2].n_narrow = std::cmp::max(states[ident2].n_narrow, i + 1);
                        break;
                    }
                }

                if !success {
                    use std::cmp::Ordering;

                    match n1.cmp(&n2) {
                        Ordering::Greater => {
                            states[ident1].n_narrow =
                                std::cmp::max(states[ident1].n_narrow, n2 + 1);
                        }
                        Ordering::Less => {
                            states[ident2].n_narrow =
                                std::cmp::max(states[ident2].n_narrow, n1 + 1);
                        }
                        Ordering::Equal => {
                            return Err(NamingClashError(states[ident1].compute_name(proj1)).into());
                        }
                    }
                }

                if name_to_id
                    .insert(states[ident1].compute_name(proj1), ident1)
                    .is_some()
                {
                    need_another_pass = true; // this name clashes too!
                }

                if name_to_id
                    .insert(states[ident2].compute_name(proj2), ident2)
                    .is_some()
                {
                    need_another_pass = true; // this name clashes too!
                }
            }
        }

        // Now that we've figured out names, convert the ProjectBuilders into
        // projects. resolving internal dependencies and filling out the graph.
        //

        let mut projects = Vec::with_capacity(self.projects.len());

        for (ident, mut unit_builder) in self.projects.drain(..).enumerate() {
            let mut name = None;

            for (i_name, i_ident) in &name_to_id {
                if *i_ident == ident {
                    name = Some(i_name.clone());
                    break;
                }
            }

            let name = name.expect("BUG: every project should have a user-facing name assigned");
            let mut internal_deps = Vec::with_capacity(unit_builder.internal_deps.len());
            let depender_nix = self.node_ixs[ident];

            for dep in unit_builder.internal_deps.drain(..) {
                let dep_ident = match dep.target {
                    DependencyTarget::Ident(id) => id,
                    DependencyTarget::Text(ref dep_name) => *a_ok_or!(
                        name_to_id.get(dep_name);
                        ["project `{}` states a dependency on an unrecognized project name: `{}`",
                         name, dep_name]
                    ),
                };

                internal_deps.push(Dependency {
                    ident: dep_ident,
                    literal: dep.literal,
                    belaf_requirement: dep.belaf_requirement,
                    resolved_version: dep.resolved_version,
                });

                let dependee_nix = self.node_ixs[dep_ident];
                self.graph.add_edge(dependee_nix, depender_nix, ());
            }

            let unit = unit_builder.finalize(ident, name, internal_deps)?;
            projects.push(unit);
        }

        // Bind groups against the now-finalized name_to_id map. We do
        // this BEFORE toposort so intra-group edges can be filtered out
        // first — see plan §5: "internal_deps zwischen Group-Mitgliedern
        // → automatisch gefiltert vom Graph". Without the filter, two
        // grouped members that depend on each other look like a cycle
        // and break the whole graph build, even though they're meant to
        // ship as one atomic release.
        let mut groups = GroupSet::new();
        for gc in group_configs {
            let id = atry!(
                GroupId::new(&gc.id);
                ["invalid `[group.{}]` id in belaf/config.toml", gc.id]
            );
            let mut members = Vec::with_capacity(gc.members.len());
            for member_name in &gc.members {
                let pid = a_ok_or!(
                    name_to_id.get(member_name).copied();
                    ["group `{}` lists unknown member project `{}` (no such project in repo)",
                     id, member_name]
                );
                members.push(pid);
            }
            atry!(
                groups.add(Group { id: id.clone(), members, tag_format: gc.tag_format.clone() });
                ["failed to register group `{}`", id]
            );
        }

        // Filter intra-group edges. Members of the same release group
        // share one bump and one release moment, so any topological
        // ordering between them is meaningless — and a cycle between
        // them is OK by construction (e.g. a GraphQL schema published
        // as both an npm package and a Maven artifact, where both can
        // reference each other in their respective dep manifests).
        if !groups.is_empty() {
            self.graph.retain_edges(|g, e| {
                let (src_nix, dst_nix) = g.edge_endpoints(e).expect("edge exists");
                let src_pid = g[src_nix];
                let dst_pid = g[dst_nix];
                let src_group = groups.group_of(src_pid).map(|gr| gr.id.as_str());
                let dst_group = groups.group_of(dst_pid).map(|gr| gr.id.as_str());
                match (src_group, dst_group) {
                    (Some(a), Some(b)) if a == b => false, // intra-group → drop
                    _ => true,
                }
            });
        }

        // Now that intra-group edges are gone, we can verify the graph
        // has no remaining cycles and compute the topological sorting
        // once for reuse.

        let sorted_nixs = atry!(
            toposort(&self.graph, None).map_err(|cycle| {
                let ident = self.graph[cycle.node_id()];
                DependencyCycleError(projects[ident].user_facing_name.to_owned())
            });
            ["the project graph contains a dependency cycle"]
        );

        let toposorted_ids = sorted_nixs
            .iter()
            .map(|node_ix| self.graph[*node_ix])
            .collect();

        // Another bit of housekeeping: by default we set things up so that
        // project's path matchers are partially disjoint. In particular, if
        // there is a project rooted in prefix "a/" and a project rooted in
        // prefix "a/b/", we make it so that paths in "a/b/" are not flagged as
        // belonging to the project in "a/".
        //
        // The algorithm here (and in make_disjoint()) is not efficient, but it
        // shouldn't matter unless you have an unrealistically large number of
        // projects. We have to use split_at_mut() to get simultaneous
        // mutability of two pieces of the vec.

        //
        // `[cascade_inputs]` nodes are exempt in BOTH directions. Their paths
        // are *meant* to overlap the real units they feed (a base-image
        // definition can live inside a service's own directory), so:
        //   - subtracting a real unit from an input would cut away exactly the
        //     paths the input exists to watch;
        //   - subtracting an input from a real unit would stop the unit seeing
        //     its own files.
        for index1 in 1..projects.len() {
            let (left, right) = projects.split_at_mut(index1);
            let litem = &mut left[index1 - 1];

            for ritem in right {
                if litem.is_cascade_input || ritem.is_cascade_input {
                    continue;
                }
                litem.repo_paths.make_disjoint(&ritem.repo_paths);
                ritem.repo_paths.make_disjoint(&litem.repo_paths);
            }
        }

        // (Groups were bound earlier — before the intra-group edge
        // filter — so the toposort step above sees the post-filter
        // graph.)

        Ok(ReleaseUnitGraph {
            projects,
            name_to_id,
            graph: self.graph,
            toposorted_ids,
            groups,
            node_ixs: self.node_ixs,
        })
    }
}

/// An iterator for visiting the graph's pre-toposorted list of idents.
///
/// This type only exists to provide the convenience of an iterator over
/// this toposorted list that (a) doesn't clone the whole vec, by holding
/// a ref to the graph, but (b) yields ProjectIds, not &ProjectIds.
pub struct TopoSortIdentIter<'a> {
    graph: &'a ReleaseUnitGraph,
    index: usize,
}

impl<'a> Iterator for TopoSortIdentIter<'a> {
    type Item = ReleaseUnitId;

    fn next(&mut self) -> Option<ReleaseUnitId> {
        if self.index < self.graph.toposorted_ids.len() {
            let rv = self.graph.toposorted_ids[self.index];
            self.index += 1;
            Some(rv)
        } else {
            None
        }
    }
}

/// An iterator for visiting the projects in the graph.
pub struct GraphIter<'a> {
    graph: &'a ReleaseUnitGraph,
    node_idxs_iter: std::vec::IntoIter<OurNodeIndex>,
}

impl<'a> Iterator for GraphIter<'a> {
    type Item = &'a ResolvedReleaseUnit;

    fn next(&mut self) -> Option<&'a ResolvedReleaseUnit> {
        let node_ix = self.node_idxs_iter.next()?;
        let ident = self.graph.graph[node_ix];
        Some(self.graph.lookup(ident))
    }
}

#[cfg(test)]
#[path = "graph_tests.rs"]
mod graph_tests;
