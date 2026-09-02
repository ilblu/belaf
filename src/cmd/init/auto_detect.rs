//! `belaf init --auto-detect` — runs the detectors and emits
//! release_unit / allow_uncovered TOML blocks ready to be appended
//! to `belaf/config.toml`.
//!
//! Four classes of dispatch correspond directly to the
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
//! - `NeedsDecision(_)` → [`register_needs_decision`]: writes **no**
//!   config at all and raises advice instead. Auto-detect exists to
//!   turn what it understood into config; a hit it did not understand
//!   has no correct block to write, and the one block that would make
//!   the message go away — an `[allow_uncovered]` line — is the one
//!   that loses the artifact. So the path stays uncovered on purpose
//!   and the drift check keeps naming it until a human answers.
//!
//! That structural separation eliminates the 3.0.x bug class where
//! `SdkCascadeMember` (a hint) was accidentally reachable from a
//! Bundle-emit path.

use std::collections::{HashMap, HashSet};

use super::toml_util::toml_quote;
use crate::core::git::repository::{RepoPathBuf, Repository};
use crate::core::release_unit::bundle;
use crate::core::release_unit::detector::{
    self, DecisionKind, DetectedShape, DetectorMatch, ExtKind, HintKind,
};
use crate::core::release_unit::resolver::{
    default_manifest_filename_for_ecosystem, default_version_field_for_ecosystem,
};

mod apply;
mod decisions;

pub use apply::{append_validated, apply_to_config};
pub use decisions::ExistingDecisions;

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
    /// Marker-wrapped snippet (body + `[allow_uncovered]` table) for
    /// the single-shot append path ([`append_to_config`]).
    pub toml_snippet: String,
    /// Body only — `[release_unit.<name>]` blocks, hint comments, and
    /// the `[ignore_paths]` table, WITHOUT the wrapper and WITHOUT the
    /// `[allow_uncovered]` table. [`apply_to_config`] appends this and
    /// handles `allow_uncovered_paths` separately so an existing table
    /// is merged instead of duplicated.
    pub body_snippet: String,
    /// Newly detected externally-managed paths destined for
    /// `[allow_uncovered]` (already filtered against the existing
    /// config's decisions).
    pub allow_uncovered_paths: Vec<String>,
    /// Config-free advice (hint-shape detector output, e.g. the
    /// SDK-cascade suggestion). Rendered as comments into the first-run
    /// snippet only; re-runs surface it via log / CI status so `--force`
    /// never accumulates duplicate comment blocks in config.toml.
    pub advice: Vec<String>,
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
    /// Hits that emitted nothing because belaf could not classify them.
    /// Counted so the wizard summary and the `--ci` log line can say
    /// how many decisions are outstanding.
    pub needs_decision: usize,
    pub nested_npm_workspace: usize,
    pub sdk_cascade_member: usize,
    pub single_project: usize,
    pub nested_monorepo: usize,
    /// Detector hits under an existing `[allow_uncovered]` entry —
    /// scanned but not re-emitted because a human already classified
    /// them (see [`ExistingDecisions`]). Hits under `[ignore_paths]`
    /// are dropped before counting and never appear here.
    pub already_covered: usize,
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

/// Marker comment prepended to the first emitted snippet. Its presence
/// ([`is_initialized`]) distinguishes the first-run append (full
/// wrapper: marker + model header + cascade-inputs stub + advice comments)
/// from re-runs, and gates un-forced `--ci` re-appends. Idempotency of
/// the content itself comes from per-path coverage filtering
/// ([`ExistingDecisions`]), not from this marker.
const AUTO_DETECT_MARKER: &str =
    "# belaf:auto-detect-marker (do not remove — used for idempotency)";

/// F1/F4 — born-correct model documentation emitted into freshly-detected
/// configs. Auto-detect can't *classify* units (deploy/internal/ignore is a
/// human design call), so it documents the `kind` axis and leaves a
/// `[cascade_inputs]` stub instead of guessing.
const MODEL_HEADER: &str = "\n# Each unit below defaults to `kind = \"deploy\"` (versioned, tagged, released).\n# Add `kind = \"internal\"` to make a unit a cascade-only node — its changes bump\n# the deploy units that depend on it, but it is never released (e.g. internal\n# library crates, generated schemas). Add `kind = \"ignore\"` to exclude a unit\n# entirely (e.g. flat test crates).\n";

