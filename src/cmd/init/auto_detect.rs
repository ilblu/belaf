//! `belaf init --auto-detect` — runs the detectors and emits
//! release_unit / allow_uncovered TOML blocks ready to be appended
//! to `belaf/config.toml`.
//!
//! Three classes of dispatch correspond directly to the
//! [`DetectedShape`] taxonomy in [`crate::core::release_unit::shape`]:
//!
//! - `Bundle(_)`  → [`emit_bundle_block`]: writes a `[[release_unit]]`
//!   or, for hexagonal-cargo siblings, a single
//!   `[[release_unit_glob]]`.
//! - `Hint(_)`    → [`emit_hint_comment`]: drops a comment-only hint
//!   into the snippet (no toggleable config — hints decorate
//!   Standalone rows in the wizard).
//! - `ExternallyManaged(_)` → [`register_externally_managed`]: collects
//!   the path for the trailing `[allow_uncovered]` block so the drift
//!   detector stays silent on it.
//!
//! That structural separation eliminates the 3.0.x bug class where
//! `SdkCascadeMember` (a hint) was accidentally reachable from a
//! Bundle-emit path.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::toml_util::toml_quote;
use crate::core::git::repository::{RepoPathBuf, Repository};
use crate::core::release_unit::detector::{
    self, BundleKind, DetectedShape, DetectorMatch, ExtKind, HexagonalPrimary, HintKind,
    JvmVersionSource,
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
    pub single_project: usize,
    pub nested_monorepo: usize,
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

    pub fn total_advisory_hints(&self) -> usize {
        self.nested_monorepo
    }
}

/// Marker comment prepended to every emitted snippet. The
/// idempotency check in [`append_to_config`] looks for this exact
/// string instead of matching the full snippet content (which is
/// fragile: any user edit to the appended block would defeat the
/// "already there" check and cause duplicate appends on the next
/// run).
const AUTO_DETECT_MARKER: &str =
    "# belaf:auto-detect-marker (do not remove — used for idempotency)";

/// Old single-shot entry point — equivalent to `run_filtered(repo, &empty)`.
/// Kept as a stable public surface for `--ci --auto-detect` and existing
/// integration tests; the wizard's interactive path uses `run_filtered`
/// so the user's per-item exclusions can flow through.
pub fn run(repo: &Repository) -> AutoDetectResult {
    run_filtered(repo, &HashSet::new())
}

