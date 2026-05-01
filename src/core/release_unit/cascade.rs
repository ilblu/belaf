//! Cascade resolution: SDK ReleaseUnits with `cascade_from` follow
//! their named source unit's bump per a [`CascadeBumpStrategy`].
//!
//! Phase G of `BELAF_MASTER_PLAN.md`. Two pieces:
//!
//! 1. **Cycle detection** via [`petgraph::algo::tarjan_scc`] —
//!    rejects `A → B → A`-style cycles at config-load time, listing
//!    all members in the error so the user can fix it.
//! 2. **Topological cascade** — given a map of "primary" bumps
//!    (from conventional commits / `--project` / `[[bump_source]]`),
//!    propagate them through the cascade DAG and produce a final
//!    bump decision per unit.

use std::collections::HashMap;

use petgraph::{algo::tarjan_scc, graph::DiGraph, visit::EdgeRef};

use super::validator::ResolverError;
use super::{CascadeBumpStrategy, ResolvedReleaseUnit};

/// Bump granularity, ordered by rank.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BumpKind {
    NoBump,
    Prerelease,
    Patch,
    Minor,
    Major,
}

impl BumpKind {
    fn rank(self) -> u8 {
        match self {
            Self::NoBump => 0,
            Self::Prerelease => 1,
            Self::Patch => 2,
            Self::Minor => 3,
            Self::Major => 4,
        }
    }

    fn max(self, other: Self) -> Self {
        if self.rank() >= other.rank() {
            self
        } else {
            other
        }
    }
}

/// Apply a [`CascadeBumpStrategy`] given the source unit's actual
/// bump.
fn cascaded(strategy: CascadeBumpStrategy, source: BumpKind) -> BumpKind {
    if matches!(source, BumpKind::NoBump) {
        // Source didn't bump → cascade does nothing.
        return BumpKind::NoBump;
    }
    match strategy {
        CascadeBumpStrategy::Mirror => source,
        CascadeBumpStrategy::FloorPatch => source.max(BumpKind::Patch),
        CascadeBumpStrategy::FloorMinor => source.max(BumpKind::Minor),
        CascadeBumpStrategy::FloorMajor => BumpKind::Major,
    }
}

/// Build a `petgraph` directed graph of `cascade_from` edges, with
/// each edge weighted by the cascade bump strategy. Used by both
/// [`validate_no_cycles`] (which only needs structure) and
/// [`apply_cascades`] (which needs the weights). Extracted so we
/// build the graph exactly once per `apply_cascades` call.
fn build_cascade_graph(units: &[ResolvedReleaseUnit]) -> DiGraph<String, CascadeBumpStrategy> {
    let mut graph: DiGraph<String, CascadeBumpStrategy> = DiGraph::new();
    let mut node_idx: HashMap<String, _> = HashMap::new();
    for r in units {
        let idx = graph.add_node(r.unit.name.clone());
        node_idx.insert(r.unit.name.clone(), idx);
    }
    // Edge: source → cascading unit. Source must exist (resolver's
    // validate_cascade_sources catches the unknown-source case).
    for r in units {
        if let Some(c) = &r.unit.cascade_from {
            if let (Some(&src), Some(&dst)) = (node_idx.get(&c.source), node_idx.get(&r.unit.name))
            {
                graph.add_edge(src, dst, c.bump);
            }
        }
    }
    graph
}

/// Run Tarjan-SCC on the cascade graph. Any SCC with size > 1 is a
/// cycle; the error names every member so the user can locate them
/// all. Self-loops (single-node SCCs with a self-edge) also fail.
pub fn validate_no_cycles(units: &[ResolvedReleaseUnit]) -> Result<(), ResolverError> {
    let graph = build_cascade_graph(units);
    validate_no_cycles_on(&graph)
}

fn validate_no_cycles_on(
    graph: &DiGraph<String, CascadeBumpStrategy>,
) -> Result<(), ResolverError> {
    for scc in tarjan_scc(graph) {
        if scc.len() > 1 {
            let mut members: Vec<String> = scc.iter().map(|i| graph[*i].clone()).collect();
            members.sort();
            return Err(ResolverError::CascadeCycle { members });
        }
        if scc.len() == 1 {
            let n = scc[0];
            if graph.contains_edge(n, n) {
                return Err(ResolverError::CascadeCycle {
                    members: vec![graph[n].clone()],
                });
            }
        }
    }
    Ok(())
}