/// Stub for `[cascade_inputs]`. Deliberately framed as the general mechanism —
/// "files that feed units but aren't units" — rather than as a codegen special
/// case, because the common instances are shared base images and shared config
/// trees, not generated code.
const CASCADE_INPUTS_STUB: &str = "\n# Declared path inputs: files that feed release units without being units\n# themselves (a shared OCI base image, protobuf schemas, a shared config tree).\n# No package manager can see these edges, so declare them here — a change under\n# `paths` cascades into every unit in `affects`. Uncomment + adjust:\n# [cascade_inputs.apko-base]\n# paths   = [\"apko/base.yaml\", \"apko/*.lock\"]\n# affects = \"all-deploy-units\"   # or [\"my-service\", \"my-worker\"]\n# bump    = \"floor_minor\"        # optional; \"mirror\" (default) changes nothing\n";

const ALLOW_UNCOVERED_HEADER: &str = "\n# Mobile apps detected — handed off to Bitrise / fastlane / Codemagic.\n# Belaf doesn't manage mobile app releases; these paths are listed in\n# allow_uncovered so the drift detector doesn't fire on them.\n[allow_uncovered]\n";

/// Whether `config_text` was already populated by an auto-detect pass
/// (the [`AUTO_DETECT_MARKER`] is present). Callers use this to decide
/// between the first-run append and the `--force` re-detect path.
pub fn is_initialized(config_text: &str) -> bool {
    config_text.contains(AUTO_DETECT_MARKER)
}

/// Old single-shot entry point — equivalent to running with no
/// exclusions, no cascade overrides, and no existing config. Kept as
/// a stable public surface for existing integration tests.
pub fn run(repo: &Repository) -> AutoDetectResult {
    run_against(repo, &ExistingDecisions::default())
}

/// `--ci` entry: like [`run`] but respecting the `[ignore_paths]` /
/// `[allow_uncovered]` decisions already present in the loaded
/// configuration.
pub fn run_against(repo: &Repository, existing: &ExistingDecisions) -> AutoDetectResult {
    run_with_cascade(repo, &HashSet::new(), &[], &HashMap::new(), existing)
}