/// Auto-detect with per-match exclusions. Each excluded match path:
///   - gets **no** `[[release_unit]]` block emitted
///   - lands in the `[ignore_paths]` block of the snippet so the
///     resolver skips it AND the drift detector stays silent on it
///
/// Glob behaviour: a glob group with at least 2 non-excluded members
/// still becomes one `[[release_unit_glob]]` block; a group reduced
/// to a single non-excluded member by exclusions falls through to
/// the singleton-explicit-block path automatically.
pub fn run_filtered(repo: &Repository, exclusions: &HashSet<RepoPathBuf>) -> AutoDetectResult {
    let mut report = detector::detect_all(repo);
    if !exclusions.is_empty() {
        report.matches.retain(|m| !exclusions.contains(&m.path));
    }
    let mut snippet = String::new();
    let mut counters = DetectionCounters::default();
    let mut allow_uncovered: Vec<String> = Vec::new();
    let mut ignore_paths: Vec<String> = exclusions
        .iter()
        .map(|p| format!("{}/", p.escaped()))
        .collect();
    ignore_paths.sort();

    // Bundles: hexagonal-cargo glob-collapses; the rest go through
    // the per-shape emit path below.
    emit_hexagonal_cargo_block(&mut snippet, &mut counters, &report.matches);

    for m in &report.matches {
        match &m.shape {
            DetectedShape::Bundle(b) => match b {
                BundleKind::HexagonalCargo { .. } => {
                    // already handled above (glob-aware).
                }
                BundleKind::Tauri { single_source } => {
                    emit_tauri_block(&mut snippet, &mut counters, m, *single_source);
                }
                BundleKind::JvmLibrary { version_source } => {
                    emit_jvm_library_block(&mut snippet, &mut counters, m, version_source);
                }
            },
            DetectedShape::Hint(h) => emit_hint_comment(&mut snippet, &mut counters, m, h),
            DetectedShape::ExternallyManaged(e) => {
                register_externally_managed(&mut allow_uncovered, &mut counters, m, *e);
            }
        }
    }

    if !allow_uncovered.is_empty() {
        snippet.push_str("\n# Mobile apps detected — handed off to Bitrise / fastlane / Codemagic.\n# Belaf doesn't manage mobile app releases; these paths are listed in\n# allow_uncovered so the drift detector doesn't fire on them.\n[allow_uncovered]\n");
        let quoted: Vec<String> = allow_uncovered.iter().map(|p| toml_quote(p)).collect();
        snippet.push_str(&format!("paths = [{}]\n", quoted.join(", ")));
    }

    if counters.sdk_cascade_member > 0 {
        snippet.push_str(&format!(
            "\n# {} SDK packages detected under sdks/* — consider adding\n# `cascade_from = {{ source = \"<schema-unit>\", bump = \"floor_minor\" }}`\n# to each so they bump in lockstep when the schema bumps.\n",
            counters.sdk_cascade_member
        ));
    }

    if !ignore_paths.is_empty() {
        snippet.push_str(
            "\n# User-deselected detector hits — kept out of belaf's release\n# pipeline AND silenced for the drift detector. Move to\n# [allow_uncovered] manually if these are released externally.\n[ignore_paths]\n",
        );
        let quoted: Vec<String> = ignore_paths.iter().map(|p| toml_quote(p)).collect();
        snippet.push_str(&format!("paths = [{}]\n", quoted.join(", ")));
    }

    let prefixed_snippet = if snippet.is_empty() {
        snippet
    } else {
        format!("\n{AUTO_DETECT_MARKER}\n{snippet}")
    };

    AutoDetectResult {
        toml_snippet: prefixed_snippet,
        counters,
    }
}

