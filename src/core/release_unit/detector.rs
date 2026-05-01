//! Auto-detectors that scan a repository and propose ReleaseUnit
//! configuration in the wizard. Plus the always-on drift detector
//! that fires at `belaf prepare` time.
//!
//! Phase F of `BELAF_MASTER_PLAN.md`. Each `detect_*` function is a
//! pure scan: takes a [`Repository`], returns a `Vec<DetectorMatch>`.
//! [`detect_all`] aggregates them; the wizard (Phase I) renders the
//! results as a confirmable list.
//!
//! TODO(belaf-3.0/wave1e): split this 1192-LOC file into
//! `detector/{hexagonal,tauri,kotlin_jvm,mobile,sdk_cascade,
//! single_project,nested_monorepo,drift,common}.rs` in a focused
//! cleanup PR. The new detectors landed in Wave 1e directly here
//! because each helper function (`is_covered`, `is_tauri_single_source`)
//! is shared across detectors and needs careful visibility extraction
//! before a physical split is safe.
//!
//! Drift detection in [`detect_drift`] runs at every prepare,
//! regardless of TUI/CI mode (Phase H).

use std::path::{Path, PathBuf};

use crate::core::config::ConfigurationFile;
use crate::core::git::repository::{RepoPathBuf, Repository};

use super::ResolvedReleaseUnit;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// One match produced by a detector: the directory hit, the kind of
/// pattern matched, plus optional sub-classification details so the
/// wizard can render specific TUI prompts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectorMatch {
    pub kind: DetectorKind,
    pub path: RepoPathBuf,
    pub note: Option<String>,
}

/// Stable detector identifiers + sub-classification for the variants
/// that have meaningfully-different remediation paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetectorKind {
    /// Phase F.1 — hexagonal Cargo service. `D/crates/{bin,lib,workers,
    /// basename}` exists with `[package]`.
    HexagonalCargo {
        /// Which sub-crate type was detected as the public face.
        primary: HexagonalPrimary,
    },

    /// Phase F.2 — Tauri triplet. `D/package.json` +
    /// `D/src-tauri/Cargo.toml` + `D/src-tauri/tauri.conf.json`.
    Tauri {
        /// Single-source: `tauri.conf.json` references
        /// `"../package.json"`. Legacy multi-file: hand-managed
        /// versions in all three files.
        single_source: bool,
    },

    /// Phase F.3 — JVM library SDK.
    JvmLibrary {
        /// Where the version actually lives.
        version_source: JvmVersionSource,
    },

    /// Phase F.4 — mobile app (warning only — not configured).
    MobileApp { platform: MobilePlatform },

    /// Phase F.6 — nested npm workspace.
    NestedNpmWorkspace,

    /// Phase F.7 — generated-SDK package within an `sdks/*` tree.
    SdkCascadeMember,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HexagonalPrimary {
    Bin,
    Lib,
    Workers,
    BaseName,
}