/// Backwards-compatible entry that takes only path exclusions; the
/// wizard's interactive path now calls [`run_with_cascade`] directly
/// so user-confirmed cascade-from overrides flow into the emitted
/// snippet alongside the bundle blocks.
pub fn run_filtered(repo: &Repository, exclusions: &HashSet<RepoPathBuf>) -> AutoDetectResult {
    run_with_cascade(
        repo,
        exclusions,
        &[],
        &HashMap::new(),
        &ExistingDecisions::default(),
    )
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
///
/// `existing` carries the `[ignore_paths]` / `[allow_uncovered]`
/// decisions of an already-present config — see [`ExistingDecisions`]
/// for the two suppression levels.
pub fn run_with_cascade(
    repo: &Repository,
    exclusions: &HashSet<RepoPathBuf>,
    standalones: &[StandaloneRef],
    cascade_overrides: &HashMap<String, CascadeOverrideEmit>,
    existing: &ExistingDecisions,
) -> AutoDetectResult {
    let mut report = detector::detect_all(repo);
    // `[ignore_paths]` = "belaf does not scan inside at all" — hits
    // under those paths are dropped before anything downstream (emission,
    // counters, wizard rows) can see them.
    report
        .matches
        .retain(|m| !detector::is_covered_by_config_paths(&m.path, &existing.ignore_paths));
    if !exclusions.is_empty() {
        report.matches.retain(|m| !exclusions.contains(&m.path));
    }
    let mut snippet = String::new();
    let mut advice: Vec<String> = Vec::new();
    let mut counters = DetectionCounters::default();
    // `[allow_uncovered]` = scanned but already classified by a human;
    // `unit_paths` = already claimed by a configured `[release_unit]`
    // block. Both suppress emission only (re-emitting would override
    // the decision and/or append a duplicate table, which is a TOML
    // parse error) and are counted for the summary.
    let matches: Vec<DetectorMatch> = report
        .matches
        .into_iter()
        .filter(|m| {
            let covered = detector::is_covered_by_config_paths(&m.path, &existing.allow_uncovered)
                || detector::is_covered_by_config_paths(&m.path, &existing.unit_paths);
            if covered {
                counters.already_covered += 1;
            }
            !covered
        })
        .collect();
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
    bundle::emit_all(&matches, &mut snippet, &mut counters);

    // Hints + ExternallyManaged still dispatch inline because they
    // share the `allow_uncovered` accumulator and the SDK-cascade
    // aggregated message after the loop.
    for m in &matches {
        match &m.shape {
            DetectedShape::Bundle(_) => {
                // already emitted by bundle::emit_all above
            }
            DetectedShape::Hint(h) => collect_hint_advice(&mut advice, &mut counters, m, h),
            DetectedShape::ExternallyManaged(e) => {
                register_externally_managed(&mut allow_uncovered, &mut counters, m, *e);
            }
            DetectedShape::NeedsDecision(d) => {
                register_needs_decision(&mut advice, &mut counters, m, d);
            }
        }
    }

    if counters.sdk_cascade_member > 0 && cascade_overrides.is_empty() {
        // Only surface the "consider adding cascade_from" hint when the
        // user hasn't already wired one up via the wizard's `[c]` flow.
        // If they have, the actual cascade-blocks below replace this
        // generic suggestion.
        advice.push(format!(
            "{} SDK packages detected under sdks/* — consider adding\n`cascade_from = {{ source = \"<schema-unit>\", bump = \"floor_minor\" }}`\nto each so they bump in lockstep when the schema bumps.",
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

    // Advice is config-free by design: the first-run snippet renders it
    // as comments, re-runs surface it via [`AutoDetectResult::advice`]
    // (log / CI status) so `--force` re-appends never accumulate
    // duplicate comment blocks. The [allow_uncovered] table comes last
    // so callers can substitute a toml_edit merge for it when the
    // config already has such a table (see [`apply_to_config`]).
    let mut full = format!("{snippet}{}", advice_comments(&advice));
    if !allow_uncovered.is_empty() {
        full.push_str(ALLOW_UNCOVERED_HEADER);
        full.push_str(&allow_uncovered_paths_line(&allow_uncovered));
    }
    let prefixed_snippet = if full.is_empty() {
        full
    } else {
        format!("\n{AUTO_DETECT_MARKER}\n{MODEL_HEADER}{full}{CASCADE_INPUTS_STUB}")
    };

    AutoDetectResult {
        toml_snippet: prefixed_snippet,
        body_snippet: snippet,
        allow_uncovered_paths: allow_uncovered,
        advice,
        counters,
    }
}

pub(super) fn allow_uncovered_paths_line(paths: &[String]) -> String {
    let quoted: Vec<String> = paths.iter().map(|p| toml_quote(p)).collect();
    format!("paths = [{}]\n", quoted.join(", "))
}

/// Render advice strings as `# `-prefixed comment blocks for first-run
/// config emission.
pub(super) fn advice_comments(advice: &[String]) -> String {
    let mut out = String::new();
    for a in advice {
        out.push('\n');
        for line in a.lines() {
            out.push_str("# ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Pure metadata; never togglable, never produces a `[release_unit.<name>]`.
/// Hints are advice, not config: they help the user understand what the
/// detector saw without committing config they'd have to maintain, so
/// they live in [`AutoDetectResult::advice`] rather than the body.
fn collect_hint_advice(
    advice: &mut Vec<String>,
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
            advice.push(format!(
                "Nested npm workspace detected at {path} — its members will\nbe auto-detected by the npm loader. Add an explicit [release_unit.<name>]\nentry if you want a non-default tag-format / cascade / visibility.",
            ));
        }
        HintKind::SingleProject { ecosystem } => {
            counters.single_project += 1;
            advice.push(format!(
                "Single-project repo detected ({ecosystem}) — `v{{version}}`\ntag format is suggested instead of the ecosystem default.\nOverride per-unit if you publish under a different naming convention.",
            ));
        }
        HintKind::NestedMonorepo => {
            counters.nested_monorepo += 1;
            let path = m.path.escaped();
            let note = m
                .note
                .as_deref()
                .unwrap_or("submodule looks like its own monorepo");
            advice.push(format!(
                "Nested submodule at {path} — {note}.\nConsider running `belaf init` inside the submodule and excluding\nits path from this repo's detection rather than driving both from one config.",
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

/// Unclassifiable hit: advice only, no config. Note the missing
/// `allow_uncovered.push` — that is the entire point of the class, not
/// an omission.
fn register_needs_decision(
    advice: &mut Vec<String>,
    counters: &mut DetectionCounters,
    m: &DetectorMatch,
    decision: &DecisionKind,
) {
    counters.needs_decision += 1;
    let path = m.path.escaped();
    let detail = match m.note.as_deref() {
        Some(n) => n.to_string(),
        None => match decision {
            DecisionKind::JvmVersionUnwritable { build_file, .. } => {
                format!("JVM project with no version belaf can write ({build_file})")
            }
        },
    };
    advice.push(format!(
        "{path} needs a decision — {detail}.\nLeft out of config deliberately: `prepare` will keep reporting it until it is resolved.",
    ));
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

    /// A Gradle project belaf cannot version must leave the config
    /// untouched — in particular it must not gain an `[allow_uncovered]`
    /// line. That line is what made the clikd Kotlin SDK vanish: it
    /// removed the path from the drift report while leaving it out of
    /// every release, so the gap had no way of surfacing.
    #[test]
    fn unclassifiable_jvm_project_writes_no_config() {
        let dir = tempfile::TempDir::new().unwrap();
        let _ = std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output();
        let sdk = dir.path().join("sdks/kotlin");
        std::fs::create_dir_all(&sdk).unwrap();
        std::fs::write(
            sdk.join("gradle.properties"),
            "android.useAndroidX=true\norg.gradle.jvmargs=-Xmx2048m\n",
        )
        .unwrap();
        std::fs::write(
            sdk.join("build.gradle.kts"),
            "plugins {\n    id(\"maven-publish\")\n}\n\npublishing {\n    publications {\n        register<MavenPublication>(\"release\") {\n            version = \"0.1.0\"\n        }\n    }\n}\n",
        )
        .unwrap();

        let repo = Repository::open(dir.path()).expect("open");
        let r = run(&repo);

        assert!(
            r.allow_uncovered_paths.is_empty(),
            "must not silence the drift detector for it: {:?}",
            r.allow_uncovered_paths
        );
        assert!(
            !r.body_snippet.contains("[release_unit."),
            "must not invent a release_unit block: {}",
            r.body_snippet
        );
        assert_eq!(r.counters.needs_decision, 1);
        assert_eq!(r.counters.jvm_library, 0);
        assert_eq!(r.counters.jvm_plugin_managed, 0);
        assert!(
            r.advice.iter().any(|a| a.contains("sdks/kotlin")),
            "the user has to be told: {:?}",
            r.advice
        );
    }

    /// The counterpart: a real versioning plugin still gets the
    /// hand-off it always had.
    #[test]
    fn plugin_managed_jvm_project_still_lands_in_allow_uncovered() {
        let dir = tempfile::TempDir::new().unwrap();
        let _ = std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output();
        let sdk = dir.path().join("sdks/kotlin");
        std::fs::create_dir_all(&sdk).unwrap();
        std::fs::write(
            sdk.join("build.gradle.kts"),
            "plugins { id(\"pl.allegro.tech.build.axion-release\") version \"1.18.1\" }\n",
        )
        .unwrap();

        let repo = Repository::open(dir.path()).expect("open");
        let r = run(&repo);

        assert_eq!(r.allow_uncovered_paths, vec!["sdks/kotlin/".to_string()]);
        assert_eq!(r.counters.jvm_plugin_managed, 1);
        assert_eq!(r.counters.needs_decision, 0);
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

        let result = run_with_cascade(
            &repo,
            &HashSet::new(),
            &standalones,
            &overrides,
            &ExistingDecisions::default(),
        );
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
}
