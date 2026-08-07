//! Which repo paths the configured `[release_unit.X]` blocks already cover.
//!
//! Auto-discovery has to know this for two reasons, and they are not the same
//! reason:
//!
//! 1. **Don't re-emit.** A package a configured unit covers must not also
//!    become an auto-discovered unit — two graph nodes on one directory, and
//!    when the unit name equals the package name the two are indistinguishable
//!    and the naming round fails outright.
//!
//! 2. **Re-address the edges.** The covered package still has dependencies,
//!    and they belong to the unit that covers it. A hexagonal service spanning
//!    a bin crate and a tree of satellites collects the dependencies of all of
//!    them; edges *between* its own crates collapse to nothing.
//!
//! The path-level skip-list only ever solved (1), and only for manifests the
//! repo walk dispatches one at a time. A workspace protocol enumerates every
//! member from a single call on the workspace root, so those members never
//! pass the walk's filter at all.

use crate::core::ecosystem::format_handler::UnitOwnership;
use crate::core::git::repository::RepoPathBuf;
use crate::core::release_unit::{ResolvedReleaseUnit, VersionSource};

/// Whether a `paths = [...]` entry is a glob pattern rather than a literal
/// directory prefix. Conservative: any of `*`, `?`, `[`.
fn is_glob(s: &str) -> bool {
    s.contains(['*', '?', '['])
}

