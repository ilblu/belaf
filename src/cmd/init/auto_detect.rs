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

/// Marker comment prepended to every emitted snippet. The
/// idempotency check in [`append_to_config`] looks for this exact
/// string instead of matching the full snippet content (which is
/// fragile: any user edit to the appended block would defeat the
/// "already there" check and cause duplicate appends on the next
/// run).
const AUTO_DETECT_MARKER: &str =
    "# belaf:auto-detect-marker (do not remove — used for idempotency)";

/// Wrap `s` as a TOML basic-string. Properly escapes embedded `"`
/// and `\` plus control characters, so a path / name containing
/// shell-or-TOML metacharacters can't break out of its slot and
/// inject arbitrary structure into the emitted config.
fn toml_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str(r#"\""#),
            '\\' => out.push_str(r"\\"),
            '\n' => out.push_str(r"\n"),
            '\r' => out.push_str(r"\r"),
            '\t' => out.push_str(r"\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
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
            // Singletons → explicit blocks.
            for m in matches {
                let primary = match &m.kind {
                    DetectorKind::HexagonalCargo { primary } => *primary,
                    _ => continue,
                };
                let path = m.path.escaped();
                // Owned `String` so the `BaseName` branch doesn't have
                // to leak a heap allocation (previous code did
                // `.to_string().leak()` to coerce to `&'static str`,
                // which permanently leaked memory on every call —
                // unbounded under repeated `belaf init` / `prepare`).
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

    // Tauri
    for m in &report.matches {
        if let DetectorKind::Tauri { single_source } = &m.kind {
            let path = m.path.escaped();
            let name_raw = path.rsplit('/').next().unwrap_or("desktop");
            let name_q = toml_quote(name_raw);
            let satellites_q = toml_quote(&path);
            if *single_source {
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
    }

    // JVM library
    for m in &report.matches {
        if let DetectorKind::JvmLibrary { version_source } = &m.kind {
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
                    continue;
                }
            };
            let manifest_q = toml_quote(&manifest_raw);
            snippet.push_str(&format!(
                "\n[[release_unit]]\nname = {name_q}\necosystem = \"jvm-library\"\nsatellites = [{satellites_q}]\n[[release_unit.source.manifests]]\npath = {manifest_q}\nversion_field = \"{vfield}\"\n",
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
        let quoted: Vec<String> = allow_uncovered.iter().map(|p| toml_quote(p)).collect();
        snippet.push_str(&format!("paths = [{}]\n", quoted.join(", ")));
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

    // Prepend the idempotency marker so re-runs of `belaf init
    // --auto-detect` don't duplicate-append the same blocks. The
    // marker is a unique comment line that `append_to_config` greps
    // for before writing.
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
        // \x07 (BEL) is below 0x20 — must surface as .
        assert_eq!(toml_quote("\x07"), "\"\\u0007\"");
    }

    #[test]
    fn toml_quote_round_trips_via_toml_parser() {
        // The strongest end-to-end check: feed the quoted output to a
        // TOML parser and confirm we get the original bytes back, even
        // with malicious metacharacters mixed in.
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
        // The user yanks the marker line — next `--auto-detect` run
        // should re-emit the snippet because the idempotency anchor
        // is gone.
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
