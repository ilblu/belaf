//! `belaf init --auto-detect` — runs the detectors and emits
//! release_unit / allow_uncovered TOML blocks ready to be appended
//! to `belaf/config.toml`.
//!
//! Three classes of dispatch correspond directly to the
//! [`DetectedShape`] taxonomy in [`crate::core::release_unit::shape`]:
//!
//! - `Bundle(_)`  → `bundle::emit_all`: writes a `[release_unit.<name>]`
//!   block (or, for hexagonal-cargo siblings sharing a parent, one
//!   block with a `glob` field).
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
use crate::core::release_unit::bundle;
use crate::core::release_unit::detector::{self, DetectedShape, DetectorMatch, ExtKind, HintKind};
use crate::core::release_unit::resolver::{
    default_manifest_filename_for_ecosystem, default_version_field_for_ecosystem,
};

/// Identifies a loader-discovered standalone unit by name + ecosystem +
/// repo prefix. Auto-detect uses this to emit decorator blocks for units
/// the user attached a `cascade_from` override to via the wizard.
#[derive(Clone, Debug)]
pub struct StandaloneRef {
    pub name: String,
    pub ecosystem: String,
    pub prefix: String,
}

/// Wizard-confirmed `cascade_from` rule for one unit, keyed by unit
/// name. Wire form mirrors `cascade_from = { source = ..., bump = ... }`.
#[derive(Clone, Debug)]
pub struct CascadeOverrideEmit {
    pub source: String,
    pub strategy: String,
}

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
    pub jvm_plugin_managed: usize,
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

/// Old single-shot entry point — equivalent to running with no
/// exclusions and no cascade overrides. Kept as a stable public
/// surface for `--ci --auto-detect` and existing integration tests.
pub fn run(repo: &Repository) -> AutoDetectResult {
    run_with_cascade(repo, &HashSet::new(), &[], &HashMap::new())
}

/// Backwards-compatible entry that takes only path exclusions; the
/// wizard's interactive path now calls [`run_with_cascade`] directly
/// so user-confirmed cascade-from overrides flow into the emitted
/// snippet alongside the bundle blocks.
pub fn run_filtered(repo: &Repository, exclusions: &HashSet<RepoPathBuf>) -> AutoDetectResult {
    run_with_cascade(repo, exclusions, &[], &HashMap::new())
}

