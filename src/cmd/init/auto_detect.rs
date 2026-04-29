//! `belaf init --auto-detect` — runs the Phase F detectors and emits
//! release_unit / allow_uncovered TOML blocks ready to be appended
//! to `belaf/config.toml`.
//!
//! Phase I.2 + I.5 of `BELAF_MASTER_PLAN.md`. Mobile-app detector
//! hits are auto-added to `[allow_uncovered]` so the drift detector
//! doesn't fire on them. Other detector hits are emitted as
//! `[[release_unit]]` blocks (or, where idiomatic, a single
//! `[[release_unit_glob]]` collapsing many sibling matches).

use std::collections::HashMap;
use std::path::Path;

use crate::core::git::repository::{RepoPathBuf, Repository};
use crate::core::release_unit::detector::{
    self, DetectorKind, DetectorMatch, HexagonalPrimary, JvmVersionSource, MobilePlatform,
};

/// Result of an auto-detect pass: TOML snippet to append, plus
/// counters per detector kind for the wizard summary.
#[derive(Debug, Default)]
pub struct AutoDetectResult {
    pub toml_snippet: String,
    pub counters: DetectionCounters,
}

#[derive(Debug, Default)]
pub struct DetectionCounters {
    pub hexagonal_cargo: usize,
    pub tauri_single_source: usize,
    pub tauri_legacy: usize,
    pub jvm_library: usize,
    pub mobile_ios: usize,
    pub mobile_android: usize,
    pub nested_npm_workspace: usize,
    pub sdk_cascade_member: usize,
}

impl DetectionCounters {
    pub fn total_release_unit_candidates(&self) -> usize {
        self.hexagonal_cargo
            + self.tauri_single_source
            + self.tauri_legacy
            + self.jvm_library
            + self.nested_npm_workspace
            + self.sdk_cascade_member
    }

    pub fn total_mobile_warnings(&self) -> usize {
        self.mobile_ios + self.mobile_android
    }
}