impl std::fmt::Display for HexagonalPrimary {
    /// Lowercase so error messages and config snippets read uniformly
    /// (`crates/bin/Cargo.toml`) rather than the PascalCase that the
    /// `Debug` derive would produce.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            HexagonalPrimary::Bin => "bin",
            HexagonalPrimary::Lib => "lib",
            HexagonalPrimary::Workers => "workers",
            HexagonalPrimary::BaseName => "basename",
        };
        f.write_str(s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JvmVersionSource {
    /// `gradle.properties` has `version=...` (recommended).
    GradleProperties,
    /// `build.gradle.kts` has literal `version = "..."` (acceptable
    /// via GenericRegex).
    BuildGradleKtsLiteral,
    /// Plugin-managed (ReactiveCircus, etc.) — version is computed
    /// from git tags. Suggest external_versioner.
    PluginManaged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobilePlatform {
    Ios,
    Android,
}

/// Aggregate report covering every detector hit in the repo.
#[derive(Debug, Default)]
pub struct DetectionReport {
    pub matches: Vec<DetectorMatch>,
}

impl DetectionReport {
    pub fn matches_of(
        &self,
        kind_predicate: impl Fn(&DetectorKind) -> bool,
    ) -> Vec<&DetectorMatch> {
        self.matches
            .iter()
            .filter(|m| kind_predicate(&m.kind))
            .collect()
    }

    pub fn is_single_mobile_repo(&self) -> bool {
        // Phase F.5: only mobile_app matches and nothing else.
        !self.matches.is_empty()
            && self
                .matches
                .iter()
                .all(|m| matches!(m.kind, DetectorKind::MobileApp { .. }))
    }

    pub fn count_release_unit_candidates(&self) -> usize {
        // Phase F.8: every non-mobile-warning match is a candidate
        // ReleaseUnit. Mobile apps are intentionally excluded
        // (handled via [allow_uncovered] instead of being released
        // by belaf).
        self.matches
            .iter()
            .filter(|m| !matches!(m.kind, DetectorKind::MobileApp { .. }))
            .count()
    }
}

/// One detector hit that isn't covered by any resolved ReleaseUnit,
/// `[ignore_paths]`, or `[allow_uncovered]` — listed in the drift
/// error so the user can remediate it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UncoveredHit {
    pub path: RepoPathBuf,
    pub kind: DetectorKind,
}

/// Drift report: paths matching a detector pattern but not covered
/// by any resolved ReleaseUnit, ignore_paths, or allow_uncovered.
#[derive(Debug, Default)]
pub struct DriftReport {
    pub uncovered: Vec<UncoveredHit>,
}

impl DriftReport {
    pub fn is_empty(&self) -> bool {
        self.uncovered.is_empty()
    }

    /// Format per `BELAF_MASTER_PLAN.md` §3.9 — actionable, lists
    /// every uncovered hit with its detector kind, suggests the
    /// three remediation paths.
    pub fn format_error(&self) -> String {
        let mut s = String::from(
            "✗ belaf prepare detected uncovered release artifacts.\n\nThe following paths match known detector patterns but are not part of any ReleaseUnit:\n",
        );
        for hit in &self.uncovered {
            let path_lit = hit.path.escaped();
            let label = drift_kind_label(&hit.kind);
            s.push_str(&format!("  - {path_lit:<48}  ({label})\n"));
        }
        s.push_str(
            "\nChoose one:\n  → run `belaf init --ci --auto-detect --force` to re-detect bundles and append release_unit blocks to belaf/config.toml (idempotent — the auto-detect marker prevents duplicate appends)\n  → add explicit [[release_unit]] entries\n  → if intentional (mobile app, archive, etc.), add to [ignore_paths] or [allow_uncovered]\n\nAborting prepare. No releases will be drafted.",
        );
        s
    }
}

fn drift_kind_label(k: &DetectorKind) -> String {
    match k {
        DetectorKind::HexagonalCargo { primary } => {
            format!("hexagonal cargo: crates/{primary}/Cargo.toml present")
        }
        DetectorKind::Tauri { single_source } => format!(
            "tauri triplet ({})",
            if *single_source {
                "single-source"
            } else {
                "legacy multi-file"
            }
        ),
        DetectorKind::JvmLibrary { version_source } => {
            format!("jvm library ({})", jvm_label(version_source))
        }
        DetectorKind::MobileApp { platform } => match platform {
            MobilePlatform::Ios => "iOS app — recommend Bitrise/fastlane".to_string(),
            MobilePlatform::Android => "Android app — recommend Bitrise/Codemagic".to_string(),
        },
        DetectorKind::NestedNpmWorkspace => "nested npm workspace".to_string(),
        DetectorKind::SdkCascadeMember => "generated SDK package under sdks/*".to_string(),
    }
}

/// Phase H — `pre_prepare_drift_check`. Convenience wrapper that
/// runs [`detect_drift`] and returns an error with the §3.9-format
/// message when uncovered hits exist. Callers (the prepare command,
/// hooked up in Phase I) just `?`-propagate.
pub fn pre_prepare_drift_check(
    repo: &Repository,
    resolved: &[ResolvedReleaseUnit],
    cfg: &ConfigurationFile,
) -> std::result::Result<(), String> {
    let report = detect_drift(repo, resolved, cfg);
    if report.is_empty() {
        Ok(())
    } else {
        Err(report.format_error())
    }
}

/// Drift check variant that takes the path lists directly instead of
/// requiring a full [`ConfigurationFile`]. Useful when callers don't
/// hold the parsed config object — e.g. [`crate::core::session::AppSession`]
/// stashes only the paths it needs and then calls this.
pub fn pre_prepare_drift_check_paths(
    repo: &Repository,
    resolved: &[ResolvedReleaseUnit],
    ignore_paths: &[String],
    allow_uncovered: &[String],
) -> std::result::Result<(), String> {
    let report = detect_drift_paths(repo, resolved, ignore_paths, allow_uncovered);
    if report.is_empty() {
        Ok(())
    } else {
        Err(report.format_error())
    }
}

/// Like [`detect_drift`] but accepts the ignore / allow path lists
/// directly. Avoids forcing every caller to hand-build a
/// [`ConfigurationFile`] just to call the drift check.
pub fn detect_drift_paths(
    repo: &Repository,
    resolved: &[ResolvedReleaseUnit],
    ignore_paths: &[String],
    allow_uncovered: &[String],
) -> DriftReport {
    let report = detect_all(repo);
    detect_drift_from_report(&report, resolved, ignore_paths, allow_uncovered)
}

/// Drift check variant that consumes a pre-computed
/// [`DetectionReport`]. Intended for callers that already cache the
/// detection output (e.g. [`crate::core::session::AppSession`]) and
/// want to avoid re-walking the filesystem. The `_paths` and
/// `ConfigurationFile`-based variants both delegate to this once
/// they have a report in hand.
pub fn detect_drift_from_report(
    report: &DetectionReport,
    resolved: &[ResolvedReleaseUnit],
    ignore_paths: &[String],
    allow_uncovered: &[String],
) -> DriftReport {
    let mut coverage: Vec<RepoPathBuf> = Vec::new();
    for r in resolved {
        if let super::VersionSource::Manifests(ms) = &r.unit.source {
            for m in ms {
                if let Some(parent) = Path::new(&m.path.escaped().to_string()).parent() {
                    let p = parent.to_string_lossy().to_string();
                    if !p.is_empty() {
                        coverage.push(RepoPathBuf::new(p.as_bytes()));
                    }
                }
            }
        }
        for s in &r.unit.satellites {
            coverage.push(s.clone());
        }
    }
    for p in ignore_paths {
        coverage.push(RepoPathBuf::new(p.trim_end_matches('/').as_bytes()));
    }
    for p in allow_uncovered {
        coverage.push(RepoPathBuf::new(p.trim_end_matches('/').as_bytes()));
    }

    let uncovered: Vec<UncoveredHit> = report
        .matches
        .iter()
        .filter(|m| !is_covered(&m.path, &coverage))
        .map(|m| UncoveredHit {
            path: m.path.clone(),
            kind: m.kind.clone(),
        })
        .collect();

    DriftReport { uncovered }
}

// ---------------------------------------------------------------------------
// Top-level dispatch
// ---------------------------------------------------------------------------

/// Run every init-time detector against the repo's working tree.
/// Used by `belaf init` to seed the TUI list.
pub fn detect_all(repo: &Repository) -> DetectionReport {
    let workdir = match workdir(repo) {
        Some(w) => w,
        None => return DetectionReport::default(),
    };

    let mut matches = Vec::new();
    matches.extend(hexagonal_cargo(&workdir));
    matches.extend(tauri(&workdir));
    matches.extend(jvm_library(&workdir));
    matches.extend(mobile_app(&workdir));
    matches.extend(nested_npm_workspace(&workdir));
    matches.extend(sdk_cascade_members(&workdir));

    DetectionReport { matches }
}

/// Drift check: find paths matching a detector pattern that are not
/// covered by any resolved ReleaseUnit, `[ignore_paths]`, or
/// `[allow_uncovered]`. Runs on every `belaf prepare` (Phase H).
/// Delegates to [`detect_drift_from_report`] so the coverage-set
/// construction lives in one place.
pub fn detect_drift(
    repo: &Repository,
    resolved: &[ResolvedReleaseUnit],
    cfg: &ConfigurationFile,
) -> DriftReport {
    let report = detect_all(repo);
    detect_drift_from_report(
        &report,
        resolved,
        &cfg.ignore_paths.paths,
        &cfg.allow_uncovered.paths,
    )
}

/// A detector-hit path is "covered" iff one of:
///
///   - the hit is **inside** a coverage path (the standard case —
///     coverage is `apps/services/aura`, hit is `apps/services/aura/crates/bin`),
///     OR
///   - the hit is an **ancestor** of a coverage path (the inverted
///     case — coverage is `apps/services/aura/crates` (the satellite),
///     hit is `apps/services/aura` (the service dir the hexagonal-cargo
///     detector reports). The deeper satellite means the user has
///     definitely claimed the parent service even if no explicit
///     entry names it).
///
/// Without the ancestor branch, hexagonal-cargo services with their
/// canonical `satellites = ["{path}/crates"]` config drift on every
/// `belaf prepare`.
fn is_covered(path: &RepoPathBuf, coverage: &[RepoPathBuf]) -> bool {
    if crate::core::ecosystem::registry::is_path_inside_any(path, coverage) {
        return true;
    }
    let path_str = path.escaped();
    let path_prefix = format!("{path_str}/");
    coverage.iter().any(|c| {
        let c_str = c.escaped();
        c_str.starts_with(&*path_prefix) || *c_str == *path_str
    })
}

// ---------------------------------------------------------------------------
// F.1 — Hexagonal cargo
// ---------------------------------------------------------------------------

fn hexagonal_cargo(workdir: &Path) -> Vec<DetectorMatch> {
    let mut out = Vec::new();
    let crates_dirs = find_dirs_with_subdir_pattern(workdir, "crates");
    for crates_dir in crates_dirs {
        // crates_dir is .../D/crates. The service "D" is its parent.
        let service_dir = match crates_dir.parent() {
            Some(p) => p,
            None => continue,
        };
        // Need ≥2 children with Cargo.toml.
        let cargo_subs = list_subdirs_with_file(&crates_dir, "Cargo.toml");
        if cargo_subs.len() < 2 {
            continue;
        }
        // Match condition: at least one of {bin, lib, workers,
        // basename(D)} has [package].
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
            kind: DetectorKind::HexagonalCargo { primary },
            path: repopath,
            note: Some(format!(
                "primary crate: {}",
                primary_label(primary, basename)
            )),
        });
    }
    out
}