/// Full auto-detect entry. Each excluded match path:
///   - gets **no** `[release_unit.<name>]` block emitted
///   - lands in the `[ignore_paths]` block of the snippet so the
///     resolver skips it AND the drift detector stays silent on it
///
/// Glob behaviour: a glob group with at least 2 non-excluded members
/// still becomes one `[release_unit.<name>]` block with `glob = ...`;
/// a group reduced to a single non-excluded member by exclusions falls
/// through to the singleton-explicit-block path automatically.
///
/// `standalones` + `cascade_overrides` together drive emission of
/// decorator blocks for loader-discovered standalone units the user
/// attached a `cascade_from` rule to. Each override produces one
/// `[release_unit.<name>]` block carrying the unit's ecosystem, the
/// canonical manifest path (derived from the loader-known prefix +
/// ecosystem default filename), and the wire-form `cascade_from`
/// inline table.
pub fn run_with_cascade(
    repo: &Repository,
    exclusions: &HashSet<RepoPathBuf>,
    standalones: &[StandaloneRef],
    cascade_overrides: &HashMap<String, CascadeOverrideEmit>,
) -> AutoDetectResult {
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

    // Bundles: dispatched as a single call. Each per-bundle module
    // owns its own emission (per-match for Tauri / JVM, cross-match
    // glob-collapse for hexagonal). Adding a new bundle = one new
    // file under `bundle/` + one `mod` + one call inside
    // `bundle::emit_all` — never an edit here.
    bundle::emit_all(&report.matches, &mut snippet, &mut counters);

    // Hints + ExternallyManaged still dispatch inline because they
    // share the `allow_uncovered` accumulator and the SDK-cascade
    // aggregated message after the loop.
    for m in &report.matches {
        match &m.shape {
            DetectedShape::Bundle(_) => {
                // already emitted by bundle::emit_all above
            }
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

    if counters.sdk_cascade_member > 0 && cascade_overrides.is_empty() {
        // Only print the "consider adding cascade_from" hint when the
        // user hasn't already wired one up via the wizard's `[c]` flow.
        // If they have, the actual cascade-blocks below replace this
        // generic suggestion.
        snippet.push_str(&format!(
            "\n# {} SDK packages detected under sdks/* — consider adding\n# `cascade_from = {{ source = \"<schema-unit>\", bump = \"floor_minor\" }}`\n# to each so they bump in lockstep when the schema bumps.\n",
            counters.sdk_cascade_member
        ));
    }

    // Wizard-confirmed cascade-from overrides become decorator blocks
    // for the matching auto-discovered standalone units. Each block
    // carries ecosystem + the canonical manifest path (derived from
    // the loader-known prefix + ecosystem default filename) so the
    // resolver accepts the entry; cascade_from is the actual added
    // semantic.
    let mut override_names: Vec<&String> = cascade_overrides.keys().collect();
    override_names.sort();
    for name in override_names {
        let Some(unit_ref) = standalones.iter().find(|s| s.name == *name) else {
            // Override targets a unit no longer in the loader output —
            // skip silently. The user can re-run init.
            continue;
        };
        let Some(filename) = default_manifest_filename_for_ecosystem(&unit_ref.ecosystem) else {
            // Unknown ecosystem — emit an explanatory comment instead
            // so the user can hand-edit the block.
            snippet.push_str(&format!(
                "\n# cascade_from override for `{name}` was selected interactively but ecosystem\n# `{}` has no canonical manifest filename — add a manifests entry by hand:\n",
                unit_ref.ecosystem,
            ));
            continue;
        };
        let Some(ov) = cascade_overrides.get(name) else {
            continue;
        };
        // prefix == "root" or empty means the unit lives at the repo
        // root. Synthesise the manifest path accordingly.
        let manifest_raw = if unit_ref.prefix.is_empty() || unit_ref.prefix == "root" {
            filename.to_string()
        } else {
            format!("{}/{}", unit_ref.prefix, filename)
        };
        let key_q = toml_quote(name);
        let manifest_q = toml_quote(&manifest_raw);
        let source_q = toml_quote(&ov.source);
        let strategy_q = toml_quote(&ov.strategy);
        let version_field = default_version_field_for_ecosystem(&unit_ref.ecosystem);
        snippet.push_str(&format!(
            "\n# cascade_from override picked interactively in `belaf init` for {name}.\n[release_unit.{key_q}]\necosystem = \"{eco}\"\nmanifests = [{{ path = {manifest_q}, version_field = \"{version_field}\" }}]\ncascade_from = {{ source = {source_q}, bump = {strategy_q} }}\n",
            eco = unit_ref.ecosystem,
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

/// Pure metadata; never togglable, never produces a `[release_unit.<name>]`.
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
                "\n# Nested npm workspace detected at {path} — its members will\n# be auto-detected by the npm loader. Add an explicit [release_unit.<name>]\n# here if you want a non-default tag-format / cascade / visibility.\n",
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
        ExtKind::JvmPluginManaged => counters.jvm_plugin_managed += 1,
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
    fn cascade_override_emits_parseable_block() {
        use crate::core::release_unit::syntax::ReleaseUnitConfig;

        let dir = tempfile::TempDir::new().unwrap();
        let _ = std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output();
        let repo = Repository::open(dir.path()).expect("open");

        let standalones = vec![StandaloneRef {
            name: "@org/sdk-ts".to_string(),
            ecosystem: "npm".to_string(),
            prefix: "sdks/typescript".to_string(),
        }];
        let mut overrides = HashMap::new();
        overrides.insert(
            "@org/sdk-ts".to_string(),
            CascadeOverrideEmit {
                source: "schema".to_string(),
                strategy: "floor_minor".to_string(),
            },
        );

        let result = run_with_cascade(&repo, &HashSet::new(), &standalones, &overrides);
        assert!(
            result
                .toml_snippet
                .contains("[release_unit.\"@org/sdk-ts\"]"),
            "expected cascade decorator block, got:\n{}",
            result.toml_snippet
        );
        assert!(
            result.toml_snippet.contains("cascade_from = { source ="),
            "expected cascade_from inline table, got:\n{}",
            result.toml_snippet
        );

        // Strip the leading marker comment + extract just the
        // `[release_unit.<name>]` block, then ensure it parses through
        // the syntax loader.
        let block_start = result
            .toml_snippet
            .find("[release_unit.\"@org/sdk-ts\"]")
            .expect("block present");
        let block_text = &result.toml_snippet[block_start..];
        let parsed: std::collections::HashMap<
            String,
            std::collections::HashMap<String, ReleaseUnitConfig>,
        > = toml::from_str(&format!(
            "[release_unit]\n[release_unit.\"@org/sdk-ts\"]\n{}",
            &block_text["[release_unit.\"@org/sdk-ts\"]\n".len()..]
        ))
        .or_else(|_| {
            // Simpler form: parse the block as-is wrapped under
            // a release_unit table.
            toml::from_str::<
                std::collections::HashMap<
                    String,
                    std::collections::HashMap<String, ReleaseUnitConfig>,
                >,
            >(&format!(
                "{}\n",
                block_text
                    .lines()
                    .take_while(|l| !l.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n")
            ))
        })
        .expect("emitted cascade block must parse via syntax loader");
        let cfg = parsed
            .get("release_unit")
            .and_then(|m| m.get("@org/sdk-ts"))
            .expect("release_unit.@org/sdk-ts entry");
        assert_eq!(cfg.ecosystem.as_deref(), Some("npm"));
        let cascade = cfg.cascade_from.as_ref().expect("cascade_from set");
        assert_eq!(cascade.source, "schema");
        assert_eq!(cascade.bump, "floor_minor");
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
        let snippet =
            format!("{AUTO_DETECT_MARKER}\n[release_unit.alpha]\necosystem = \"cargo\"\n");
        append_to_config(&cfg, &snippet).unwrap();
        let after = std::fs::read_to_string(&cfg).unwrap();
        assert!(
            !after.contains("[release_unit.alpha]"),
            "marker present → snippet must NOT be re-appended; got:\n{after}"
        );
    }

    #[test]
    fn append_to_config_re_appends_when_marker_removed() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = dir.path().join("config.toml");
        std::fs::write(&cfg, "# user removed the marker\n").unwrap();
        let snippet =
            format!("{AUTO_DETECT_MARKER}\n[release_unit.alpha]\necosystem = \"cargo\"\n");
        append_to_config(&cfg, &snippet).unwrap();
        let after = std::fs::read_to_string(&cfg).unwrap();
        assert!(
            after.contains("[release_unit.alpha]") && after.contains(AUTO_DETECT_MARKER),
            "missing marker → snippet must be appended; got:\n{after}"
        );
    }
}