pub fn run(repo: &Repository) -> AutoDetectResult {
    let report = detector::detect_all(repo);
    let mut snippet = String::new();
    let mut counters = DetectionCounters::default();
    let mut allow_uncovered: Vec<String> = Vec::new();

    // Group hexagonal cargo matches into a single glob block per
    // common parent (e.g. `apps/services/*`) when at least 2
    // matches share the same parent — clikd-shape collapses 13
    // services into 1 glob block.
    let mut hex_by_parent: HashMap<String, Vec<&DetectorMatch>> = HashMap::new();
    for m in &report.matches {
        if matches!(m.kind, DetectorKind::HexagonalCargo { .. }) {
            counters.hexagonal_cargo += 1;
            let parent = parent_of(&m.path).unwrap_or_default();
            hex_by_parent.entry(parent).or_default().push(m);
        }
    }
    for (parent, matches) in hex_by_parent {
        if matches.len() >= 2 && !parent.is_empty() {
            // Glob block.
            let primary = match &matches[0].kind {
                DetectorKind::HexagonalCargo { primary } => *primary,
                _ => unreachable!(),
            };
            let primary_str = match primary {
                HexagonalPrimary::Bin => "bin",
                HexagonalPrimary::Lib => "lib",
                HexagonalPrimary::Workers => "workers",
                HexagonalPrimary::BaseName => "bin", // safe default; user can edit
            };
            snippet.push_str(&format!(
                "\n# Auto-detected {n} hexagonal cargo services under {parent}/*\n[[release_unit_glob]]\nglob = \"{parent}/*\"\necosystem = \"cargo\"\nmanifests = [\"{{path}}/crates/{primary_str}/Cargo.toml\"]\nfallback_manifests = [\"{{path}}/crates/workers/Cargo.toml\"]\nsatellites = [\"{{path}}/crates\"]\nname = \"{{basename}}\"\n",
                n = matches.len(),
            ));
        } else {
            // Singletons → explicit blocks.
            for m in matches {
                let primary = match &m.kind {
                    DetectorKind::HexagonalCargo { primary } => *primary,
                    _ => continue,
                };
                let primary_str = match primary {
                    HexagonalPrimary::Bin => "bin",
                    HexagonalPrimary::Lib => "lib",
                    HexagonalPrimary::Workers => "workers",
                    HexagonalPrimary::BaseName => m
                        .path
                        .escaped()
                        .rsplit('/')
                        .next()
                        .unwrap_or("bin")
                        .to_string()
                        .leak(),
                };
                let path = m.path.escaped();
                let basename = path.rsplit('/').next().unwrap_or("unit");
                snippet.push_str(&format!(
                    "\n[[release_unit]]\nname = \"{basename}\"\necosystem = \"cargo\"\nsatellites = [\"{path}/crates\"]\n[[release_unit.source.manifests]]\npath = \"{path}/crates/{primary_str}/Cargo.toml\"\nversion_field = \"cargo_toml\"\n",
                ));
            }
        }
    }

    // Tauri
    for m in &report.matches {
        if let DetectorKind::Tauri { single_source } = &m.kind {
            if *single_source {
                counters.tauri_single_source += 1;
                let path = m.path.escaped();
                snippet.push_str(&format!(
                    "\n[[release_unit]]\nname = \"{name}\"\necosystem = \"tauri\"\nsatellites = [\"{path}\"]\n[[release_unit.source.manifests]]\npath = \"{path}/package.json\"\nversion_field = \"npm_package_json\"\n",
                    name = path.rsplit('/').next().unwrap_or("desktop"),
                ));
            } else {
                counters.tauri_legacy += 1;
                let path = m.path.escaped();
                snippet.push_str(&format!(
                    "\n# Tauri legacy multi-file (3 manifests in lockstep)\n[[release_unit]]\nname = \"{name}\"\necosystem = \"tauri\"\nsatellites = [\"{path}\"]\n[[release_unit.source.manifests]]\npath = \"{path}/package.json\"\nversion_field = \"npm_package_json\"\n[[release_unit.source.manifests]]\npath = \"{path}/src-tauri/Cargo.toml\"\nversion_field = \"cargo_toml\"\n[[release_unit.source.manifests]]\npath = \"{path}/src-tauri/tauri.conf.json\"\nversion_field = \"tauri_conf_json\"\n",
                    name = path.rsplit('/').next().unwrap_or("desktop"),
                ));
            }
        }
    }

    // JVM library
    for m in &report.matches {
        if let DetectorKind::JvmLibrary { version_source } = &m.kind {
            counters.jvm_library += 1;
            let path = m.path.escaped();
            let name = path.rsplit('/').next().unwrap_or("sdk");
            let (vfield, manifest) = match version_source {
                JvmVersionSource::GradleProperties => {
                    ("gradle_properties", format!("{path}/gradle.properties"))
                }
                JvmVersionSource::BuildGradleKtsLiteral => {
                    ("generic_regex", format!("{path}/build.gradle.kts"))
                }
                JvmVersionSource::PluginManaged => {
                    snippet.push_str(&format!(
                        "\n# Plugin-managed JVM library at {path} — recommend external_versioner.\n# Edit the [release_unit.source.external] block below to drive your gradle plugin.\n[[release_unit]]\nname = \"{name}\"\necosystem = \"external\"\nsatellites = [\"{path}\"]\n[release_unit.source.external]\ntool = \"gradle\"\nread_command = \"./gradlew :printVersion -q\"\nwrite_command = \"./gradlew :setVersion -PnewVersion={{version}}\"\ntimeout_sec = 120\n",
                    ));
                    continue;
                }
            };
            snippet.push_str(&format!(
                "\n[[release_unit]]\nname = \"{name}\"\necosystem = \"jvm-library\"\nsatellites = [\"{path}\"]\n[[release_unit.source.manifests]]\npath = \"{manifest}\"\nversion_field = \"{vfield}\"\n",
            ));
            // GenericRegex needs pattern + replace
            if vfield == "generic_regex" {
                snippet.push_str("regex_pattern = '(?m)^version\\s*=\\s*\"([^\"]+)\"'\n");
                snippet.push_str("regex_replace = \"version = \\\"{version}\\\"\"\n");
            }
        }
    }

    // Mobile apps → allow_uncovered (Phase I.5).
    for m in &report.matches {
        if let DetectorKind::MobileApp { platform } = &m.kind {
            match platform {
                MobilePlatform::Ios => counters.mobile_ios += 1,
                MobilePlatform::Android => counters.mobile_android += 1,
            }
            allow_uncovered.push(format!("{}/", m.path.escaped()));
        }
    }
    if !allow_uncovered.is_empty() {
        snippet.push_str("\n# Mobile apps detected — handed off to Bitrise / fastlane / Codemagic.\n# Belaf doesn't manage mobile app releases; these paths are listed in\n# allow_uncovered so the drift detector doesn't fire on them.\n[allow_uncovered]\n");
        snippet.push_str(&format!("paths = {:?}\n", allow_uncovered));
    }

    // Nested npm workspace
    for m in &report.matches {
        if matches!(m.kind, DetectorKind::NestedNpmWorkspace) {
            counters.nested_npm_workspace += 1;
            // Just a comment hint — the npm loader already covers the
            // packages within. User can add explicit blocks if they
            // want different release-unit granularity.
            let path = m.path.escaped();
            snippet.push_str(&format!(
                "\n# Nested npm workspace detected at {path} — its members will\n# be auto-detected by the npm loader. Add an explicit [[release_unit]]\n# here if you want a non-default tag-format / cascade / visibility.\n",
            ));
        }
    }

    // SDK cascade members
    for m in &report.matches {
        if matches!(m.kind, DetectorKind::SdkCascadeMember) {
            counters.sdk_cascade_member += 1;
        }
    }
    if counters.sdk_cascade_member > 0 {
        snippet.push_str(&format!(
            "\n# {} SDK packages detected under sdks/* — consider adding\n# `cascade_from = {{ source = \"<schema-unit>\", bump = \"floor_minor\" }}`\n# to each so they bump in lockstep when the schema bumps.\n",
            counters.sdk_cascade_member
        ));
    }

    AutoDetectResult {
        toml_snippet: snippet,
        counters,
    }
}

