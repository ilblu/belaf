//! Cross-cutting validations that only make sense once the whole set of
//! units is resolved: name collisions, nested bundle anchors, and unknown
//! `cascade_from.source` names.
//!
//! [`unit_paths`] is here too because it is the shared answer to "which repo
//! paths does this unit claim" — used both to build the explicit-wins shadow
//! set during glob expansion and by the checks below.

use std::collections::{BTreeMap, BTreeSet};

use crate::core::release_unit::validator::ResolverError;
use crate::core::release_unit::{ReleaseUnit, ResolveOrigin, ResolvedReleaseUnit, VersionSource};

pub(super) fn unit_paths(unit: &ReleaseUnit) -> Vec<String> {
    let mut paths = Vec::new();
    match &unit.source {
        VersionSource::Manifests(ms) => {
            for m in ms {
                paths.push(m.path.escaped().to_string());
            }
        }
        // A `paths = [...]` unit — which the config grammar only permits for
        // `kind = "internal"` or `"ignore"`. These were missing here, so a
        // directory a user had explicitly declared as ignored contributed
        // nothing to the covered set and did not shadow a glob at all.
        VersionSource::PathsOnly(ps) => {
            for p in ps {
                paths.push(p.escaped().to_string());
            }
        }
        // Version comes from an external tool; the variant carries no paths.
        // Listed explicitly so a new variant is a compile error here rather
        // than a silently uncovered path.
        VersionSource::External(_) => {}
    }
    for s in &unit.satellites {
        paths.push(s.escaped().to_string());
    }
    paths
}

pub(super) fn detect_name_collisions(units: &[ResolvedReleaseUnit]) -> Result<(), ResolverError> {
    let mut by_name: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for r in units {
        let name = r.unit.name.clone();
        let path_label = match &r.origin {
            ResolveOrigin::Explicit { config_index } => {
                format!("[release_unit] #{config_index}")
            }
            ResolveOrigin::Glob { matched_path, .. } => matched_path.escaped().to_string(),
            ResolveOrigin::PartialOverride { config_index } => {
                format!("[release_unit] #{config_index} (partial)")
            }
            ResolveOrigin::Detected { detector } => format!("detector {detector}"),
        };
        by_name.entry(name).or_default().push(path_label);
    }
    for (name, paths) in by_name {
        if paths.len() > 1 {
            return Err(ResolverError::NameCollision { name, paths });
        }
    }
    Ok(())
}

pub(super) fn detect_nested_bundles(units: &[ResolvedReleaseUnit]) -> Result<(), ResolverError> {
    // A bundle's "anchor" path is the dirname of its first manifest
    // (for Manifests source) or its first satellite (External).
    fn anchor(u: &ReleaseUnit) -> Option<String> {
        if let VersionSource::Manifests(ms) = &u.source {
            if let Some(first) = ms.first() {
                let s = first.path.escaped().to_string();
                let parent = std::path::Path::new(&s)
                    .parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                return Some(if parent.is_empty() {
                    "".to_string()
                } else {
                    parent
                });
            }
        }
        u.satellites.first().map(|p| p.escaped().to_string())
    }

    // Edge case 10: anchor == "" means root → reject.
    for r in units {
        if let Some(a) = anchor(&r.unit) {
            if a.is_empty() {
                return Err(ResolverError::BundlePathIsRepoRoot {
                    unit: r.unit.name.clone(),
                });
            }
        }
    }

    // Edge case 9: one anchor is a strict prefix of another's.
    let anchored: Vec<(String, String)> = units
        .iter()
        .filter_map(|r| anchor(&r.unit).map(|a| (r.unit.name.clone(), a)))
        .collect();

    for i in 0..anchored.len() {
        for j in 0..anchored.len() {
            if i == j {
                continue;
            }
            let (outer, outer_path) = &anchored[i];
            let (inner, inner_path) = &anchored[j];
            // strict prefix: inner_path starts with `outer_path/`
            let outer_prefixed = format!("{outer_path}/");
            if inner_path.starts_with(&outer_prefixed) {
                return Err(ResolverError::NestedBundlePath {
                    outer: outer.clone(),
                    outer_path: outer_path.clone(),
                    inner: inner.clone(),
                    inner_path: inner_path.clone(),
                });
            }
        }
    }
    Ok(())
}

