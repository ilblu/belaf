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
use thiserror::Error as ThisError;

use crate::core::{
    config::syntax::ProjectConfiguration,
    errors::Result,
    git::repository::{RepoHistory, Repository},
    project::{
        DepRequirement, Dependency, DependencyBuilder, DependencyTarget, Project, ProjectBuilder,
        ProjectId,
    },
};
use crate::{a_ok_or, atry};

type OurNodeIndex = NodeIndex<DefaultIx>;

/// A DAG of projects expressing their dependencies.
#[derive(Debug, Default)]
pub struct ProjectGraph {
    /// The projects. Projects are uniquely identified by their index into this
    /// vector.
    projects: Vec<Project>,

    /// The `petgraph` state expressing the project graph.
    graph: DiGraph<ProjectId, ()>,

    /// Mapping from user-facing project name to project ID. This is calculated
    /// in the complete_loading() method.
    name_to_id: HashMap<String, ProjectId>,

    /// Project IDs in a topologically sorted order.
    toposorted_ids: Vec<ProjectId>,
}

/// An error returned when an input has requested a project with a certain name,
/// and it just doesn't exist.
#[derive(Debug, ThisError)]
#[error("no such project with the name `{0}`")]
pub struct NoSuchProjectError(pub String);

impl ProjectGraph {
    /// Get a reference to a project in the graph from its ID.
    pub fn lookup(&self, ident: ProjectId) -> &Project {
        &self.projects[ident]
    }

    /// Get a mutable reference to a project in the graph from its ID.
    pub fn lookup_mut(&mut self, ident: ProjectId) -> &mut Project {
        &mut self.projects[ident]
    }

