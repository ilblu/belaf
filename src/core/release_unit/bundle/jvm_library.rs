//! JVM library bundle — `gradle.properties` / `build.gradle(.kts)`
//! under `sdks/*` or `libs/*`, plus the repo root for monorepos that
//! ship one JVM artifact at the top level.
//!
//! Classification, in order. The first three produce a unit or a
//! deliberate hand-off; the fourth is the honest "ask a human".
//!
//! 1. `gradle.properties` (recommended) — a literal `version=…` line.
//! 2. `build.gradle(.kts)` literal — a line-anchored `version = "…"`.
//! 3. plugin-managed — a **known versioning plugin** is applied
//!    ([`VERSIONING_PLUGIN_IDS`]). The plugin owns the version, so
//!    belaf stays out and the path goes to `[allow_uncovered]`, same
//!    mental model as Mobile (Fastlane/Bitrise).
//! 4. everything else — [`DecisionKind::JvmVersionUnwritable`]: a JVM
//!    project whose version belaf can neither read nor write, with no
//!    plugin to hand it off to. Emits nothing and keeps signalling
//!    drift.
//!
//! Step 3 used to be a bare `else`: *any* build script without a
//! writable version was called plugin-managed and silenced via
//! `[allow_uncovered]`. Nothing checked that a versioning plugin was
//! actually applied. A Gradle project that keeps a plain literal
//! version somewhere the line-anchored rewriter cannot reach — inside
//! `publishing { publications { … } }`, say — therefore disappeared
//! from the release set *and* took the drift warning down with it, so
//! the only signal that would have reported the gap was removed by the
//! same misclassification. Steps 3 and 4 are now separate questions:
//! "is a plugin in charge?" and, only if not, "can I write the
//! version?".

use std::path::{Path, PathBuf};

use super::super::shape::{
    BundleKind, DecisionKind, DetectedShape, DetectorMatch, ExtKind, GradleBuildFile,
    JvmUnwritableReason, JvmVersionSource,
};
use super::super::walk::{file_contains_line, file_contains_pattern, relative_repopath};

use crate::cmd::init::auto_detect::DetectionCounters;
use crate::cmd::init::toml_util::toml_quote;

/// Gradle plugins that take ownership of a project's version. A hit on
/// any of these is what licenses belaf to step aside — the user picked
/// a release tool, and rewriting `version` behind its back would fight
/// it. Matched as a plain substring of the build script: these ids are
/// long and distinctive, and the same id appears in the Kotlin DSL
/// (`id("…")`), the Groovy DSL (`id '…'`) and the legacy form
/// (`apply plugin: '…'`) alike, so one substring covers all three
/// spellings without three regexes to keep in sync.
///
/// Unknown-but-real versioning plugins are the expected miss here, and
/// they fail safe: the project falls through to step 4 and is
/// *reported* rather than silently dropped. Adding an id is a one-line
/// change.
const VERSIONING_PLUGIN_IDS: &[&str] = &[
    "pl.allegro.tech.build.axion-release",
    "com.netflix.nebula.release",
    "nebula.release",
    "io.github.reactivecircus.app-versioning",
    "com.palantir.git-version",
    "org.ajoberstar.reckon",
    "net.researchgate.release",
    "me.qoomon.git-versioning",
    "com.cinnober.gradle.semver-git",
    "org.shipkit.shipkit-auto-version",
];

/// Line-anchored `version = "…"` — the only shape
/// [`JvmVersionSource::BuildGradleKtsLiteral`]'s rewriter can write.
const LINE_ANCHORED_VERSION: &str = r#"(?m)^version\s*=\s*""#;

/// The same assignment anywhere on a line, indentation allowed. Used
/// only to tell "nested, move it" apart from "no version at all" in
/// the [`JvmUnwritableReason`] we report.
const ANY_VERSION_ASSIGNMENT: &str = r#"(?m)^\s*version\s*=\s*""#;