/// Build the ownership map from the resolved config.
///
/// Claims come from three places on each unit: the directory of every entry in
/// `manifests`, every `satellites` prefix, and the literal entries of a
/// manifest-less `paths = [...]` unit. Glob-shaped `paths` entries claim
/// nothing here — they are matched against changed paths after the fact and
/// have no directory to hand out.
///
/// `[ignore_paths]` entries are claimed with no owner: nothing inside them is
/// a release unit, and nothing inside them is a valid dependency target
/// either.
pub fn ownership_for(resolved: &[ResolvedReleaseUnit], ignore_paths: &[String]) -> UnitOwnership {
    let mut claims: Vec<(RepoPathBuf, Option<String>)> = Vec::new();

    for r in resolved {
        let name = &r.unit.name;

        match &r.unit.source {
            VersionSource::Manifests(manifests) => {
                for m in manifests {
                    let escaped = m.path.escaped().to_string();
                    let Some(parent) = std::path::Path::new(&escaped).parent() else {
                        continue;
                    };
                    let parent_str = parent.to_string_lossy().to_string();
                    // A manifest at the repo root has an empty parent, which
                    // would claim the whole tree.
                    if parent_str.is_empty() {
                        continue;
                    }
                    claims.push((RepoPathBuf::new(parent_str.as_bytes()), Some(name.clone())));
                }
            }
            VersionSource::PathsOnly(paths) => {
                for p in paths {
                    if !is_glob(&p.escaped()) {
                        claims.push((p.clone(), Some(name.clone())));
                    }
                }
            }
            // An external versioner owns no manifest belaf reads; its
            // satellites below are the only paths it speaks for.
            VersionSource::External(_) => {}
        }

        for sat in &r.unit.satellites {
            claims.push((sat.clone(), Some(name.clone())));
        }
    }

    for p in ignore_paths {
        claims.push((RepoPathBuf::new(p.trim_end_matches('/').as_bytes()), None));
    }

    UnitOwnership::new(claims)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ecosystem::format_handler::PathOwner;
    use crate::core::release_unit::{
        ManifestFile, ReleaseUnit, ResolveOrigin, VersionFieldSpec, Visibility,
    };
    use crate::core::resolved_release_unit::UnitKind;
    use crate::core::wire::known::Ecosystem;

    fn unit(name: &str, source: VersionSource, satellites: Vec<&str>) -> ResolvedReleaseUnit {
        ResolvedReleaseUnit {
            unit: ReleaseUnit {
                name: name.to_owned(),
                ecosystem: Ecosystem::classify("cargo"),
                source,
                satellites: satellites
                    .into_iter()
                    .map(|s| RepoPathBuf::new(s.as_bytes()))
                    .collect(),
                tag_format: None,
                visibility: Visibility::Public,
                cascade_from: None,
                kind: UnitKind::Deploy,
                bump_override: None,
                baseline: None,
            },
            origin: ResolveOrigin::Explicit { config_index: 0 },
        }
    }

    fn manifests(paths: &[&str]) -> VersionSource {
        VersionSource::Manifests(
            paths
                .iter()
                .map(|p| ManifestFile {
                    path: RepoPathBuf::new(p.as_bytes()),
                    ecosystem: Ecosystem::classify("cargo"),
                    version_field: VersionFieldSpec::CargoToml,
                })
                .collect(),
        )
    }

    fn owner<'a>(o: &'a UnitOwnership, path: &str) -> Option<PathOwner<'a>> {
        o.owner_of(&RepoPathBuf::new(path.as_bytes()))
    }

    #[test]
    fn manifest_directory_and_satellites_belong_to_the_unit() {
        let o = ownership_for(
            &[unit(
                "gate",
                manifests(&["apps/services/gate/crates/bin/Cargo.toml"]),
                vec!["apps/services/gate/crates"],
            )],
            &[],
        );

        assert_eq!(
            owner(&o, "apps/services/gate/crates/bin/Cargo.toml"),
            Some(PathOwner::Unit("gate"))
        );
        // A satellite crate deep under the satellite prefix.
        assert_eq!(
            owner(&o, "apps/services/gate/crates/api/Cargo.toml"),
            Some(PathOwner::Unit("gate"))
        );
        assert_eq!(owner(&o, "packages/observability/Cargo.toml"), None);
    }

    #[test]
    fn the_most_specific_claim_wins() {
        // A nested unit inside another unit's satellite tree keeps its own
        // packages: resolving to the outer unit would fold two releases into
        // one.
        let o = ownership_for(
            &[
                unit("outer", manifests(&["apps/outer/Cargo.toml"]), vec!["apps"]),
                unit(
                    "inner",
                    manifests(&["apps/outer/nested/Cargo.toml"]),
                    vec![],
                ),
            ],
            &[],
        );

        assert_eq!(
            owner(&o, "apps/outer/nested/Cargo.toml"),
            Some(PathOwner::Unit("inner"))
        );
        assert_eq!(
            owner(&o, "apps/outer/src/lib.rs"),
            Some(PathOwner::Unit("outer"))
        );
    }

    #[test]
    fn paths_only_units_claim_literals_but_not_globs() {
        let o = ownership_for(
            &[unit(
                "e2e",
                VersionSource::PathsOnly(vec![
                    RepoPathBuf::new(b"apps/services/e2e"),
                    RepoPathBuf::new(b"tests/**"),
                ]),
                vec![],
            )],
            &[],
        );

        assert_eq!(
            owner(&o, "apps/services/e2e/Cargo.toml"),
            Some(PathOwner::Unit("e2e"))
        );
        // The glob is matched against changed paths later; claiming the
        // literal string `tests/**` as a directory would match nothing.
        assert_eq!(owner(&o, "tests/foo/Cargo.toml"), None);
    }

    #[test]
    fn ignore_paths_are_claimed_without_an_owner() {
        let o = ownership_for(&[], &["bazel-out/".to_owned()]);
        assert_eq!(
            owner(&o, "bazel-out/gen/Cargo.toml"),
            Some(PathOwner::Ignored)
        );
    }

    #[test]
    fn a_root_manifest_does_not_claim_the_whole_repo() {
        let o = ownership_for(&[unit("root", manifests(&["Cargo.toml"]), vec![])], &[]);
        assert_eq!(owner(&o, "packages/thing/Cargo.toml"), None);
    }
}