/// Hexagonal-cargo Bundle emitter: groups siblings into a glob block
/// when 2+ share a parent; otherwise emits one explicit
/// `[[release_unit]]` per service. We use a sorted Vec instead of
/// HashMap iteration so the snippet output is byte-deterministic
/// across runs.
fn emit_hexagonal_cargo_block(
    snippet: &mut String,
    counters: &mut DetectionCounters,
    matches: &[DetectorMatch],
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
                HexagonalPrimary::BaseName => "bin", // safe default; user can edit
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

fn emit_tauri_block(
    snippet: &mut String,
    counters: &mut DetectionCounters,
    m: &DetectorMatch,
    single_source: bool,
) {
    let path = m.path.escaped();
    let name_raw = path.rsplit('/').next().unwrap_or("desktop");
    let name_q = toml_quote(name_raw);
    let satellites_q = toml_quote(&path);
    if single_source {
        counters.tauri_single_source += 1;
        let manifest_q = toml_quote(&format!("{path}/package.json"));
        snippet.push_str(&format!(
            "\n[[release_unit]]\nname = {name_q}\necosystem = \"tauri\"\nsatellites = [{satellites_q}]\n[[release_unit.source.manifests]]\npath = {manifest_q}\nversion_field = \"npm_package_json\"\n",
        ));
    } else {
        counters.tauri_legacy += 1;
        let pkg_q = toml_quote(&format!("{path}/package.json"));
        let cargo_q = toml_quote(&format!("{path}/src-tauri/Cargo.toml"));
        let conf_q = toml_quote(&format!("{path}/src-tauri/tauri.conf.json"));
        snippet.push_str(&format!(
            "\n# Tauri legacy multi-file (3 manifests in lockstep)\n[[release_unit]]\nname = {name_q}\necosystem = \"tauri\"\nsatellites = [{satellites_q}]\n[[release_unit.source.manifests]]\npath = {pkg_q}\nversion_field = \"npm_package_json\"\n[[release_unit.source.manifests]]\npath = {cargo_q}\nversion_field = \"cargo_toml\"\n[[release_unit.source.manifests]]\npath = {conf_q}\nversion_field = \"tauri_conf_json\"\n",
        ));
    }
}

fn emit_jvm_library_block(
    snippet: &mut String,
    counters: &mut DetectionCounters,
    m: &DetectorMatch,
    version_source: &JvmVersionSource,
) {
    counters.jvm_library += 1;
    let path = m.path.escaped();
    let name_raw = path.rsplit('/').next().unwrap_or("sdk");
    let name_q = toml_quote(name_raw);
    let satellites_q = toml_quote(&path);
    let (vfield, manifest_raw) = match version_source {
        JvmVersionSource::GradleProperties => {
            ("gradle_properties", format!("{path}/gradle.properties"))
        }
        JvmVersionSource::BuildGradleKtsLiteral => {
            ("generic_regex", format!("{path}/build.gradle.kts"))
        }
        JvmVersionSource::PluginManaged => {
            snippet.push_str(&format!(
                "\n# Plugin-managed JVM library at {path} — recommend external_versioner.\n# Edit the [release_unit.source.external] block below to drive your gradle plugin.\n[[release_unit]]\nname = {name_q}\necosystem = \"external\"\nsatellites = [{satellites_q}]\n[release_unit.source.external]\ntool = \"gradle\"\nread_command = \"./gradlew :printVersion -q\"\nwrite_command = \"./gradlew :setVersion -PnewVersion={{version}}\"\ntimeout_sec = 120\n",
            ));
            return;
        }
    };
    let manifest_q = toml_quote(&manifest_raw);
    snippet.push_str(&format!(
        "\n[[release_unit]]\nname = {name_q}\necosystem = \"jvm-library\"\nsatellites = [{satellites_q}]\n[[release_unit.source.manifests]]\npath = {manifest_q}\nversion_field = \"{vfield}\"\n",
    ));
    if vfield == "generic_regex" {
        snippet.push_str("regex_pattern = '(?m)^version\\s*=\\s*\"([^\"]+)\"'\n");
        snippet.push_str("regex_replace = \"version = \\\"{version}\\\"\"\n");
    }
}

/// Pure metadata; never togglable, never produces a `[[release_unit]]`.
/// Hint comments help the user understand what the wizard saw without
/// committing config they'd have to maintain.
fn emit_hint_comment(
    snippet: &mut String,
    counters: &mut DetectionCounters,
    m: &DetectorMatch,
    hint: &HintKind,
) {
    match hint {
        HintKind::SdkCascade => {
            counters.sdk_cascade_member += 1;
            // Aggregated message after the per-shape loop.
        }
        HintKind::NpmWorkspace => {
            counters.nested_npm_workspace += 1;
            let path = m.path.escaped();
            snippet.push_str(&format!(
                "\n# Nested npm workspace detected at {path} — its members will\n# be auto-detected by the npm loader. Add an explicit [[release_unit]]\n# here if you want a non-default tag-format / cascade / visibility.\n",
            ));
        }
        HintKind::SingleProject { ecosystem } => {
            counters.single_project += 1;
            snippet.push_str(&format!(
                "\n# Single-project repo detected ({ecosystem}) — `v{{version}}`\n# tag format is suggested instead of the ecosystem default.\n# Override per-unit if you publish under a different naming convention.\n",
            ));
        }
        HintKind::NestedMonorepo => {
            counters.nested_monorepo += 1;
            let path = m.path.escaped();
            let note = m
                .note
                .as_deref()
                .unwrap_or("submodule looks like its own monorepo");
            snippet.push_str(&format!(
                "\n# Nested submodule at {path} — {note}.\n# Consider running `belaf init` inside the submodule and excluding\n# its path from this repo's detection rather than driving both from one config.\n",
            ));
        }
    }
}

fn register_externally_managed(
    allow_uncovered: &mut Vec<String>,
    counters: &mut DetectionCounters,
    m: &DetectorMatch,
    ext: ExtKind,
) {
    match ext {
        ExtKind::MobileIos => counters.mobile_ios += 1,
        ExtKind::MobileAndroid => counters.mobile_android += 1,
    }
    allow_uncovered.push(format!("{}/", m.path.escaped()));
}

/// Append the snippet to `belaf/config.toml`. Idempotent via the
/// [`AUTO_DETECT_MARKER`] comment line: the snippet is appended only
/// if the marker isn't already present in the config. This survives
/// the user editing the appended block (e.g. tweaking a tag_format
/// or commenting a manifest out) — only removing the marker line
/// itself causes a re-append on the next `--auto-detect` run.
///
/// Tag-format-override snippets (which don't carry the marker) skip
/// the idempotency check and always append; they're emitted at most
/// once per wizard run anyway.
pub fn append_to_config(config_path: &Path, snippet: &str) -> std::io::Result<()> {
    if snippet.is_empty() {
        return Ok(());
    }
    let existing = std::fs::read_to_string(config_path).unwrap_or_default();
    if snippet.contains(AUTO_DETECT_MARKER) && existing.contains(AUTO_DETECT_MARKER) {
        return Ok(());
    }
    let mut content = existing;
    if !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(snippet);
    std::fs::write(config_path, content)
}

/// Vote-based primary picker for the hexagonal-cargo glob block.
/// Tie-break order: `Bin` (the convention) > `Lib` > `Workers` >
/// `BaseName`.
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
        let dir = tempfile::TempDir::new().unwrap();
        let _ = std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output();
        let repo = Repository::open(dir.path()).expect("open");
        let r = run(&repo);
        assert!(
            r.toml_snippet.is_empty(),
            "empty repo must produce no snippet, got: {}",
            r.toml_snippet
        );
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

    // -------------------------------------------------------------------
    // C3 — TOML injection regression tests for `toml_quote`. A malicious
    // path / unit name must not be able to break out of its TOML slot.
    // -------------------------------------------------------------------

    #[test]
    fn toml_quote_escapes_double_quotes() {
        assert_eq!(toml_quote(r#"foo"bar"#), r#""foo\"bar""#);
    }

    #[test]
    fn toml_quote_escapes_backslashes() {
        assert_eq!(toml_quote(r"foo\bar"), r#""foo\\bar""#);
    }

    #[test]
    fn toml_quote_escapes_newlines_and_tabs() {
        assert_eq!(toml_quote("a\nb\tc"), r#""a\nb\tc""#);
    }

    #[test]
    fn toml_quote_escapes_control_characters() {
        // \x07 (BEL) is below 0x20 — must surface as .
        assert_eq!(toml_quote("\x07"), "\"\\u0007\"");
    }

    #[test]
    fn toml_quote_round_trips_via_toml_parser() {
        let nasty = r#"a"b\c]] = inject"#;
        let s = format!("key = {}", toml_quote(nasty));
        let parsed: toml::Value = toml::from_str(&s).expect("must parse as valid TOML");
        assert_eq!(parsed["key"].as_str(), Some(nasty));
    }

    // -------------------------------------------------------------------
    // M7 — idempotency must rest on a stable marker, not on
    // string-equal-snippet matching.
    // -------------------------------------------------------------------

    #[test]
    fn append_to_config_skips_when_marker_already_present() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = dir.path().join("config.toml");
        std::fs::write(&cfg, format!("# existing line\n{AUTO_DETECT_MARKER}\n")).unwrap();
        let snippet = format!("{AUTO_DETECT_MARKER}\n[[release_unit]]\nname = \"alpha\"\n");
        append_to_config(&cfg, &snippet).unwrap();
        let after = std::fs::read_to_string(&cfg).unwrap();
        assert!(
            !after.contains("[[release_unit]]"),
            "marker present → snippet must NOT be re-appended; got:\n{after}"
        );
    }

    #[test]
    fn append_to_config_re_appends_when_marker_removed() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = dir.path().join("config.toml");
        std::fs::write(&cfg, "# user removed the marker\n").unwrap();
        let snippet = format!("{AUTO_DETECT_MARKER}\n[[release_unit]]\nname = \"alpha\"\n");
        append_to_config(&cfg, &snippet).unwrap();
        let after = std::fs::read_to_string(&cfg).unwrap();
        assert!(
            after.contains("[[release_unit]]") && after.contains(AUTO_DETECT_MARKER),
            "missing marker → snippet must be appended; got:\n{after}"
        );
    }
}