fn primary_label(p: HexagonalPrimary, basename: &str) -> &str {
    match p {
        HexagonalPrimary::Bin => "bin",
        HexagonalPrimary::Lib => "lib",
        HexagonalPrimary::Workers => "workers",
        HexagonalPrimary::BaseName => basename,
    }
}

// ---------------------------------------------------------------------------
// F.2 — Tauri (single-source vs legacy multi-file)
// ---------------------------------------------------------------------------

fn tauri(workdir: &Path) -> Vec<DetectorMatch> {
    let mut out = Vec::new();
    for triplet_root in find_dirs_with_files_set(
        workdir,
        &[
            "package.json",
            "src-tauri/Cargo.toml",
            "src-tauri/tauri.conf.json",
        ],
    ) {
        let conf_path = triplet_root.join("src-tauri/tauri.conf.json");
        let single_source = is_tauri_single_source(&conf_path);
        let repopath = match relative_repopath(workdir, &triplet_root) {
            Some(r) => r,
            None => continue,
        };
        out.push(DetectorMatch {
            kind: DetectorKind::Tauri { single_source },
            path: repopath,
            note: Some(if single_source {
                "single-source (version derived from package.json)".to_string()
            } else {
                "legacy multi-file (3 files hand-managed)".to_string()
            }),
        });
    }
    out
}