pub fn detect(workdir: &Path) -> Vec<DetectorMatch> {
    let mut out = Vec::new();
    for dir in collect_candidates(workdir) {
        let Some(repopath) = relative_repopath(workdir, &dir) else {
            continue;
        };

        // 1 — the recommended source: a plain `version=` line belaf can
        // read and write without touching the build script at all.
        let gp = dir.join("gradle.properties");
        if gp.exists() && file_contains_line(&gp, "version=") {
            out.push(bundle_match(repopath, JvmVersionSource::GradleProperties));
            continue;
        }

        let Some((build_path, build_file)) = build_script(&dir) else {
            continue;
        };

        // 2 — a version assignment the line-anchored rewriter can reach.
        if file_contains_pattern(&build_path, LINE_ANCHORED_VERSION) {
            out.push(bundle_match(
                repopath,
                JvmVersionSource::BuildScriptLiteral { build_file },
            ));
            continue;
        }

        // 3 — a known plugin positively claims the version. Only a
        // named match hands the project off; the absence of a writable
        // version is not itself evidence that anything else manages it.
        if let Some(plugin) = versioning_plugin(workdir, &dir, &build_path) {
            out.push(DetectorMatch {
                shape: DetectedShape::ExternallyManaged(ExtKind::JvmPluginManaged),
                path: repopath,
                note: Some(format!(
                    "versioning owned by the `{plugin}` Gradle plugin — use its release flow"
                )),
            });
            continue;
        }

        // 4 — recognised, unclassifiable. Report it and let a human
        // decide; writing `[allow_uncovered]` here would drop the
        // artifact from every release and silence the drift check that
        // is the only thing that would have said so.
        let reason = unwritable_reason(&build_path);
        out.push(DetectorMatch {
            note: Some(unwritable_note(build_file, &reason)),
            shape: DetectedShape::NeedsDecision(DecisionKind::JvmVersionUnwritable {
                build_file,
                reason,
            }),
            path: repopath,
        });
    }
    out
}

fn bundle_match(
    path: crate::core::git::repository::RepoPathBuf,
    v: JvmVersionSource,
) -> DetectorMatch {
    DetectorMatch {
        note: Some(jvm_label(&v)),
        shape: DetectedShape::Bundle(BundleKind::JvmLibrary { version_source: v }),
        path,
    }
}

/// The project's build script, Kotlin DSL preferred when both exist —
/// Gradle itself evaluates only one, and a project mid-migration keeps
/// the version in the one it actually builds with.
fn build_script(dir: &Path) -> Option<(PathBuf, GradleBuildFile)> {
    let kts = dir.join("build.gradle.kts");
    if kts.is_file() {
        return Some((kts, GradleBuildFile::Kts));
    }
    let groovy = dir.join("build.gradle");
    if groovy.is_file() {
        return Some((groovy, GradleBuildFile::Groovy));
    }
    None
}

/// A project declares its own build when it carries a settings script;
/// otherwise it is a subproject of an enclosing Gradle build, whose
/// root script may apply the versioning plugin on its behalf
/// (`allprojects { version = scmVersion.version }` is the canonical
/// axion-release shape). Scanning the root only in that case keeps the
/// answer true to how Gradle resolves it: a directory with its own
/// `settings.gradle{.kts}` is an independent build and inherits
/// nothing from the repo root.
fn owns_its_build(dir: &Path) -> bool {
    dir.join("settings.gradle.kts").is_file() || dir.join("settings.gradle").is_file()
}

/// Name of the first known versioning plugin applied to this project,
/// looking at its own build script and — for a subproject of an outer
/// build — the enclosing root script.
fn versioning_plugin(workdir: &Path, dir: &Path, build_path: &Path) -> Option<&'static str> {
    if let Some(id) = plugin_id_in(build_path) {
        return Some(id);
    }
    if dir == workdir || owns_its_build(dir) {
        return None;
    }
    let (root_script, _) = build_script(workdir)?;
    plugin_id_in(&root_script)
}

fn plugin_id_in(path: &Path) -> Option<&'static str> {
    let content = std::fs::read_to_string(path).ok()?;
    VERSIONING_PLUGIN_IDS
        .iter()
        .find(|id| content.contains(**id))
        .copied()
}

/// Whether the unreachable version is a nested assignment (a one-line
/// move fixes it) or missing entirely (a design question).
fn unwritable_reason(build_path: &Path) -> JvmUnwritableReason {
    let Ok(content) = std::fs::read_to_string(build_path) else {
        return JvmUnwritableReason::Absent;
    };
    let Ok(re) = regex::Regex::new(ANY_VERSION_ASSIGNMENT) else {
        return JvmUnwritableReason::Absent;
    };
    content
        .lines()
        .position(|l| re.is_match(l))
        .map(|i| JvmUnwritableReason::NotAtLineStart { line: i + 1 })
        .unwrap_or(JvmUnwritableReason::Absent)
}