/// Apply cascade rules. Inputs:
///
/// - `units`: the resolved release units (each carries its
///   `cascade_from`)
/// - `primary_bumps`: name → primary bump decision (from conventional
///   commits / explicit `--project` / `[[bump_source]]`). Units NOT
///   in this map are treated as `NoBump` initially; cascade rules
///   may still bump them via their source.
///
/// Returns: final name → BumpKind for every unit.
///
/// Topological order: roots first, leaves last. We process roots
/// with their primary bumps fixed, then for each cascade-rooted
/// unit we apply the strategy on top of the source's already-decided
/// bump. **Primary bumps win over cascade**: if `primary_bumps`
/// already names a bump for a unit, that bump is kept (cascade can
/// only escalate, not override downward).
pub fn apply_cascades(
    units: &[ResolvedReleaseUnit],
    primary_bumps: &HashMap<String, BumpKind>,
) -> Result<HashMap<String, BumpKind>, ResolverError> {
    // One graph build for both cycle detection AND the topological
    // walk below. Previously we built two identical-shape graphs back
    // to back — wasted O(V+E) every time `apply_cascades` ran.
    let graph = build_cascade_graph(units);
    validate_no_cycles_on(&graph)?;

    // Topological order: petgraph's toposort returns Err on cycles,
    // but we already rejected cycles above.
    let order = match petgraph::algo::toposort(&graph, None) {
        Ok(o) => o,
        Err(_) => unreachable!("validate_no_cycles must catch cycles before this point"),
    };

    let mut decisions: HashMap<String, BumpKind> = HashMap::new();
    for idx in order {
        let name = &graph[idx];
        let primary = primary_bumps.get(name).copied().unwrap_or(BumpKind::NoBump);

        // Aggregate all incoming cascade contributions.
        let mut cascaded_bump = BumpKind::NoBump;
        for in_edge in graph.edges_directed(idx, petgraph::Direction::Incoming) {
            let src_idx = in_edge.source();
            let src_name = &graph[src_idx];
            let src_bump = decisions.get(src_name).copied().unwrap_or(BumpKind::NoBump);
            let strategy = *in_edge.weight();
            cascaded_bump = cascaded_bump.max(cascaded(strategy, src_bump));
        }

        // Final = max(primary, cascaded). Primary always wins
        // tie-breaks at the same rank (no semantic difference, but
        // stable behaviour).
        let final_bump = primary.max(cascaded_bump);
        decisions.insert(name.clone(), final_bump);
    }

    Ok(decisions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::git::repository::RepoPathBuf;
    use crate::core::release_unit::{
        CascadeRule, ManifestFile, ReleaseUnit, ResolveOrigin, VersionFieldSpec, VersionSource,
        Visibility,
    };
    use crate::core::wire::known::Ecosystem;

    fn unit(name: &str, cascade: Option<(&str, CascadeBumpStrategy)>) -> ResolvedReleaseUnit {
        ResolvedReleaseUnit {
            unit: ReleaseUnit {
                name: name.to_string(),
                ecosystem: Ecosystem::classify("cargo"),
                source: VersionSource::Manifests(vec![ManifestFile {
                    path: RepoPathBuf::new(format!("{name}/Cargo.toml").as_bytes()),
                    ecosystem: Ecosystem::classify("cargo"),
                    version_field: VersionFieldSpec::CargoToml,
                }]),
                satellites: vec![],
                tag_format: None,
                visibility: Visibility::Public,
                cascade_from: cascade.map(|(src, bump)| CascadeRule {
                    source: src.to_string(),
                    bump,
                }),
            },
            origin: ResolveOrigin::Explicit { config_index: 0 },
        }
    }

    #[test]
    fn cascaded_strategies() {
        // Source minor → Mirror gives minor
        assert_eq!(
            cascaded(CascadeBumpStrategy::Mirror, BumpKind::Minor),
            BumpKind::Minor
        );
        // Source minor → FloorPatch escalates to minor (source is bigger)
        assert_eq!(
            cascaded(CascadeBumpStrategy::FloorPatch, BumpKind::Minor),
            BumpKind::Minor
        );
        // Source patch → FloorMinor escalates to minor (floor wins)
        assert_eq!(
            cascaded(CascadeBumpStrategy::FloorMinor, BumpKind::Patch),
            BumpKind::Minor
        );
        // Source major → FloorMinor escalates to major (source wins)
        assert_eq!(
            cascaded(CascadeBumpStrategy::FloorMinor, BumpKind::Major),
            BumpKind::Major
        );
        // Source patch → FloorMajor → major (always)
        assert_eq!(
            cascaded(CascadeBumpStrategy::FloorMajor, BumpKind::Patch),
            BumpKind::Major
        );
        // Source no-bump → cascade no-op
        assert_eq!(
            cascaded(CascadeBumpStrategy::Mirror, BumpKind::NoBump),
            BumpKind::NoBump
        );
    }

    #[test]
    fn validate_no_cycles_accepts_dag() {
        let units = vec![
            unit("schema", None),
            unit("sdk-ts", Some(("schema", CascadeBumpStrategy::FloorMinor))),
            unit(
                "sdk-kotlin",
                Some(("schema", CascadeBumpStrategy::FloorMinor)),
            ),
        ];
        validate_no_cycles(&units).unwrap();
    }

    #[test]
    fn validate_no_cycles_detects_two_node_cycle() {
        let units = vec![
            unit("a", Some(("b", CascadeBumpStrategy::Mirror))),
            unit("b", Some(("a", CascadeBumpStrategy::Mirror))),
        ];
        let err = validate_no_cycles(&units).unwrap_err();
        match err {
            ResolverError::CascadeCycle { members } => {
                assert_eq!(members, vec!["a", "b"]);
            }
            _ => panic!("expected CascadeCycle"),
        }
    }

    #[test]
    fn validate_no_cycles_detects_self_loop() {
        let units = vec![unit("a", Some(("a", CascadeBumpStrategy::Mirror)))];
        let err = validate_no_cycles(&units).unwrap_err();
        match err {
            ResolverError::CascadeCycle { members } => assert_eq!(members, vec!["a"]),
            _ => panic!(),
        }
    }

    #[test]
    fn apply_cascades_propagates_minor_to_three_sdks() {
        let units = vec![
            unit("schema", None),
            unit("sdk-ts", Some(("schema", CascadeBumpStrategy::FloorMinor))),
            unit(
                "sdk-kotlin",
                Some(("schema", CascadeBumpStrategy::FloorMinor)),
            ),
            unit(
                "sdk-swift",
                Some(("schema", CascadeBumpStrategy::FloorMinor)),
            ),
        ];
        let mut primaries = HashMap::new();
        primaries.insert("schema".to_string(), BumpKind::Minor);

        let decisions = apply_cascades(&units, &primaries).unwrap();
        assert_eq!(decisions.get("schema").copied(), Some(BumpKind::Minor));
        assert_eq!(decisions.get("sdk-ts").copied(), Some(BumpKind::Minor));
        assert_eq!(decisions.get("sdk-kotlin").copied(), Some(BumpKind::Minor));
        assert_eq!(decisions.get("sdk-swift").copied(), Some(BumpKind::Minor));
    }

    #[test]
    fn apply_cascades_floor_patch_escalates_to_major_when_source_major() {
        let units = vec![
            unit("schema", None),
            unit("sdk-ts", Some(("schema", CascadeBumpStrategy::FloorPatch))),
        ];
        let mut primaries = HashMap::new();
        primaries.insert("schema".to_string(), BumpKind::Major);

        let decisions = apply_cascades(&units, &primaries).unwrap();
        assert_eq!(decisions.get("sdk-ts").copied(), Some(BumpKind::Major));
    }

    #[test]
    fn apply_cascades_primary_bump_wins_over_lesser_cascade() {
        // schema bumps minor; sdk-ts has primary major (e.g. user
        // explicit override). Final sdk-ts must be major, not minor.
        let units = vec![
            unit("schema", None),
            unit("sdk-ts", Some(("schema", CascadeBumpStrategy::FloorMinor))),
        ];
        let mut primaries = HashMap::new();
        primaries.insert("schema".to_string(), BumpKind::Minor);
        primaries.insert("sdk-ts".to_string(), BumpKind::Major);

        let decisions = apply_cascades(&units, &primaries).unwrap();
        assert_eq!(decisions.get("sdk-ts").copied(), Some(BumpKind::Major));
    }

    #[test]
    fn apply_cascades_no_op_when_source_does_not_bump() {
        let units = vec![
            unit("schema", None),
            unit("sdk-ts", Some(("schema", CascadeBumpStrategy::FloorMinor))),
        ];
        // Empty primary_bumps → schema NoBump → cascade no-op
        let decisions = apply_cascades(&units, &HashMap::new()).unwrap();
        assert_eq!(decisions.get("schema").copied(), Some(BumpKind::NoBump));
        assert_eq!(decisions.get("sdk-ts").copied(), Some(BumpKind::NoBump));
    }

    #[test]
    fn apply_cascades_chain_a_to_b_to_c() {
        // schema → mid → leaf, each FloorMinor
        let units = vec![
            unit("schema", None),
            unit("mid", Some(("schema", CascadeBumpStrategy::Mirror))),
            unit("leaf", Some(("mid", CascadeBumpStrategy::FloorPatch))),
        ];
        let mut primaries = HashMap::new();
        primaries.insert("schema".to_string(), BumpKind::Minor);

        let decisions = apply_cascades(&units, &primaries).unwrap();
        assert_eq!(decisions.get("schema").copied(), Some(BumpKind::Minor));
        assert_eq!(decisions.get("mid").copied(), Some(BumpKind::Minor));
        // mid bumped minor → leaf escalates to minor (FloorPatch
        // accepts source's minor).
        assert_eq!(decisions.get("leaf").copied(), Some(BumpKind::Minor));
    }

    #[test]
    fn apply_cascades_rejects_cycles() {
        let units = vec![
            unit("a", Some(("b", CascadeBumpStrategy::Mirror))),
            unit("b", Some(("a", CascadeBumpStrategy::Mirror))),
        ];
        assert!(matches!(
            apply_cascades(&units, &HashMap::new()).unwrap_err(),
            ResolverError::CascadeCycle { .. }
        ));
    }
}