/// Compiled once per process. The detector now runs at the start of
/// every wizard launch AND every `belaf prepare` (drift check) — paying
/// the regex compile cost on every call would be unnecessary overhead.
static TAURI_PATH_REF_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r#""version"\s*:\s*"\.\./[^"]+\.json""#).expect("static regex must compile")
});
static TAURI_ANY_VERSION_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r#""version"\s*:\s*"[^"]+""#).expect("static regex must compile")
});

fn is_tauri_single_source(conf_path: &Path) -> bool {
    let content = match std::fs::read_to_string(conf_path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    // Either: no version field at all, OR version refers to a path
    // (Tauri 2.x supports `"version": "../package.json"`).
    if TAURI_PATH_REF_RE.is_match(&content) {
        return true;
    }
    !TAURI_ANY_VERSION_RE.is_match(&content)
}

// ---------------------------------------------------------------------------
// F.3 — JVM library
// ---------------------------------------------------------------------------

fn jvm_library(workdir: &Path) -> Vec<DetectorMatch> {
    let mut out = Vec::new();
    // Scan top-level + sdks/* + libs/* + a couple of generic spots.
    let candidates = collect_jvm_candidates(workdir);
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
            // A gradle file exists but no literal version — likely
            // plugin-managed (ReactiveCircus, semver-gradle, …).
            JvmVersionSource::PluginManaged
        } else {
            continue;
        };

        let repopath = match relative_repopath(workdir, &dir) {
            Some(r) => r,
            None => continue,
        };
        out.push(DetectorMatch {
            kind: DetectorKind::JvmLibrary {
                version_source: kind.clone(),
            },
            path: repopath,
            note: Some(jvm_label(&kind).to_string()),
        });
    }
    out
}