/// The remediation sentence carried on the match — reused verbatim by
/// the drift error and the init wizard so a user reads the same
/// instruction wherever the hit surfaces.
pub fn unwritable_note(build_file: GradleBuildFile, reason: &JvmUnwritableReason) -> String {
    match reason {
        JvmUnwritableReason::NotAtLineStart { line } => format!(
            "JVM project with no version belaf can write: {build_file}:{line} assigns `version = \"…\"` inside a block, and the rewriter only writes an assignment at the start of a line. Move the version to `gradle.properties` as `version=<current>` and read it back in the build script, then re-run `belaf init --auto-detect`"
        ),
        JvmUnwritableReason::Absent => format!(
            "JVM project with no version belaf can write: no `version=` in gradle.properties, no `version = \"…\"` in {build_file}, and no known versioning plugin applied. Add `version=<current>` to gradle.properties and re-run `belaf init --auto-detect`, or add the path to [allow_uncovered] if it is genuinely never released"
        ),
    }
}

/// Emit blocks for every JvmLibrary match in the slice. Filters out
/// non-JvmLibrary matches; safe to call with an unfiltered slice (the
/// dispatch in `bundle::emit_all` passes only Bundle matches).
pub fn emit_all(
    matches: &[&DetectorMatch],
    snippet: &mut String,
    counters: &mut DetectionCounters,
) {
    for m in matches {
        if matches!(
            m.shape,
            DetectedShape::Bundle(BundleKind::JvmLibrary { .. })
        ) {
            emit_block(m, snippet, counters);
        }
    }
}

fn emit_block(m: &DetectorMatch, snippet: &mut String, counters: &mut DetectionCounters) {
    let DetectedShape::Bundle(BundleKind::JvmLibrary { version_source }) = &m.shape else {
        return;
    };
    counters.jvm_library += 1;
    let path = m.path.escaped();
    let name_raw = path.rsplit('/').next().unwrap_or("sdk");
    let satellites_q = toml_quote(&path);
    let (vfield, manifest_raw) = match version_source {
        JvmVersionSource::GradleProperties => {
            ("gradle_properties", format!("{path}/gradle.properties"))
        }
        JvmVersionSource::BuildScriptLiteral { build_file } => {
            ("generic_regex", format!("{path}/{}", build_file.filename()))
        }
    };
    let manifest_q = toml_quote(&manifest_raw);
    if vfield == "generic_regex" {
        snippet.push_str(&format!(
            "\n[release_unit.{name_raw}]\necosystem = \"jvm-library\"\nsatellites = [{satellites_q}]\nmanifests = [{{ path = {manifest_q}, version_field = \"generic_regex\", regex_pattern = '(?m)^version\\s*=\\s*\"([^\"]+)\"', regex_replace = \"version = \\\"{{version}}\\\"\" }}]\n",
        ));
    } else {
        snippet.push_str(&format!(
            "\n[release_unit.{name_raw}]\necosystem = \"jvm-library\"\nsatellites = [{satellites_q}]\nmanifests = [{{ path = {manifest_q}, version_field = \"{vfield}\" }}]\n",
        ));
    }
}

fn jvm_label(s: &JvmVersionSource) -> String {
    match s {
        JvmVersionSource::GradleProperties => "gradle.properties (recommended)".to_string(),
        JvmVersionSource::BuildScriptLiteral { build_file } => {
            format!("literal version in {build_file}")
        }
    }
}

