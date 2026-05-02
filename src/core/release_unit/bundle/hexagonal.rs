//! Hexagonal Cargo bundle — `apps/services/<svc>/crates/{bin,lib,workers,…}`.
//!
//! A directory matches when its `crates/` subdir holds at least 2
//! Cargo packages and one of `bin` / `lib` / `workers` (or the
//! service basename) carries the primary `[package]` block. Multiple
//! sibling services under the same parent are emitted as a single
//! `[[release_unit_glob]]` so adding a new service to
//! `apps/services/*` doesn't require a config edit.

use std::collections::HashMap;
use std::path::Path;

use super::super::shape::{BundleKind, DetectedShape, DetectorMatch, HexagonalPrimary};
use super::super::walk::{
    cargo_toml_has_package_section, find_dirs_with_subdir_pattern, list_subdirs_with_file,
    relative_repopath,
};

use crate::cmd::init::auto_detect::DetectionCounters;
use crate::cmd::init::toml_util::toml_quote;

pub fn detect(workdir: &Path) -> Vec<DetectorMatch> {
    let mut out = Vec::new();
    let crates_dirs = find_dirs_with_subdir_pattern(workdir, "crates");
    for crates_dir in crates_dirs {
        let service_dir = match crates_dir.parent() {
            Some(p) => p,
            None => continue,
        };
        let cargo_subs = list_subdirs_with_file(&crates_dir, "Cargo.toml");
        if cargo_subs.len() < 2 {
            continue;
        }
        let basename = service_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        let primaries = [
            ("bin", HexagonalPrimary::Bin),
            ("lib", HexagonalPrimary::Lib),
            ("workers", HexagonalPrimary::Workers),
            (basename, HexagonalPrimary::BaseName),
        ];
        let mut found_primary: Option<HexagonalPrimary> = None;
        for (sub, kind) in primaries {
            let sub_cargo = crates_dir.join(sub).join("Cargo.toml");
            if sub_cargo.exists() && cargo_toml_has_package_section(&sub_cargo) {
                found_primary = Some(kind);
                break;
            }
        }
        let Some(primary) = found_primary else {
            continue;
        };

        let repopath = match relative_repopath(workdir, service_dir) {
            Some(r) => r,
            None => continue,
        };
        out.push(DetectorMatch {
            shape: DetectedShape::Bundle(BundleKind::HexagonalCargo { primary }),
            path: repopath,
            note: Some(format!(
                "primary crate: {}",
                primary_label(primary, basename)
            )),
        });
    }
    out
}

/// Emit blocks for every hexagonal-cargo match at once. Cross-match
/// aggregation: siblings under the same parent collapse into one
/// `[[release_unit_glob]]`. We use a sorted Vec instead of HashMap
/// iteration to keep the snippet output byte-deterministic across
/// runs.
pub fn emit_all(
    matches: &[&DetectorMatch],
    snippet: &mut String,
    counters: &mut DetectionCounters,
) {
    let mut hex_by_parent: HashMap<String, Vec<&DetectorMatch>> = HashMap::new();
    for m in matches {
        if let DetectedShape::Bundle(BundleKind::HexagonalCargo { .. }) = m.shape {
            counters.hexagonal_cargo += 1;
            let parent = parent_of(&m.path).unwrap_or_default();
            hex_by_parent.entry(parent).or_default().push(m);
        }
    }
    let mut hex_groups: Vec<(String, Vec<&DetectorMatch>)> = hex_by_parent.into_iter().collect();
    hex_groups.sort_by(|a, b| a.0.cmp(&b.0));
    for (parent, matches) in hex_groups {
        if matches.len() >= 2 && !parent.is_empty() {
            // Majority-vote primary so the emitted `manifests = […]`
            // matches the most members. Tie-break: Bin > Lib > Workers
            // > BaseName. The `fallback_manifests` line catches outliers.
            let mut votes: HashMap<HexagonalPrimary, usize> = HashMap::new();
            for m in &matches {
                if let DetectedShape::Bundle(BundleKind::HexagonalCargo { primary }) = m.shape {
                    *votes.entry(primary).or_insert(0) += 1;
                }
            }
            let primary = pick_majority_primary(&votes);
            let primary_str = match primary {
                HexagonalPrimary::Bin => "bin",
                HexagonalPrimary::Lib => "lib",
                HexagonalPrimary::Workers => "workers",
                HexagonalPrimary::BaseName => "bin",
            };
            let glob_value = toml_quote(&format!("{parent}/*"));
            let manifests_value = toml_quote(&format!("{{path}}/crates/{primary_str}/Cargo.toml"));
            let fallback_value = toml_quote("{path}/crates/workers/Cargo.toml");
            let satellites_value = toml_quote("{path}/crates");
            let name_value = toml_quote("{basename}");
            snippet.push_str(&format!(
                "\n# Auto-detected {n} hexagonal cargo services under {parent}/*\n[[release_unit_glob]]\nglob = {glob_value}\necosystem = \"cargo\"\nmanifests = [{manifests_value}]\nfallback_manifests = [{fallback_value}]\nsatellites = [{satellites_value}]\nname = {name_value}\n",
                n = matches.len(),
            ));
        } else {
            for m in matches {
                let DetectedShape::Bundle(BundleKind::HexagonalCargo { primary }) = m.shape else {
                    continue;
                };
                let path = m.path.escaped();
                // Owned `String` so the BaseName branch doesn't leak
                // a heap allocation (the previous code did
                // `.to_string().leak()` to coerce to `&'static str`,
                // which permanently leaked memory on every call).
                let primary_str: String = match primary {
                    HexagonalPrimary::Bin => "bin".to_string(),
                    HexagonalPrimary::Lib => "lib".to_string(),
                    HexagonalPrimary::Workers => "workers".to_string(),
                    HexagonalPrimary::BaseName => {
                        path.rsplit('/').next().unwrap_or("bin").to_string()
                    }
                };
                let basename = path.rsplit('/').next().unwrap_or("unit");
                let name_q = toml_quote(basename);
                let satellites_q = toml_quote(&format!("{path}/crates"));
                let manifest_q = toml_quote(&format!("{path}/crates/{primary_str}/Cargo.toml"));
                snippet.push_str(&format!(
                    "\n[[release_unit]]\nname = {name_q}\necosystem = \"cargo\"\nsatellites = [{satellites_q}]\n[[release_unit.source.manifests]]\npath = {manifest_q}\nversion_field = \"cargo_toml\"\n",
                ));
            }
        }
    }
}