fn jvm_label(s: &JvmVersionSource) -> &'static str {
    match s {
        JvmVersionSource::GradleProperties => "gradle.properties (recommended)",
        JvmVersionSource::BuildGradleKtsLiteral => "literal version in build.gradle(.kts)",
        JvmVersionSource::PluginManaged => "plugin-managed (suggest external_versioner)",
    }
}

fn collect_jvm_candidates(workdir: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    // sdks/*
    if let Ok(entries) = std::fs::read_dir(workdir.join("sdks")) {
        for e in entries.flatten() {
            if e.path().is_dir() {
                dirs.push(e.path());
            }
        }
    }
    // libs/*
    if let Ok(entries) = std::fs::read_dir(workdir.join("libs")) {
        for e in entries.flatten() {
            if e.path().is_dir() {
                dirs.push(e.path());
            }
        }
    }
    // top-level (single-project case)
    if workdir.join("gradle.properties").exists() || workdir.join("build.gradle.kts").exists() {
        dirs.push(workdir.to_path_buf());
    }
    dirs
}

// ---------------------------------------------------------------------------
// F.4 — Mobile app (warning only)
// ---------------------------------------------------------------------------

fn mobile_app(workdir: &Path) -> Vec<DetectorMatch> {
    let mut out = Vec::new();
    // iOS: any *.xcodeproj/project.pbxproj
    for ios_dir in find_xcodeproj_parents(workdir) {
        let repopath = match relative_repopath(workdir, &ios_dir) {
            Some(r) => r,
            None => continue,
        };
        out.push(DetectorMatch {
            kind: DetectorKind::MobileApp {
                platform: MobilePlatform::Ios,
            },
            path: repopath,
            note: Some("iOS app (recommend Bitrise/fastlane)".to_string()),
        });
    }
    // Android: build.gradle{.kts}? with versionName + versionCode
    for android_dir in find_android_app_dirs(workdir) {
        let repopath = match relative_repopath(workdir, &android_dir) {
            Some(r) => r,
            None => continue,
        };
        out.push(DetectorMatch {
            kind: DetectorKind::MobileApp {
                platform: MobilePlatform::Android,
            },
            path: repopath,
            note: Some("Android app (recommend Bitrise/Codemagic)".to_string()),
        });
    }
    out
}

fn find_xcodeproj_parents(workdir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_capped(workdir, 6, |p| {
        if p.is_dir() && p.extension().and_then(|s| s.to_str()) == Some("xcodeproj") {
            let pbx = p.join("project.pbxproj");
            if pbx.exists() {
                if let Some(parent) = p.parent() {
                    out.push(parent.to_path_buf());
                }
            }
        }
    });
    out
}

fn find_android_app_dirs(workdir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_capped(workdir, 5, |p| {
        let bgk = p.join("build.gradle.kts");
        let bg = p.join("build.gradle");
        for f in [&bgk, &bg] {
            if f.exists()
                && file_contains_pattern(f, r"versionName\s*=")
                && file_contains_pattern(f, r"versionCode\s*=")
            {
                out.push(p.to_path_buf());
                break;
            }
        }
    });
    out
}

// ---------------------------------------------------------------------------
// F.6 — Nested npm workspace
// ---------------------------------------------------------------------------

fn nested_npm_workspace(workdir: &Path) -> Vec<DetectorMatch> {
    let mut out = Vec::new();
    walk_capped(workdir, 5, |p| {
        if p == workdir {
            return;
        }
        let pkg = p.join("package.json");
        if pkg.exists() && file_contains_pattern(&pkg, r#""workspaces"\s*:"#) {
            if let Some(repopath) = relative_repopath(workdir, p) {
                out.push(DetectorMatch {
                    kind: DetectorKind::NestedNpmWorkspace,
                    path: repopath,
                    note: None,
                });
            }
        }
    });
    out
}

// ---------------------------------------------------------------------------
// F.7 — SDK cascade members
// ---------------------------------------------------------------------------

