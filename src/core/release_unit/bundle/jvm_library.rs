//! JVM library bundle — `gradle.properties` / `build.gradle(.kts)`
//! under `sdks/*` or `libs/*`, plus the repo root for monorepos that
//! ship one JVM artifact at the top level.
//!
//! Three version sources, each with a different rewriter:
//!
//! - `gradle.properties` (recommended) — a literal `version=…` line.
//! - `build.gradle.kts` literal — `version = "…"` in the Kotlin DSL.
//! - plugin-managed — version derived from a Gradle plugin
//!   (`io.github.reactivecircus.app-versioning`, etc.). Emitted as
//!   an `external_versioner` `[release_unit]` because we can't write
//!   the version through file rewrites.

use std::path::{Path, PathBuf};

use super::super::shape::{BundleKind, DetectedShape, DetectorMatch, JvmVersionSource};
use super::super::walk::{file_contains_line, file_contains_pattern, relative_repopath};

use crate::cmd::init::auto_detect::DetectionCounters;
use crate::cmd::init::toml_util::toml_quote;

pub fn detect(workdir: &Path) -> Vec<DetectorMatch> {
    let mut out = Vec::new();
    let candidates = collect_candidates(workdir);
    for dir in candidates {
        let gp = dir.join("gradle.properties");
        let bgk = dir.join("build.gradle.kts");
        let bg = dir.join("build.gradle");

        let kind = if gp.exists() && file_contains_line(&gp, "version=") {
            JvmVersionSource::GradleProperties
        } else if (bgk.exists() && file_contains_pattern(&bgk, r#"version\s*=\s*""#))
            || (bg.exists() && file_contains_pattern(&bg, r#"version\s*=\s*""#))
        {
            JvmVersionSource::BuildGradleKtsLiteral
        } else if bgk.exists() || bg.exists() {
            JvmVersionSource::PluginManaged
        } else {
            continue;
        };

        let repopath = match relative_repopath(workdir, &dir) {
            Some(r) => r,
            None => continue,
        };
        out.push(DetectorMatch {
            shape: DetectedShape::Bundle(BundleKind::JvmLibrary {
                version_source: kind.clone(),
            }),
            path: repopath,
            note: Some(jvm_label(&kind).to_string()),
        });
    }
    out
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
        if matches!(m.shape, DetectedShape::Bundle(BundleKind::JvmLibrary { .. })) {
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

fn jvm_label(s: &JvmVersionSource) -> &'static str {
    match s {
        JvmVersionSource::GradleProperties => "gradle.properties (recommended)",
        JvmVersionSource::BuildGradleKtsLiteral => "literal version in build.gradle(.kts)",
        JvmVersionSource::PluginManaged => "plugin-managed (suggest external_versioner)",
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

    #[test]
    fn gradle_properties() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("sdks/kotlin/gradle.properties"),
            "version=0.1.0\n",
        );
        let matches = detect(root);
        assert_eq!(matches.len(), 1);
        match &matches[0].shape {
            DetectedShape::Bundle(BundleKind::JvmLibrary { version_source }) => {
                assert_eq!(*version_source, JvmVersionSource::GradleProperties);
            }
            _ => panic!("expected JvmLibrary bundle"),
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
        assert_eq!(matches.len(), 1);
        match &matches[0].shape {
            DetectedShape::Bundle(BundleKind::JvmLibrary { version_source }) => {
                assert_eq!(*version_source, JvmVersionSource::BuildGradleKtsLiteral);
            }
            _ => panic!("expected JvmLibrary bundle"),
        }
    }

    #[test]
    fn plugin_managed() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("sdks/javakt/build.gradle.kts"),
            "plugins { id(\"io.github.reactivecircus.app-versioning\") version \"1.3.1\" }\n",
        );
        let matches = detect(root);
        assert_eq!(matches.len(), 1);
        match &matches[0].shape {
            DetectedShape::Bundle(BundleKind::JvmLibrary { version_source }) => {
                assert_eq!(*version_source, JvmVersionSource::PluginManaged);
            }
            _ => panic!("expected JvmLibrary bundle"),
        }
    }
}