fn primary_label(p: HexagonalPrimary, basename: &str) -> &str {
    match p {
        HexagonalPrimary::Bin => "bin",
        HexagonalPrimary::Lib => "lib",
        HexagonalPrimary::Workers => "workers",
        HexagonalPrimary::BaseName => basename,
    }
}

fn pick_majority_primary(votes: &HashMap<HexagonalPrimary, usize>) -> HexagonalPrimary {
    let priority = |p: HexagonalPrimary| match p {
        HexagonalPrimary::Bin => 4,
        HexagonalPrimary::Lib => 3,
        HexagonalPrimary::Workers => 2,
        HexagonalPrimary::BaseName => 1,
    };
    votes
        .iter()
        .max_by(|a, b| {
            a.1.cmp(b.1)
                .then_with(|| priority(*a.0).cmp(&priority(*b.0)))
        })
        .map(|(k, _)| *k)
        .unwrap_or(HexagonalPrimary::Bin)
}

fn parent_of(path: &crate::core::git::repository::RepoPathBuf) -> Option<String> {
    let s = path.escaped();
    let p = std::path::Path::new(&*s);
    p.parent()
        .and_then(|p| p.to_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(p: &Path, content: &str) {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, content).unwrap();
    }

    #[test]
    fn detects_bin_primary() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("apps/services/aura/crates/bin/Cargo.toml"),
            "[package]\nname = \"aura-bin\"\nversion = \"0.1.0\"\n",
        );
        write(
            &root.join("apps/services/aura/crates/api/Cargo.toml"),
            "[package]\nname = \"aura-api\"\nversion = \"0.1.0\"\n",
        );
        let matches = detect(root);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].path.escaped(), "apps/services/aura");
        match &matches[0].shape {
            DetectedShape::Bundle(BundleKind::HexagonalCargo { primary }) => {
                assert_eq!(*primary, HexagonalPrimary::Bin);
            }
            _ => panic!("expected HexagonalCargo bundle"),
        }
    }

    #[test]
    fn detects_workers_fallback() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("apps/services/mondo/crates/workers/Cargo.toml"),
            "[package]\nname = \"mondo-workers\"\nversion = \"0.1.0\"\n",
        );
        write(
            &root.join("apps/services/mondo/crates/core/Cargo.toml"),
            "[package]\nname = \"mondo-core\"\nversion = \"0.1.0\"\n",
        );
        let matches = detect(root);
        assert_eq!(matches.len(), 1);
        match &matches[0].shape {
            DetectedShape::Bundle(BundleKind::HexagonalCargo { primary }) => {
                assert_eq!(*primary, HexagonalPrimary::Workers);
            }
            _ => panic!("expected HexagonalCargo bundle"),
        }
    }

    #[test]
    fn skips_when_only_one_crate_subdir() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("apps/foo/crates/bin/Cargo.toml"),
            "[package]\nname = \"foo\"\nversion = \"0.1.0\"\n",
        );
        let matches = detect(root);
        assert!(matches.is_empty());
    }
}