fn sdk_cascade_members(workdir: &Path) -> Vec<DetectorMatch> {
    let mut out = Vec::new();
    let sdks = workdir.join("sdks");
    if !sdks.is_dir() {
        return out;
    }
    if let Ok(entries) = std::fs::read_dir(&sdks) {
        for e in entries.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            // SDK indicators: codegen config files, generated SDK
            // sentinels.
            let indicators = [
                "graphql-codegen.yml",
                "graphql-codegen.yaml",
                "orval.config.ts",
                "orval.config.js",
                "apollo.gradle.kts",
                "swift-codegen.yml",
                "openapi-generator.yaml",
                "openapi-generator.yml",
            ];
            let has_indicator = indicators.iter().any(|f| p.join(f).exists());
            // OR: this is a sub-directory under sdks/ with a
            // package.json / Cargo.toml / Package.swift /
            // gradle.properties — treat as a generated package by
            // default (cascade detection is conservative; user
            // declines if false-positive).
            let has_package = p.join("package.json").exists()
                || p.join("Cargo.toml").exists()
                || p.join("Package.swift").exists()
                || p.join("gradle.properties").exists();

            if has_indicator || has_package {
                if let Some(repopath) = relative_repopath(workdir, &p) {
                    out.push(DetectorMatch {
                        kind: DetectorKind::SdkCascadeMember,
                        path: repopath,
                        note: None,
                    });
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn workdir(repo: &Repository) -> Option<PathBuf> {
    let p = repo.resolve_workdir(&RepoPathBuf::new(b""));
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

fn relative_repopath(workdir: &Path, abs: &Path) -> Option<RepoPathBuf> {
    let rel = abs.strip_prefix(workdir).ok()?;
    let s = rel.to_string_lossy().to_string();
    if s.is_empty() {
        return None;
    }
    Some(RepoPathBuf::new(s.as_bytes()))
}

/// Walk under `workdir` up to `max_depth` levels deep, applying `f`
/// to each entry. Skips common heavy directories so repo-wide scans
/// stay fast.
fn walk_capped<F: FnMut(&Path)>(workdir: &Path, max_depth: usize, mut f: F) {
    fn skip_dir(name: &str) -> bool {
        matches!(
            name,
            "node_modules"
                | "target"
                | ".git"
                | ".idea"
                | ".vscode"
                | "build"
                | "dist"
                | ".next"
                | "out"
                | "vendor"
                | "third_party"
                | "Pods"
                | "DerivedData"
        )
    }

    fn rec<F: FnMut(&Path)>(p: &Path, depth_left: usize, f: &mut F) {
        if depth_left == 0 {
            return;
        }
        f(p);
        let entries = match std::fs::read_dir(p) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if skip_dir(name) {
                        continue;
                    }
                }
                rec(&path, depth_left - 1, f);
            }
        }
    }

    rec(workdir, max_depth, &mut f);
}

/// Find every directory `D` under `workdir` (capped depth) where
/// `D/<name>` exists and is a directory.
fn find_dirs_with_subdir_pattern(workdir: &Path, name: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_capped(workdir, 5, |p| {
        let candidate = p.join(name);
        if candidate.is_dir() {
            out.push(candidate);
        }
    });
    out
}

/// Find every directory `D` under `workdir` (capped depth) where
/// every relative file path in `files` exists under `D`.
fn find_dirs_with_files_set(workdir: &Path, files: &[&str]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_capped(workdir, 5, |p| {
        if files.iter().all(|f| p.join(f).exists()) {
            out.push(p.to_path_buf());
        }
    });
    out
}

/// Subdirectories of `dir` that contain `file_name` directly inside.
fn list_subdirs_with_file(dir: &Path, file_name: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join(file_name).exists() {
                out.push(path);
            }
        }
    }
    out
}

fn cargo_toml_has_package_section(path: &Path) -> bool {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let doc: toml_edit::DocumentMut = match content.parse() {
        Ok(d) => d,
        Err(_) => return false,
    };
    doc.get("package").and_then(|p| p.as_table()).is_some()
}

fn file_contains_line(path: &Path, prefix: &str) -> bool {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    content.lines().any(|l| l.trim_start().starts_with(prefix))
}