    /// Get a project ID from its user-facing name.
    ///
    /// None indicates that the name is not found.
    pub fn lookup_ident<S: AsRef<str>>(&self, name: S) -> Option<ProjectId> {
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

    pub fn query(&self, query: GraphQueryBuilder) -> Result<Vec<ProjectId>> {
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
            let proj = &self.projects[id];

            if let Some(ref ptype) = query.project_type {
                let qnames = proj.qualified_names();
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

    pub fn analyze_histories(&self, repo: &Repository) -> Result<RepoHistories> {
        Ok(RepoHistories {
            histories: repo.analyze_histories(&self.projects[..])?,
        })
    }
}

/// This type is how we "launder" the knowledge that the vector that
/// comes out of repo.analyze_histories can be mapped into ProjectId values.
#[derive(Clone, Debug)]
pub struct RepoHistories {
    histories: Vec<RepoHistory>,
}

impl RepoHistories {
    /// Given a project ID, look up its history
    pub fn lookup(&self, projid: ProjectId) -> &RepoHistory {
        &self.histories[projid]
    }
}

/// Builder structure for querying projects in the graph.
#[derive(Debug, Default)]
pub struct GraphQueryBuilder {
    names: Vec<String>,
    project_type: Option<String>,
}

impl GraphQueryBuilder {
    pub fn names<T: std::fmt::Display>(&mut self, names: impl IntoIterator<Item = T>) -> &mut Self {
        self.names = names.into_iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn only_project_type<T: std::fmt::Display>(&mut self, ptype: T) -> &mut Self {
        self.project_type = Some(ptype.to_string());
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
pub struct ProjectGraphBuilder {
    /// The projects. Projects are uniquely identified by their index into this
    /// vector.
    projects: Vec<ProjectBuilder>,

    /// NodeIndex values for each project based on its identifier.
    node_ixs: Vec<OurNodeIndex>,

    /// The `petgraph` state expressing the project graph.
    graph: DiGraph<ProjectId, ()>,
}

/// An error returned when the internal project graph has a dependency cycle.
/// The inner value is the user-facing name of a project involved in the cycle.
#[derive(Debug, ThisError)]
#[error("detected an internal dependency cycle associated with project {0}")]
pub struct DependencyCycleError(pub String);

/// An error returned when it is impossible to come up with distinct names for
/// two projects. This "should never happen", but ... The inner value is the
/// clashing name.
#[derive(Debug, ThisError)]
#[error("multiple projects with same name `{0}`")]
pub struct NamingClashError(pub String);

impl ProjectGraphBuilder {
    pub(crate) fn new() -> ProjectGraphBuilder {
        ProjectGraphBuilder {
            projects: Vec::new(),
            node_ixs: Vec::new(),
            graph: DiGraph::default(),
        }
    }

    /// Request to register a new project with the graph.
    ///
    /// The request may be denied if the user has specified that
    /// the project should be ignored.
    pub fn try_add_project(
        &mut self,
        qnames: Vec<String>,
        pconfig: &HashMap<String, ProjectConfiguration>,
    ) -> Option<ProjectId> {
        // Not the most elegant ... I can't get join() to work here due to the
        // rev(), though.

        let mut full_name = String::new();

        for term in qnames.iter().rev() {
            if !full_name.is_empty() {
                full_name.push(':')
            }

            full_name.push_str(term);
        }

        let ignore = pconfig
            .get(&full_name)
            .map(|c| c.ignore)
            .unwrap_or_default();
        if ignore {
            return None;
        }

        let mut pbuilder = ProjectBuilder::new();
        pbuilder.qnames = qnames;

        let id = self.projects.len();
        self.projects.push(pbuilder);
        self.node_ixs.push(self.graph.add_node(id));
        Some(id)
    }

    /// Get a mutable reference to a project buider from its ID.
    pub fn lookup_mut(&mut self, ident: ProjectId) -> &mut ProjectBuilder {
        &mut self.projects[ident]
    }

    /// Get the number of projects in the graph.
    pub fn project_count(&self) -> usize {
        self.projects.len()
    }

    /// Get an iterator over all project IDs.
    pub fn project_ids(&self) -> std::ops::Range<ProjectId> {
        0..self.projects.len()
    }

    /// Add a dependency between two projects in the graph.
    pub fn add_dependency(
        &mut self,
        depender_id: ProjectId,
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
    pub fn complete_loading(mut self) -> Result<ProjectGraph> {
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
            fn compute_name(&self, proj: &ProjectBuilder) -> String {
                let mut s = String::new();
                const SEP: char = ':';

                for i in 0..self.n_narrow {
                    if i != 0 {
                        s.push(SEP);
                    }

                    s.push_str(&proj.qnames[self.n_narrow - 1 - i]);
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

                let ident2: ProjectId = match name_to_id.entry(candidate_name) {
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

        for (ident, mut proj_builder) in self.projects.drain(..).enumerate() {
            let mut name = None;

            for (i_name, i_ident) in &name_to_id {
                if *i_ident == ident {
                    name = Some(i_name.clone());
                    break;
                }
            }

            let name = name.expect("BUG: every project should have a user-facing name assigned");
            let mut internal_deps = Vec::with_capacity(proj_builder.internal_deps.len());
            let depender_nix = self.node_ixs[ident];

            for dep in proj_builder.internal_deps.drain(..) {
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

            let proj = proj_builder.finalize(ident, name, internal_deps)?;
            projects.push(proj);
        }

        // Now that we've done that and compiled all of the interdependencies,
        // we can verify that the graph has no cycles. We compute the
        // topological sorting once and just reuse it later.

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

        for index1 in 1..projects.len() {
            let (left, right) = projects.split_at_mut(index1);
            let litem = &mut left[index1 - 1];

            for ritem in right {
                litem.repo_paths.make_disjoint(&ritem.repo_paths);
                ritem.repo_paths.make_disjoint(&litem.repo_paths);
            }
        }

        // All done

        Ok(ProjectGraph {
            projects,
            name_to_id,
            graph: self.graph,
            toposorted_ids,
        })
    }
}

/// An iterator for visiting the graph's pre-toposorted list of idents.
///
/// This type only exists to provide the convenience of an iterator over
/// this toposorted list that (a) doesn't clone the whole vec, by holding
/// a ref to the graph, but (b) yields ProjectIds, not &ProjectIds.
pub struct TopoSortIdentIter<'a> {
    graph: &'a ProjectGraph,
    index: usize,
}

impl<'a> Iterator for TopoSortIdentIter<'a> {
    type Item = ProjectId;

    fn next(&mut self) -> Option<ProjectId> {
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
    graph: &'a ProjectGraph,
    node_idxs_iter: std::vec::IntoIter<OurNodeIndex>,
}

impl<'a> Iterator for GraphIter<'a> {
    type Item = &'a Project;

    fn next(&mut self) -> Option<&'a Project> {
        let node_ix = self.node_idxs_iter.next()?;
        let ident = self.graph.graph[node_ix];
        Some(self.graph.lookup(ident))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{git::repository::RepoPathBuf, version::Version};

    fn do_name_assignment_test(spec: &[(&[&str], &str)]) -> Result<()> {
        let mut graph = ProjectGraphBuilder::new();
        let mut ids = HashMap::new();
        let empty_config = HashMap::new();

        for (qnames, user_facing) in spec {
            let qnames = qnames.iter().map(|s| (*s).to_owned()).collect();
            let projid = graph
                .try_add_project(qnames, &empty_config)
                .expect("BUG: test project should be added successfully");
            let b = graph.lookup_mut(projid);
            b.version = Some(Version::Semver(semver::Version::new(0, 0, 0)));
            b.prefix = Some(RepoPathBuf::new(b""));
            ids.insert(projid, user_facing);
        }

        let graph = graph.complete_loading()?;

        for (projid, user_facing) in ids {
            assert_eq!(graph.lookup(projid).user_facing_name, *user_facing);
        }

        Ok(())
    }

    #[test]
    fn name_assignment_1() {
        do_name_assignment_test(&[(&["A", "B"], "A")]).expect("BUG: test should succeed");
    }

    #[test]
    fn name_assignment_2() {
        do_name_assignment_test(&[(&["A", "B"], "B:A"), (&["A", "C"], "C:A")])
            .expect("BUG: test should succeed");
    }

    #[test]
    fn name_assignment_3() {
        do_name_assignment_test(&[
            (&["A", "B"], "B:A"),
            (&["A", "C"], "C:A"),
            (&["D", "B"], "D"),
            (&["E"], "E"),
        ])
        .expect("BUG: test should succeed");
    }

    #[test]
    fn name_assignment_4() {
        do_name_assignment_test(&[(&["A", "A"], "A:A"), (&["A"], "A")])
            .expect("BUG: test should succeed");
    }

    #[test]
    fn name_assignment_5() {
        do_name_assignment_test(&[
            (&["A"], "A"),
            (&["A", "B"], "B:A"),
            (&["A", "B", "C"], "C:B:A"),
            (&["A", "B", "C", "D"], "D:C:B:A"),
        ])
        .expect("BUG: test should succeed");
    }

    fn create_test_project(graph: &mut ProjectGraphBuilder, name: &str) -> ProjectId {
        let empty_config = HashMap::new();
        let qnames = vec![name.to_owned(), "test".to_owned()];
        let projid = graph
            .try_add_project(qnames, &empty_config)
            .expect("BUG: test project should be added successfully");
        let b = graph.lookup_mut(projid);
        b.version = Some(Version::Semver(semver::Version::new(0, 0, 0)));
        b.prefix = Some(RepoPathBuf::new(name.as_bytes()));
        projid
    }

    #[test]
    fn cycle_detection_simple_two_node() {
        use crate::core::project::{DepRequirement, DependencyTarget};

        let mut graph = ProjectGraphBuilder::new();

        let proj_a = create_test_project(&mut graph, "A");
        let proj_b = create_test_project(&mut graph, "B");

        graph.add_dependency(
            proj_a,
            DependencyTarget::Ident(proj_b),
            "0.0.0-dev.0".to_string(),
            DepRequirement::Manual("^0.1".to_string()),
        );
        graph.add_dependency(
            proj_b,
            DependencyTarget::Ident(proj_a),
            "0.0.0-dev.0".to_string(),
            DepRequirement::Manual("^0.1".to_string()),
        );

        let result = graph.complete_loading();
        assert!(result.is_err());

        let err = result.unwrap_err();
        let cycle_err = err.downcast_ref::<DependencyCycleError>();
        assert!(
            cycle_err.is_some(),
            "expected DependencyCycleError, got: {:?}",
            err
        );
    }

    #[test]
    fn cycle_detection_self_referential() {
        use crate::core::project::{DepRequirement, DependencyTarget};

        let mut graph = ProjectGraphBuilder::new();

        let proj_a = create_test_project(&mut graph, "A");

        graph.add_dependency(
            proj_a,
            DependencyTarget::Ident(proj_a),
            "0.0.0-dev.0".to_string(),
            DepRequirement::Manual("^0.1".to_string()),
        );

        let result = graph.complete_loading();
        assert!(result.is_err());

        let err = result.unwrap_err();
        let cycle_err = err.downcast_ref::<DependencyCycleError>();
        assert!(
            cycle_err.is_some(),
            "expected DependencyCycleError, got: {:?}",
            err
        );
    }

    #[test]
    fn cycle_detection_three_node() {
        use crate::core::project::{DepRequirement, DependencyTarget};

        let mut graph = ProjectGraphBuilder::new();

        let proj_a = create_test_project(&mut graph, "A");
        let proj_b = create_test_project(&mut graph, "B");
        let proj_c = create_test_project(&mut graph, "C");

        graph.add_dependency(
            proj_a,
            DependencyTarget::Ident(proj_b),
            "0.0.0-dev.0".to_string(),
            DepRequirement::Manual("^0.1".to_string()),
        );
        graph.add_dependency(
            proj_b,
            DependencyTarget::Ident(proj_c),
            "0.0.0-dev.0".to_string(),
            DepRequirement::Manual("^0.1".to_string()),
        );
        graph.add_dependency(
            proj_c,
            DependencyTarget::Ident(proj_a),
            "0.0.0-dev.0".to_string(),
            DepRequirement::Manual("^0.1".to_string()),
        );

        let result = graph.complete_loading();
        assert!(result.is_err());

        let err = result.unwrap_err();
        let cycle_err = err.downcast_ref::<DependencyCycleError>();
        assert!(
            cycle_err.is_some(),
            "expected DependencyCycleError, got: {:?}",
            err
        );
    }

    #[test]
    fn cycle_detection_valid_linear_chain() {
        use crate::core::project::{DepRequirement, DependencyTarget};

        let mut graph = ProjectGraphBuilder::new();

        let proj_a = create_test_project(&mut graph, "A");
        let proj_b = create_test_project(&mut graph, "B");
        let proj_c = create_test_project(&mut graph, "C");

        graph.add_dependency(
            proj_a,
            DependencyTarget::Ident(proj_b),
            "0.0.0-dev.0".to_string(),
            DepRequirement::Manual("^0.1".to_string()),
        );
        graph.add_dependency(
            proj_b,
            DependencyTarget::Ident(proj_c),
            "0.0.0-dev.0".to_string(),
            DepRequirement::Manual("^0.1".to_string()),
        );

        let result = graph.complete_loading();
        assert!(
            result.is_ok(),
            "valid linear chain should not be a cycle: {:?}",
            result.err()
        );

        let graph = result.unwrap();
        let sorted: Vec<_> = graph.toposorted().collect();
        assert_eq!(sorted.len(), 3);
    }

    #[test]
    fn cycle_detection_valid_diamond() {
        use crate::core::project::{DepRequirement, DependencyTarget};

        let mut graph = ProjectGraphBuilder::new();

        let proj_a = create_test_project(&mut graph, "A");
        let proj_b = create_test_project(&mut graph, "B");
        let proj_c = create_test_project(&mut graph, "C");
        let proj_d = create_test_project(&mut graph, "D");

        graph.add_dependency(
            proj_a,
            DependencyTarget::Ident(proj_b),
            "0.0.0-dev.0".to_string(),
            DepRequirement::Manual("^0.1".to_string()),
        );
        graph.add_dependency(
            proj_a,
            DependencyTarget::Ident(proj_c),
            "0.0.0-dev.0".to_string(),
            DepRequirement::Manual("^0.1".to_string()),
        );
        graph.add_dependency(
            proj_b,
            DependencyTarget::Ident(proj_d),
            "0.0.0-dev.0".to_string(),
            DepRequirement::Manual("^0.1".to_string()),
        );
        graph.add_dependency(
            proj_c,
            DependencyTarget::Ident(proj_d),
            "0.0.0-dev.0".to_string(),
            DepRequirement::Manual("^0.1".to_string()),
        );

        let result = graph.complete_loading();
        assert!(
            result.is_ok(),
            "valid diamond pattern should not be a cycle: {:?}",
            result.err()
        );

        let graph = result.unwrap();
        let sorted: Vec<_> = graph.toposorted().collect();
        assert_eq!(sorted.len(), 4);

        let d_pos = sorted.iter().position(|&id| id == proj_d).unwrap();
        let b_pos = sorted.iter().position(|&id| id == proj_b).unwrap();
        let c_pos = sorted.iter().position(|&id| id == proj_c).unwrap();
        let a_pos = sorted.iter().position(|&id| id == proj_a).unwrap();

        assert!(d_pos < b_pos, "D should come before B in toposort");
        assert!(d_pos < c_pos, "D should come before C in toposort");
        assert!(b_pos < a_pos, "B should come before A in toposort");
        assert!(c_pos < a_pos, "C should come before A in toposort");
    }

    #[test]
    fn cycle_detection_independent_projects() {
        let mut graph = ProjectGraphBuilder::new();

        create_test_project(&mut graph, "A");
        create_test_project(&mut graph, "B");
        create_test_project(&mut graph, "C");

        let result = graph.complete_loading();
        assert!(
            result.is_ok(),
            "independent projects should not be a cycle: {:?}",
            result.err()
        );

        let graph = result.unwrap();
        let sorted: Vec<_> = graph.toposorted().collect();
        assert_eq!(sorted.len(), 3);
    }

    #[test]
    fn cycle_detection_complex_valid_dag() {
        use crate::core::project::{DepRequirement, DependencyTarget};

        let mut graph = ProjectGraphBuilder::new();

        let proj_a = create_test_project(&mut graph, "A");
        let proj_b = create_test_project(&mut graph, "B");
        let proj_c = create_test_project(&mut graph, "C");
        let proj_d = create_test_project(&mut graph, "D");
        let proj_e = create_test_project(&mut graph, "E");

        graph.add_dependency(
            proj_a,
            DependencyTarget::Ident(proj_b),
            "0.0.0-dev.0".to_string(),
            DepRequirement::Manual("^0.1".to_string()),
        );
        graph.add_dependency(
            proj_a,
            DependencyTarget::Ident(proj_c),
            "0.0.0-dev.0".to_string(),
            DepRequirement::Manual("^0.1".to_string()),
        );
        graph.add_dependency(
            proj_b,
            DependencyTarget::Ident(proj_d),
            "0.0.0-dev.0".to_string(),
            DepRequirement::Manual("^0.1".to_string()),
        );
        graph.add_dependency(
            proj_c,
            DependencyTarget::Ident(proj_d),
            "0.0.0-dev.0".to_string(),
            DepRequirement::Manual("^0.1".to_string()),
        );
        graph.add_dependency(
            proj_c,
            DependencyTarget::Ident(proj_e),
            "0.0.0-dev.0".to_string(),
            DepRequirement::Manual("^0.1".to_string()),
        );
        graph.add_dependency(
            proj_d,
            DependencyTarget::Ident(proj_e),
            "0.0.0-dev.0".to_string(),
            DepRequirement::Manual("^0.1".to_string()),
        );

        let result = graph.complete_loading();
        assert!(
            result.is_ok(),
            "complex valid DAG should not be a cycle: {:?}",
            result.err()
        );

        let graph = result.unwrap();
        let sorted: Vec<_> = graph.toposorted().collect();
        assert_eq!(sorted.len(), 5);

        let e_pos = sorted.iter().position(|&id| id == proj_e).unwrap();
        let d_pos = sorted.iter().position(|&id| id == proj_d).unwrap();
        let a_pos = sorted.iter().position(|&id| id == proj_a).unwrap();

        assert!(e_pos < d_pos, "E should come before D in toposort");
        assert!(d_pos < a_pos, "D should come before A in toposort");
    }

    #[test]
    fn cycle_detection_partial_cycle_in_larger_graph() {
        use crate::core::project::{DepRequirement, DependencyTarget};

        let mut graph = ProjectGraphBuilder::new();

        let proj_a = create_test_project(&mut graph, "A");
        let proj_b = create_test_project(&mut graph, "B");
        let proj_c = create_test_project(&mut graph, "C");
        let proj_d = create_test_project(&mut graph, "D");
        let proj_e = create_test_project(&mut graph, "E");

        graph.add_dependency(
            proj_a,
            DependencyTarget::Ident(proj_b),
            "0.0.0-dev.0".to_string(),
            DepRequirement::Manual("^0.1".to_string()),
        );
        graph.add_dependency(
            proj_b,
            DependencyTarget::Ident(proj_c),
            "0.0.0-dev.0".to_string(),
            DepRequirement::Manual("^0.1".to_string()),
        );
        graph.add_dependency(
            proj_c,
            DependencyTarget::Ident(proj_b),
            "0.0.0-dev.0".to_string(),
            DepRequirement::Manual("^0.1".to_string()),
        );
        graph.add_dependency(
            proj_d,
            DependencyTarget::Ident(proj_e),
            "0.0.0-dev.0".to_string(),
            DepRequirement::Manual("^0.1".to_string()),
        );

        let result = graph.complete_loading();
        assert!(
            result.is_err(),
            "graph with partial cycle should be detected"
        );

        let err = result.unwrap_err();
        let cycle_err = err.downcast_ref::<DependencyCycleError>();
        assert!(
            cycle_err.is_some(),
            "expected DependencyCycleError, got: {:?}",
            err
        );
    }

    #[test]
    fn dependency_resolution_multiple_paths() {
        use crate::core::project::{DepRequirement, DependencyTarget};

        let mut graph = ProjectGraphBuilder::new();

        let leaf = create_test_project(&mut graph, "leaf");
        let mid_a = create_test_project(&mut graph, "mid-a");
        let mid_b = create_test_project(&mut graph, "mid-b");
        let root = create_test_project(&mut graph, "root");

        graph.add_dependency(
            mid_a,
            DependencyTarget::Ident(leaf),
            "0.0.0-dev.0".to_string(),
            DepRequirement::Manual("^0.1".to_string()),
        );
        graph.add_dependency(
            mid_b,
            DependencyTarget::Ident(leaf),
            "0.0.0-dev.0".to_string(),
            DepRequirement::Manual("^0.1".to_string()),
        );
        graph.add_dependency(
            root,
            DependencyTarget::Ident(mid_a),
            "0.0.0-dev.0".to_string(),
            DepRequirement::Manual("^0.1".to_string()),
        );
        graph.add_dependency(
            root,
            DependencyTarget::Ident(mid_b),
            "0.0.0-dev.0".to_string(),
            DepRequirement::Manual("^0.1".to_string()),
        );

        let result = graph.complete_loading();
        assert!(
            result.is_ok(),
            "multiple paths should resolve: {:?}",
            result.err()
        );

        let graph = result.unwrap();
        let sorted: Vec<_> = graph.toposorted().collect();
        assert_eq!(sorted.len(), 4);

        let leaf_pos = sorted.iter().position(|&id| id == leaf).unwrap();
        let mid_a_pos = sorted.iter().position(|&id| id == mid_a).unwrap();
        let mid_b_pos = sorted.iter().position(|&id| id == mid_b).unwrap();
        let root_pos = sorted.iter().position(|&id| id == root).unwrap();

        assert!(leaf_pos < mid_a_pos, "leaf before mid_a");
        assert!(leaf_pos < mid_b_pos, "leaf before mid_b");
        assert!(mid_a_pos < root_pos, "mid_a before root");
        assert!(mid_b_pos < root_pos, "mid_b before root");
    }

    #[test]
    fn dependency_resolution_deep_chain() {
        use crate::core::project::{DepRequirement, DependencyTarget};

        let mut graph = ProjectGraphBuilder::new();

        let names = ["p0", "p1", "p2", "p3", "p4", "p5", "p6", "p7", "p8", "p9"];
        let mut projects: Vec<ProjectId> = Vec::new();

        for name in &names {
            projects.push(create_test_project(&mut graph, name));
        }

        for i in 1..projects.len() {
            graph.add_dependency(
                projects[i],
                DependencyTarget::Ident(projects[i - 1]),
                "0.0.0-dev.0".to_string(),
                DepRequirement::Manual("^0.1".to_string()),
            );
        }

        let result = graph.complete_loading();
        assert!(
            result.is_ok(),
            "deep chain should resolve: {:?}",
            result.err()
        );

        let graph = result.unwrap();
        let sorted: Vec<_> = graph.toposorted().collect();
        assert_eq!(sorted.len(), 10);

        for i in 0..9 {
            let curr_pos = sorted.iter().position(|&id| id == projects[i]).unwrap();
            let next_pos = sorted.iter().position(|&id| id == projects[i + 1]).unwrap();
            assert!(curr_pos < next_pos, "p{} should come before p{}", i, i + 1);
        }
    }

    #[test]
    fn dependency_resolution_many_to_many() {
        use crate::core::project::{DepRequirement, DependencyTarget};

        let mut graph = ProjectGraphBuilder::new();

        let leaves = [
            create_test_project(&mut graph, "leaf-a"),
            create_test_project(&mut graph, "leaf-b"),
            create_test_project(&mut graph, "leaf-c"),
        ];

        let roots = [
            create_test_project(&mut graph, "root-x"),
            create_test_project(&mut graph, "root-y"),
            create_test_project(&mut graph, "root-z"),
        ];

        for root in &roots {
            for leaf in &leaves {
                graph.add_dependency(
                    *root,
                    DependencyTarget::Ident(*leaf),
                    "0.0.0-dev.0".to_string(),
                    DepRequirement::Manual("^0.1".to_string()),
                );
            }
        }

        let result = graph.complete_loading();
        assert!(
            result.is_ok(),
            "many-to-many should resolve: {:?}",
            result.err()
        );

        let graph = result.unwrap();
        let sorted: Vec<_> = graph.toposorted().collect();
        assert_eq!(sorted.len(), 6);

        for root in &roots {
            let root_pos = sorted.iter().position(|&id| id == *root).unwrap();
            for leaf in &leaves {
                let leaf_pos = sorted.iter().position(|&id| id == *leaf).unwrap();
                assert!(
                    leaf_pos < root_pos,
                    "all leaves should come before all roots"
                );
            }
        }
    }

    #[test]
    fn dependency_resolution_multiple_roots() {
        use crate::core::project::{DepRequirement, DependencyTarget};

        let mut graph = ProjectGraphBuilder::new();

        let shared = create_test_project(&mut graph, "shared");
        let root_a = create_test_project(&mut graph, "root-a");
        let root_b = create_test_project(&mut graph, "root-b");
        let isolated = create_test_project(&mut graph, "isolated");

        graph.add_dependency(
            root_a,
            DependencyTarget::Ident(shared),
            "0.0.0-dev.0".to_string(),
            DepRequirement::Manual("^0.1".to_string()),
        );
        graph.add_dependency(
            root_b,
            DependencyTarget::Ident(shared),
            "0.0.0-dev.0".to_string(),
            DepRequirement::Manual("^0.1".to_string()),
        );

        let result = graph.complete_loading();
        assert!(
            result.is_ok(),
            "multiple roots should resolve: {:?}",
            result.err()
        );

        let graph = result.unwrap();
        let sorted: Vec<_> = graph.toposorted().collect();
        assert_eq!(sorted.len(), 4);

        let shared_pos = sorted.iter().position(|&id| id == shared).unwrap();
        let root_a_pos = sorted.iter().position(|&id| id == root_a).unwrap();
        let root_b_pos = sorted.iter().position(|&id| id == root_b).unwrap();
        let _isolated_pos = sorted.iter().position(|&id| id == isolated).unwrap();

        assert!(shared_pos < root_a_pos, "shared before root_a");
        assert!(shared_pos < root_b_pos, "shared before root_b");
    }

    #[test]
    fn dependency_resolution_parallel_chains() {
        use crate::core::project::{DepRequirement, DependencyTarget};

        let mut graph = ProjectGraphBuilder::new();

        let chain_a = [
            create_test_project(&mut graph, "chain-a-1"),
            create_test_project(&mut graph, "chain-a-2"),
            create_test_project(&mut graph, "chain-a-3"),
        ];
        let chain_b = [
            create_test_project(&mut graph, "chain-b-1"),
            create_test_project(&mut graph, "chain-b-2"),
            create_test_project(&mut graph, "chain-b-3"),
        ];

        for i in 1..chain_a.len() {
            graph.add_dependency(
                chain_a[i],
                DependencyTarget::Ident(chain_a[i - 1]),
                "0.0.0-dev.0".to_string(),
                DepRequirement::Manual("^0.1".to_string()),
            );
        }

        for i in 1..chain_b.len() {
            graph.add_dependency(
                chain_b[i],
                DependencyTarget::Ident(chain_b[i - 1]),
                "0.0.0-dev.0".to_string(),
                DepRequirement::Manual("^0.1".to_string()),
            );
        }

        let result = graph.complete_loading();
        assert!(
            result.is_ok(),
            "parallel chains should resolve: {:?}",
            result.err()
        );

        let graph = result.unwrap();
        let sorted: Vec<_> = graph.toposorted().collect();
        assert_eq!(sorted.len(), 6);

        for chain in [&chain_a, &chain_b] {
            for i in 0..2 {
                let curr_pos = sorted.iter().position(|&id| id == chain[i]).unwrap();
                let next_pos = sorted.iter().position(|&id| id == chain[i + 1]).unwrap();
                assert!(curr_pos < next_pos, "chain order should be preserved");
            }
        }
    }

    #[test]
    fn dependency_lookup_by_name() {
        let mut graph = ProjectGraphBuilder::new();

        create_test_project(&mut graph, "alpha");
        create_test_project(&mut graph, "beta");
        create_test_project(&mut graph, "gamma");

        let result = graph.complete_loading();
        assert!(result.is_ok());

        let graph = result.unwrap();

        let alpha_id = graph.lookup_ident("alpha");
        let beta_id = graph.lookup_ident("beta");
        let gamma_id = graph.lookup_ident("gamma");
        let nonexistent = graph.lookup_ident("nonexistent");

        assert!(alpha_id.is_some());
        assert!(beta_id.is_some());
        assert!(gamma_id.is_some());
        assert!(nonexistent.is_none());

        assert_ne!(alpha_id, beta_id);
        assert_ne!(beta_id, gamma_id);
        assert_ne!(alpha_id, gamma_id);
    }

    #[test]
    fn dependency_internal_deps_stored_correctly() {
        use crate::core::project::{DepRequirement, DependencyTarget};

        let mut graph = ProjectGraphBuilder::new();

        let dep = create_test_project(&mut graph, "dependency");
        let consumer = create_test_project(&mut graph, "consumer");

        graph.add_dependency(
            consumer,
            DependencyTarget::Ident(dep),
            "0.0.0-dev.0".to_string(),
            DepRequirement::Manual(">=1.0.0".to_string()),
        );

        let result = graph.complete_loading();
        assert!(result.is_ok());

        let graph = result.unwrap();
        let consumer_proj = graph.lookup(consumer);

        assert_eq!(consumer_proj.internal_deps.len(), 1);
        assert_eq!(consumer_proj.internal_deps[0].ident, dep);
        assert_eq!(consumer_proj.internal_deps[0].literal, "0.0.0-dev.0");

        match &consumer_proj.internal_deps[0].belaf_requirement {
            DepRequirement::Manual(s) => assert_eq!(s, ">=1.0.0"),
            _ => panic!("Expected Manual requirement"),
        }
    }
}