/// Append the snippet to `belaf/config.toml`. Idempotent: if the
/// snippet (or part of it) is already present, only the missing
/// lines are appended. Simple "contains" check — we don't try to
/// merge TOML structures.
pub fn append_to_config(config_path: &Path, snippet: &str) -> std::io::Result<()> {
    if snippet.is_empty() {
        return Ok(());
    }
    let existing = std::fs::read_to_string(config_path).unwrap_or_default();
    if existing.contains(snippet.trim()) {
        return Ok(());
    }
    let mut content = existing;
    if !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(snippet);
    std::fs::write(config_path, content)
}

fn parent_of(path: &RepoPathBuf) -> Option<String> {
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

    #[test]
    fn empty_repo_produces_empty_snippet() {
        // Use a temp dir as repo with nothing inside.
        let dir = tempfile::TempDir::new().unwrap();
        let _ = std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output();
        let repo = Repository::open(dir.path()).expect("open");
        let r = run(&repo);
        assert!(r.toml_snippet.is_empty() || !r.toml_snippet.contains("[[release_unit"));
        assert_eq!(r.counters.total_release_unit_candidates(), 0);
    }

    #[test]
    fn counters_total_release_unit_candidates_excludes_mobile() {
        let c = DetectionCounters {
            hexagonal_cargo: 5,
            tauri_legacy: 1,
            jvm_library: 2,
            mobile_ios: 1,
            mobile_android: 1,
            ..Default::default()
        };
        assert_eq!(c.total_release_unit_candidates(), 8);
        assert_eq!(c.total_mobile_warnings(), 2);
    }
}