pub(super) fn validate_cascade_sources(units: &[ResolvedReleaseUnit]) -> Result<(), ResolverError> {
    let names: BTreeSet<String> = units.iter().map(|u| u.unit.name.clone()).collect();
    for r in units {
        if let Some(c) = &r.unit.cascade_from {
            if !names.contains(&c.source) {
                return Err(ResolverError::CascadeSourceUnknown {
                    unit: r.unit.name.clone(),
                    cascade_source: c.source.clone(),
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::git::repository::RepoPathBuf;
    use crate::core::release_unit::{
        CascadeBumpStrategy, CascadeRule, ManifestFile, VersionFieldSpec, Visibility,
    };
    use crate::core::wire::known::Ecosystem;

    #[test]
    fn detect_nested_bundles_flat_set_passes() {
        // Build two non-nested anchored units.
        let make = |name: &str, manifest_path: &str| ResolvedReleaseUnit {
            unit: ReleaseUnit {
                name: name.to_string(),
                ecosystem: Ecosystem::classify("cargo"),
                source: VersionSource::Manifests(vec![ManifestFile {
                    path: RepoPathBuf::new(manifest_path.as_bytes()),
                    ecosystem: Ecosystem::classify("cargo"),
                    version_field: VersionFieldSpec::CargoToml,
                }]),
                satellites: vec![],
                tag_format: None,
                visibility: Visibility::Public,
                cascade_from: None,
                kind: Default::default(),
                bump_override: None,
                baseline: None,
            },
            origin: ResolveOrigin::Explicit { config_index: 0 },
        };

        let units = vec![
            make("aura", "apps/services/aura/crates/bin/Cargo.toml"),
            make("ekko", "apps/services/ekko/crates/bin/Cargo.toml"),
        ];
        detect_nested_bundles(&units).unwrap();
    }

    #[test]
    fn detect_nested_bundles_strict_prefix_rejects() {
        let make = |name: &str, manifest_path: &str| ResolvedReleaseUnit {
            unit: ReleaseUnit {
                name: name.to_string(),
                ecosystem: Ecosystem::classify("cargo"),
                source: VersionSource::Manifests(vec![ManifestFile {
                    path: RepoPathBuf::new(manifest_path.as_bytes()),
                    ecosystem: Ecosystem::classify("cargo"),
                    version_field: VersionFieldSpec::CargoToml,
                }]),
                satellites: vec![],
                tag_format: None,
                visibility: Visibility::Public,
                cascade_from: None,
                kind: Default::default(),
                bump_override: None,
                baseline: None,
            },
            origin: ResolveOrigin::Explicit { config_index: 0 },
        };

        let units = vec![
            make("outer", "apps/services/Cargo.toml"),
            make("inner", "apps/services/aura/Cargo.toml"),
        ];
        let err = detect_nested_bundles(&units).unwrap_err();
        assert_eq!(err.rule(), "nested_bundle_path");
    }

    #[test]
    fn detect_name_collisions_two_explicit_same_name() {
        let make = |name: &str| ResolvedReleaseUnit {
            unit: ReleaseUnit {
                name: name.to_string(),
                ecosystem: Ecosystem::classify("cargo"),
                source: VersionSource::Manifests(vec![]),
                satellites: vec![],
                tag_format: None,
                visibility: Visibility::Public,
                cascade_from: None,
                kind: Default::default(),
                bump_override: None,
                baseline: None,
            },
            origin: ResolveOrigin::Explicit { config_index: 0 },
        };
        let units = vec![make("aura"), make("aura")];
        let err = detect_name_collisions(&units).unwrap_err();
        assert_eq!(err.rule(), "name_collision");
    }

    #[test]
    fn validate_cascade_sources_unknown_source() {
        let unit = ResolvedReleaseUnit {
            unit: ReleaseUnit {
                name: "sdk-kotlin".into(),
                ecosystem: Ecosystem::classify("jvm-library"),
                source: VersionSource::Manifests(vec![]),
                satellites: vec![],
                tag_format: None,
                visibility: Visibility::Public,
                cascade_from: Some(CascadeRule {
                    source: "ghost-schema".into(),
                    bump: CascadeBumpStrategy::FloorMinor,
                }),
                kind: Default::default(),
                bump_override: None,
                baseline: None,
            },
            origin: ResolveOrigin::Explicit { config_index: 0 },
        };
        let err = validate_cascade_sources(&[unit]).unwrap_err();
        assert_eq!(err.rule(), "cascade_source_unknown");
    }
}