fn collect_candidates(workdir: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(workdir.join("sdks")) {
        for e in entries.flatten() {
            if e.path().is_dir() {
                dirs.push(e.path());
            }
        }
    }
    if let Ok(entries) = std::fs::read_dir(workdir.join("libs")) {
        for e in entries.flatten() {
            if e.path().is_dir() {
                dirs.push(e.path());
            }
        }
    }
    if workdir.join("gradle.properties").exists() || workdir.join("build.gradle.kts").exists() {
        dirs.push(workdir.to_path_buf());
    }
    dirs
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

    fn only(matches: &[DetectorMatch]) -> &DetectorMatch {
        assert_eq!(matches.len(), 1, "expected exactly one match: {matches:?}");
        &matches[0]
    }

    #[test]
    fn gradle_properties() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("sdks/kotlin/gradle.properties"),
            "version=0.1.0\n",
        );
        let matches = detect(root);
        match &only(&matches).shape {
            DetectedShape::Bundle(BundleKind::JvmLibrary { version_source }) => {
                assert_eq!(*version_source, JvmVersionSource::GradleProperties);
            }
            other => panic!("expected JvmLibrary bundle, got {other:?}"),
        }
    }

    #[test]
    fn build_gradle_kts_literal() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("sdks/java/build.gradle.kts"),
            "plugins {}\nversion = \"0.1.0\"\n",
        );
        let matches = detect(root);
        match &only(&matches).shape {
            DetectedShape::Bundle(BundleKind::JvmLibrary { version_source }) => {
                assert_eq!(
                    *version_source,
                    JvmVersionSource::BuildScriptLiteral {
                        build_file: GradleBuildFile::Kts
                    }
                );
            }
            other => panic!("expected JvmLibrary bundle, got {other:?}"),
        }
    }

    #[test]
    fn groovy_build_script_emits_a_groovy_manifest_path() {
        // A Groovy project keeps its version in `build.gradle`. Emitting
        // a `build.gradle.kts` manifest would point the rewriter at a
        // file that does not exist, and the unit would fail at release
        // time rather than at detect time.
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("libs/legacy/build.gradle"),
            "plugins {}\nversion = \"1.2.3\"\n",
        );
        let matches = detect(root);
        let m = only(&matches);
        match &m.shape {
            DetectedShape::Bundle(BundleKind::JvmLibrary { version_source }) => {
                assert_eq!(
                    *version_source,
                    JvmVersionSource::BuildScriptLiteral {
                        build_file: GradleBuildFile::Groovy
                    }
                );
            }
            other => panic!("expected JvmLibrary bundle, got {other:?}"),
        }

        let mut snippet = String::new();
        let mut counters = DetectionCounters::default();
        emit_all(&[m], &mut snippet, &mut counters);
        assert!(
            snippet.contains("libs/legacy/build.gradle\""),
            "manifest must point at the Groovy script: {snippet}"
        );
        assert!(
            !snippet.contains("build.gradle.kts"),
            "must not invent a Kotlin DSL file: {snippet}"
        );
    }

    #[test]
    fn plugin_managed_requires_a_named_plugin() {
        // Positive match on a known versioning plugin — the user picked
        // a release tool, so belaf hands the path off to
        // [allow_uncovered] and stays out of its way.
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("sdks/javakt/build.gradle.kts"),
            "plugins { id(\"io.github.reactivecircus.app-versioning\") version \"1.3.1\" }\n",
        );
        let matches = detect(root);
        let m = only(&matches);
        assert!(
            matches!(
                m.shape,
                DetectedShape::ExternallyManaged(ExtKind::JvmPluginManaged)
            ),
            "expected JvmPluginManaged, got {:?}",
            m.shape
        );
        assert!(
            m.note
                .as_deref()
                .unwrap_or_default()
                .contains("io.github.reactivecircus.app-versioning"),
            "the note must name the plugin that claimed the version: {:?}",
            m.note
        );
    }

    #[test]
    fn axion_release_in_groovy_dsl_is_recognised() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("libs/svc/build.gradle"),
            "apply plugin: 'pl.allegro.tech.build.axion-release'\n",
        );
        let matches = detect(root);
        assert!(
            matches!(
                only(&matches).shape,
                DetectedShape::ExternallyManaged(ExtKind::JvmPluginManaged)
            ),
            "the legacy `apply plugin:` spelling must count: {matches:?}"
        );
    }

    #[test]
    fn indented_version_needs_a_decision_and_names_its_line() {
        // The clikd regression. `version = "…"` exists but sits inside
        // `publishing { publications { … } }`, so the line-anchored
        // rewriter cannot address it — and no versioning plugin is
        // applied. Before, this was called "plugin-managed" and
        // silenced via [allow_uncovered], which dropped a real SDK from
        // every release and switched off the drift check that would
        // have said so.
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("sdks/kotlin/gradle.properties"),
            "android.useAndroidX=true\norg.gradle.jvmargs=-Xmx2048m\n",
        );
        write(
            &root.join("sdks/kotlin/build.gradle.kts"),
            "plugins {\n    id(\"maven-publish\")\n}\n\nafterEvaluate {\n    publishing {\n        publications {\n            register<MavenPublication>(\"release\") {\n                version = \"0.1.0\"\n            }\n        }\n    }\n}\n",
        );
        let matches = detect(root);
        let m = only(&matches);
        match &m.shape {
            DetectedShape::NeedsDecision(DecisionKind::JvmVersionUnwritable {
                build_file,
                reason,
            }) => {
                assert_eq!(*build_file, GradleBuildFile::Kts);
                assert_eq!(*reason, JvmUnwritableReason::NotAtLineStart { line: 9 });
            }
            other => panic!("expected NeedsDecision, got {other:?}"),
        }
        let note = m.note.as_deref().unwrap_or_default();
        assert!(
            note.contains("build.gradle.kts:9"),
            "the note must point at the offending line: {note}"
        );
        assert!(
            note.contains("gradle.properties"),
            "the note must say where to move the version: {note}"
        );
    }

    #[test]
    fn indented_version_emits_no_config_at_all() {
        // The invariant that makes the class worth having: a hit belaf
        // cannot classify must not produce a release_unit block *or* an
        // allow_uncovered line. Both would end the drift report; only
        // one of them would be wrong quietly.
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("sdks/kotlin/build.gradle.kts"),
            "allprojects {\n    version = \"0.1.0\"\n}\n",
        );
        let matches = detect(root);
        let m = only(&matches);
        assert!(m.shape.is_needs_decision(), "got {:?}", m.shape);

        let mut snippet = String::new();
        let mut counters = DetectionCounters::default();
        emit_all(&[m], &mut snippet, &mut counters);
        assert!(
            snippet.is_empty(),
            "no release_unit block may be emitted: {snippet}"
        );
        assert_eq!(counters.jvm_library, 0);
    }

    #[test]
    fn no_version_anywhere_reports_absent() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("libs/thing/build.gradle.kts"),
            "plugins { id(\"java-library\") }\ndependencies {}\n",
        );
        let matches = detect(root);
        match &only(&matches).shape {
            DetectedShape::NeedsDecision(DecisionKind::JvmVersionUnwritable {
                reason,
                build_file,
            }) => {
                assert_eq!(*reason, JvmUnwritableReason::Absent);
                assert_eq!(*build_file, GradleBuildFile::Kts);
            }
            other => panic!("expected NeedsDecision/Absent, got {other:?}"),
        }
    }

    #[test]
    fn subproject_inherits_the_root_builds_versioning_plugin() {
        // `allprojects { version = scmVersion.version }` in the root is
        // the canonical axion-release shape. The subproject has no
        // settings script of its own, so it really is governed by that
        // root build.
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("build.gradle.kts"),
            "plugins { id(\"pl.allegro.tech.build.axion-release\") version \"1.18.1\" }\nallprojects { version = scmVersion.version }\n",
        );
        write(
            &root.join("libs/core/build.gradle.kts"),
            "plugins { id(\"java-library\") }\n",
        );
        let matches = detect(root);
        let sub = matches
            .iter()
            .find(|m| m.path.escaped() == "libs/core")
            .expect("subproject must be detected");
        assert!(
            matches!(
                sub.shape,
                DetectedShape::ExternallyManaged(ExtKind::JvmPluginManaged)
            ),
            "expected the root plugin to cover it, got {:?}",
            sub.shape
        );
    }

    #[test]
    fn independent_build_does_not_inherit_the_root_plugin() {
        // With its own `settings.gradle.kts` the directory is a separate
        // Gradle build: the repo-root script never evaluates for it, so
        // borrowing the root's plugin would be a false hand-off — and a
        // false hand-off is exactly an [allow_uncovered] line for a
        // project nothing is versioning.
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("build.gradle.kts"),
            "plugins { id(\"pl.allegro.tech.build.axion-release\") version \"1.18.1\" }\n",
        );
        write(
            &root.join("sdks/kotlin/settings.gradle.kts"),
            "rootProject.name = \"sdk\"\n",
        );
        write(
            &root.join("sdks/kotlin/build.gradle.kts"),
            "plugins { id(\"maven-publish\") }\n",
        );
        let matches = detect(root);
        let sub = matches
            .iter()
            .find(|m| m.path.escaped() == "sdks/kotlin")
            .expect("sdk must be detected");
        assert!(
            sub.shape.is_needs_decision(),
            "an independent build must not inherit the root plugin, got {:?}",
            sub.shape
        );
    }

    #[test]
    fn gradle_properties_wins_over_an_applied_plugin() {
        // A writable version belaf owns outright beats the hand-off:
        // plenty of projects apply a release plugin for tagging while
        // keeping the version itself in gradle.properties.
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(&root.join("libs/x/gradle.properties"), "version=2.0.0\n");
        write(
            &root.join("libs/x/build.gradle.kts"),
            "plugins { id(\"net.researchgate.release\") version \"3.0.2\" }\n",
        );
        let matches = detect(root);
        match &only(&matches).shape {
            DetectedShape::Bundle(BundleKind::JvmLibrary { version_source }) => {
                assert_eq!(*version_source, JvmVersionSource::GradleProperties);
            }
            other => panic!("expected the gradle.properties bundle, got {other:?}"),
        }
    }
}
