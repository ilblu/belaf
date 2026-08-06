//! Unit tests for [`crate::core::graph`].
//!
//! Split out of `graph.rs` to keep that file under the 800-line ceiling;
//! declared there via `#[cfg(test)] #[path = "graph_tests.rs"] mod graph_tests;`
//! (same arrangement as `git/repository_tests.rs`).

use super::*;
use crate::core::{git::repository::RepoPathBuf, version::Version};

fn do_name_assignment_test(spec: &[(&[&str], &str)]) -> Result<()> {
    let mut graph = ReleaseUnitGraphBuilder::new();
    let mut ids = HashMap::new();

    for (qnames, user_facing) in spec {
        let qnames = qnames.iter().map(|s| (*s).to_owned()).collect();
        let projid = graph.add_project(qnames);
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

fn create_test_project(graph: &mut ReleaseUnitGraphBuilder, name: &str) -> ReleaseUnitId {
    let qnames = vec![name.to_owned(), "test".to_owned()];
    let projid = graph.add_project(qnames);
    let b = graph.lookup_mut(projid);
    b.version = Some(Version::Semver(semver::Version::new(0, 0, 0)));
    b.prefix = Some(RepoPathBuf::new(name.as_bytes()));
    projid
}

#[test]
fn cycle_detection_simple_two_node() {
    use crate::core::resolved_release_unit::{DepRequirement, DependencyTarget};

    let mut graph = ReleaseUnitGraphBuilder::new();

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
    use crate::core::resolved_release_unit::{DepRequirement, DependencyTarget};

    let mut graph = ReleaseUnitGraphBuilder::new();

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
    use crate::core::resolved_release_unit::{DepRequirement, DependencyTarget};

    let mut graph = ReleaseUnitGraphBuilder::new();

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
    use crate::core::resolved_release_unit::{DepRequirement, DependencyTarget};

    let mut graph = ReleaseUnitGraphBuilder::new();

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
    use crate::core::resolved_release_unit::{DepRequirement, DependencyTarget};

    let mut graph = ReleaseUnitGraphBuilder::new();

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
    let mut graph = ReleaseUnitGraphBuilder::new();

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
    use crate::core::resolved_release_unit::{DepRequirement, DependencyTarget};

    let mut graph = ReleaseUnitGraphBuilder::new();

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
    use crate::core::resolved_release_unit::{DepRequirement, DependencyTarget};

    let mut graph = ReleaseUnitGraphBuilder::new();

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
    use crate::core::resolved_release_unit::{DepRequirement, DependencyTarget};

    let mut graph = ReleaseUnitGraphBuilder::new();

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
    use crate::core::resolved_release_unit::{DepRequirement, DependencyTarget};

    let mut graph = ReleaseUnitGraphBuilder::new();

    let names = ["p0", "p1", "p2", "p3", "p4", "p5", "p6", "p7", "p8", "p9"];
    let mut projects: Vec<ReleaseUnitId> = Vec::new();

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
    use crate::core::resolved_release_unit::{DepRequirement, DependencyTarget};

    let mut graph = ReleaseUnitGraphBuilder::new();

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
    use crate::core::resolved_release_unit::{DepRequirement, DependencyTarget};

    let mut graph = ReleaseUnitGraphBuilder::new();

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
    use crate::core::resolved_release_unit::{DepRequirement, DependencyTarget};

    let mut graph = ReleaseUnitGraphBuilder::new();

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
    let mut graph = ReleaseUnitGraphBuilder::new();

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
    use crate::core::resolved_release_unit::{DepRequirement, DependencyTarget};

    let mut graph = ReleaseUnitGraphBuilder::new();

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

// -----------------------------------------------------------------------
// F2 — dependency-closure traversal (`closure`).
// -----------------------------------------------------------------------

fn dep(graph: &mut ReleaseUnitGraphBuilder, depender: ReleaseUnitId, dependee: ReleaseUnitId) {
    use crate::core::resolved_release_unit::{DepRequirement, DependencyTarget};
    graph.add_dependency(
        depender,
        DependencyTarget::Ident(dependee),
        "0.0.0-dev.0".to_string(),
        DepRequirement::Manual("^0.1".to_string()),
    );
}

#[test]
fn closure_includes_transitive_deps_and_dedups_diamond() {
    // A depends on B and C; both B and C depend on D (diamond). closure(A)
    // = {A, B, C, D} with D visited once.
    let mut graph = ReleaseUnitGraphBuilder::new();
    let a = create_test_project(&mut graph, "A");
    let b = create_test_project(&mut graph, "B");
    let c = create_test_project(&mut graph, "C");
    let d = create_test_project(&mut graph, "D");
    dep(&mut graph, a, b);
    dep(&mut graph, a, c);
    dep(&mut graph, b, d);
    dep(&mut graph, c, d);
    let graph = graph.complete_loading().unwrap();

    let mut cl = graph.closure(a);
    cl.sort_unstable();
    assert_eq!(cl, vec![a, b, c, d]);

    // A leaf's closure is just itself.
    assert_eq!(graph.closure(d), vec![d]);
}

#[test]
fn closure_follows_incoming_edges_not_dependents() {
    // A depends on B. closure(A) must include B (its dependency), and
    // closure(B) must NOT include A (B does not depend on A). This pins the
    // edge direction (Incoming = dependencies).
    let mut graph = ReleaseUnitGraphBuilder::new();
    let a = create_test_project(&mut graph, "A");
    let b = create_test_project(&mut graph, "B");
    dep(&mut graph, a, b);
    let graph = graph.complete_loading().unwrap();

    let mut ca = graph.closure(a);
    ca.sort_unstable();
    assert_eq!(ca, vec![a, b]);
    assert_eq!(graph.closure(b), vec![b]);
}

#[test]
fn closure_skips_ignore_units() {
    use crate::core::resolved_release_unit::UnitKind;
    // A depends on B (Internal) and E (Ignore). closure(A) includes B but
    // not E, and does not traverse into E.
    let mut graph = ReleaseUnitGraphBuilder::new();
    let a = create_test_project(&mut graph, "A");
    let b = create_test_project(&mut graph, "B");
    let e = create_test_project(&mut graph, "E");
    graph.lookup_mut(b).kind = UnitKind::Internal;
    graph.lookup_mut(e).kind = UnitKind::Ignore;
    dep(&mut graph, a, b);
    dep(&mut graph, a, e);
    let graph = graph.complete_loading().unwrap();

    let mut cl = graph.closure(a);
    cl.sort_unstable();
    assert_eq!(cl, vec![a, b]);
    // An Ignore unit is never a closure root either.
    assert!(graph.closure(e).is_empty());
}