fn file_contains_pattern(path: &Path, pattern: &str) -> bool {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let re = match regex::Regex::new(pattern) {
        Ok(r) => r,
        Err(_) => return false,
    };
    re.is_match(&content)
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
    fn hexagonal_cargo_detects_bin_primary() {
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
        let matches = hexagonal_cargo(root);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].path.escaped(), "apps/services/aura");
        match &matches[0].kind {
            DetectorKind::HexagonalCargo { primary } => {
                assert_eq!(*primary, HexagonalPrimary::Bin);
            }
            _ => panic!("expected HexagonalCargo"),
        }
    }

    #[test]
    fn hexagonal_cargo_detects_workers_fallback() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        // mondo: no bin, only workers + core.
        write(
            &root.join("apps/services/mondo/crates/workers/Cargo.toml"),
            "[package]\nname = \"mondo-workers\"\nversion = \"0.1.0\"\n",
        );
        write(
            &root.join("apps/services/mondo/crates/core/Cargo.toml"),
            "[package]\nname = \"mondo-core\"\nversion = \"0.1.0\"\n",
        );
        let matches = hexagonal_cargo(root);
        assert_eq!(matches.len(), 1);
        match &matches[0].kind {
            DetectorKind::HexagonalCargo { primary } => {
                assert_eq!(*primary, HexagonalPrimary::Workers);
            }
            _ => panic!("expected HexagonalCargo"),
        }
    }

    #[test]
    fn hexagonal_cargo_skips_when_only_one_crate_subdir() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("apps/foo/crates/bin/Cargo.toml"),
            "[package]\nname = \"foo\"\nversion = \"0.1.0\"\n",
        );
        let matches = hexagonal_cargo(root);
        // Need ≥2 children with Cargo.toml; only one here.
        assert!(matches.is_empty());
    }

    #[test]
    fn tauri_single_source_via_path_ref() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("apps/desktop/package.json"),
            r#"{"version":"0.1.0"}"#,
        );
        write(
            &root.join("apps/desktop/src-tauri/Cargo.toml"),
            "[package]\nname = \"desktop\"\nversion = \"0.0.0\"\n",
        );
        write(
            &root.join("apps/desktop/src-tauri/tauri.conf.json"),
            r#"{"productName":"desktop","version":"../package.json"}"#,
        );
        let matches = tauri(root);
        assert_eq!(matches.len(), 1);
        match &matches[0].kind {
            DetectorKind::Tauri { single_source } => assert!(*single_source),
            _ => panic!("expected Tauri"),
        }
    }

    #[test]
    fn tauri_legacy_multi_file() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("apps/desktop/package.json"),
            r#"{"version":"0.1.0"}"#,
        );
        write(
            &root.join("apps/desktop/src-tauri/Cargo.toml"),
            "[package]\nname = \"desktop\"\nversion = \"0.1.0\"\n",
        );
        write(
            &root.join("apps/desktop/src-tauri/tauri.conf.json"),
            r#"{"productName":"desktop","version":"0.1.0"}"#,
        );
        let matches = tauri(root);
        assert_eq!(matches.len(), 1);
        match &matches[0].kind {
            DetectorKind::Tauri { single_source } => assert!(!*single_source),
            _ => panic!("expected Tauri"),
        }
    }

    #[test]
    fn jvm_library_gradle_properties() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("sdks/kotlin/gradle.properties"),
            "version=0.1.0\n",
        );
        let matches = jvm_library(root);
        assert_eq!(matches.len(), 1);
        match &matches[0].kind {
            DetectorKind::JvmLibrary { version_source } => {
                assert_eq!(*version_source, JvmVersionSource::GradleProperties);
            }
            _ => panic!("expected JvmLibrary"),
        }
    }

    #[test]
    fn jvm_library_build_gradle_kts_literal() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("sdks/java/build.gradle.kts"),
            "plugins {}\nversion = \"0.1.0\"\n",
        );
        let matches = jvm_library(root);
        assert_eq!(matches.len(), 1);
        match &matches[0].kind {
            DetectorKind::JvmLibrary { version_source } => {
                assert_eq!(*version_source, JvmVersionSource::BuildGradleKtsLiteral);
            }
            _ => panic!("expected JvmLibrary"),
        }
    }

    #[test]
    fn jvm_library_plugin_managed() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("sdks/javakt/build.gradle.kts"),
            "plugins { id(\"io.github.reactivecircus.app-versioning\") version \"1.3.1\" }\n",
        );
        let matches = jvm_library(root);
        assert_eq!(matches.len(), 1);
        match &matches[0].kind {
            DetectorKind::JvmLibrary { version_source } => {
                assert_eq!(*version_source, JvmVersionSource::PluginManaged);
            }
            _ => panic!("expected JvmLibrary"),
        }
    }

    #[test]
    fn mobile_app_ios_detected() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("apps/clients/ios/MyApp.xcodeproj/project.pbxproj"),
            "// not really a pbxproj — just needs to exist for detection\n",
        );
        let matches = mobile_app(root);
        let ios: Vec<_> = matches
            .iter()
            .filter(|m| {
                matches!(
                    m.kind,
                    DetectorKind::MobileApp {
                        platform: MobilePlatform::Ios
                    }
                )
            })
            .collect();
        assert_eq!(ios.len(), 1);
        assert_eq!(ios[0].path.escaped(), "apps/clients/ios");
    }

    #[test]
    fn mobile_app_android_detected_via_dual_version() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("apps/clients/android/app/build.gradle.kts"),
            "android {\n  defaultConfig {\n    versionName = \"1.0\"\n    versionCode = 1\n  }\n}\n",
        );
        let matches = mobile_app(root);
        let android: Vec<_> = matches
            .iter()
            .filter(|m| {
                matches!(
                    m.kind,
                    DetectorKind::MobileApp {
                        platform: MobilePlatform::Android
                    }
                )
            })
            .collect();
        assert_eq!(android.len(), 1);
    }

    #[test]
    fn nested_npm_workspace_detected() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        // Repo-root package.json — should NOT match (it's the root)
        write(&root.join("package.json"), r#"{"name":"root"}"#);
        // Nested workspace
        write(
            &root.join("apps/dashboards/docs/package.json"),
            r#"{"name":"docs","workspaces":["packages/*"]}"#,
        );
        let matches = nested_npm_workspace(root);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].path.escaped(), "apps/dashboards/docs");
    }

    #[test]
    fn sdk_cascade_members_via_codegen_indicator() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("sdks/typescript/graphql-codegen.yml"),
            "schema: ../../proto",
        );
        write(
            &root.join("sdks/typescript/package.json"),
            r#"{"version":"0.1.0"}"#,
        );
        let matches = sdk_cascade_members(root);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].path.escaped(), "sdks/typescript");
    }

    #[test]
    fn detection_report_helpers() {
        let mobile_only = DetectionReport {
            matches: vec![DetectorMatch {
                kind: DetectorKind::MobileApp {
                    platform: MobilePlatform::Ios,
                },
                path: RepoPathBuf::new(b"apps/ios"),
                note: None,
            }],
        };
        assert!(mobile_only.is_single_mobile_repo());
        assert_eq!(mobile_only.count_release_unit_candidates(), 0);

        let mixed = DetectionReport {
            matches: vec![
                DetectorMatch {
                    kind: DetectorKind::HexagonalCargo {
                        primary: HexagonalPrimary::Bin,
                    },
                    path: RepoPathBuf::new(b"apps/services/aura"),
                    note: None,
                },
                DetectorMatch {
                    kind: DetectorKind::MobileApp {
                        platform: MobilePlatform::Ios,
                    },
                    path: RepoPathBuf::new(b"apps/ios"),
                    note: None,
                },
            ],
        };
        assert!(!mixed.is_single_mobile_repo());
        assert_eq!(mixed.count_release_unit_candidates(), 1);
    }

    #[test]
    fn empty_report_is_not_single_mobile() {
        let empty = DetectionReport::default();
        assert!(!empty.is_single_mobile_repo());
    }

    #[test]
    fn drift_format_error_actionable() {
        let r = DriftReport {
            uncovered: vec![
                UncoveredHit {
                    path: RepoPathBuf::new(b"apps/services/foobar"),
                    kind: DetectorKind::HexagonalCargo {
                        primary: HexagonalPrimary::Bin,
                    },
                },
                UncoveredHit {
                    path: RepoPathBuf::new(b"sdks/python"),
                    kind: DetectorKind::JvmLibrary {
                        version_source: JvmVersionSource::GradleProperties,
                    },
                },
            ],
        };
        let msg = r.format_error();
        assert!(msg.contains("uncovered release artifacts"));
        assert!(msg.contains("apps/services/foobar"));
        assert!(msg.contains("sdks/python"));
        assert!(msg.contains("belaf init --ci --auto-detect"));
        assert!(msg.contains("[ignore_paths]"));
        assert!(msg.contains("[allow_uncovered]"));
        assert!(msg.contains("Aborting prepare"));
    }

    #[test]
    fn drift_empty_report_is_empty() {
        assert!(DriftReport::default().is_empty());
    }
}
