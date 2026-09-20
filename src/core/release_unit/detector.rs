//! Auto-detectors that scan a repository and propose ReleaseUnit
//! configuration in the wizard. Plus the always-on drift detector
//! that fires at `belaf prepare` time.
//!
//! Each scanner is a pure filesystem walk; aggregation happens in
//! [`detect_all`] and drift coverage is computed in
//! [`detect_drift_from_report`]. Per-detector logic lives in
//! `scanners`; shared filesystem helpers live in `walk`.
//!
//! Classification — Bundle vs Hint vs ExternallyManaged — lives in the
//! [`super::shape`] module. Consumers of [`DetectionReport`] should
//! `match` on `m.shape` exhaustively, never on the leaf enum variants.

use std::path::Path;

use crate::core::config::ConfigurationFile;
use crate::core::git::repository::{RepoPathBuf, Repository};

pub use super::shape::{
    BundleKind, DecisionKind, DetectedShape, DetectorMatch, ExtKind, GradleBuildFile,
    HexagonalPrimary, HintKind, JvmUnwritableReason, JvmVersionSource, SingleProjectEcosystem,
};

use super::ResolvedReleaseUnit;

mod scanners;
use super::walk;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Aggregate report covering every detector hit in the repo.
#[derive(Debug, Default)]
pub struct DetectionReport {
    pub matches: Vec<DetectorMatch>,
}

impl DetectionReport {
    pub fn matches_of(
        &self,
        shape_predicate: impl Fn(&DetectedShape) -> bool,
    ) -> Vec<&DetectorMatch> {
        self.matches
            .iter()
            .filter(|m| shape_predicate(&m.shape))
            .collect()
    }

    /// True when every detector hit is an externally-managed mobile
    /// app — used by the wizard to short-circuit into the
    /// `cmd::init::wizard::single_mobile` flow (private module).
    pub fn is_single_mobile_repo(&self) -> bool {
        !self.matches.is_empty()
            && self
                .matches
                .iter()
                .all(|m| matches!(m.shape, DetectedShape::ExternallyManaged(_)))
    }

    /// Bundles + Hints (which decorate Standalones) count toward the
    /// release-unit candidate count; ExternallyManaged + bare-repo
    /// hints (`SingleProject`, `NestedMonorepo`) do not.
    pub fn count_release_unit_candidates(&self) -> usize {
        self.matches
            .iter()
            .filter(|m| match &m.shape {
                DetectedShape::Bundle(_) => true,
                DetectedShape::Hint(HintKind::SdkCascade | HintKind::NpmWorkspace) => true,
                DetectedShape::Hint(HintKind::SingleProject { .. } | HintKind::NestedMonorepo) => {
                    false
                }
                DetectedShape::ExternallyManaged(_) => false,
                // Not a candidate: nothing is emitted for it, so counting
                // it would promise the user a unit that init will not
                // write. It surfaces through the drift report instead.
                DetectedShape::NeedsDecision(_) => false,
            })
            .count()
    }
}

/// One detector hit that isn't covered by any resolved ReleaseUnit,
/// `[ignore_paths]`, or `[allow_uncovered]` — listed in the drift
/// error so the user can remediate it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UncoveredHit {
    pub path: RepoPathBuf,
    pub shape: DetectedShape,
    /// The detector's own explanation for this hit, when it had one.
    /// Carried through so the drift error can print the specific
    /// remediation instead of the generic menu — for a
    /// [`DetectedShape::NeedsDecision`] hit the generic menu's "add it
    /// to `[allow_uncovered]`" is precisely the wrong advice.
    pub note: Option<String>,
}

#[derive(Debug, Default)]
pub struct DriftReport {
    pub uncovered: Vec<UncoveredHit>,
}

impl DriftReport {
    pub fn is_empty(&self) -> bool {
        self.uncovered.is_empty()
    }

    pub fn format_error(&self) -> String {
        let (needs_decision, mut classifiable): (Vec<&UncoveredHit>, Vec<&UncoveredHit>) = self
            .uncovered
            .iter()
            .partition(|h| h.shape.is_needs_decision());

        // One path, one instruction. A directory can raise several hits
        // — `sdks/kotlin` is both an SDK-cascade hint and an
        // unversionable JVM project — and listing it in both sections
        // would print the generic menu, `[allow_uncovered]` and all,
        // right above the specific fix that says not to do that. The
        // decision wins: the other hits describe a unit that does not
        // exist until it is resolved.
        classifiable.retain(|h| !needs_decision.iter().any(|d| d.path == h.path));

        let mut s = String::new();
        s.push_str(
            "uncovered release artifacts: detector hits that are not part of any ReleaseUnit, ignore_paths, or allow_uncovered.\n\n",
        );

        if !classifiable.is_empty() {
            s.push_str("The following paths match known detector patterns but are not part of any ReleaseUnit:\n");
            for h in &classifiable {
                s.push_str(&format!(
                    "  - {:50} ({})\n",
                    h.path.escaped(),
                    drift_shape_label(&h.shape),
                ));
            }
            s.push_str(
                "\nChoose one:\n  → run `belaf init --ci --auto-detect --force` to re-detect bundles and append release_unit blocks to belaf/config.toml (idempotent — paths already covered by the config are never re-emitted)\n  → add explicit [release_unit.<name>] entries\n  → if intentional (mobile app, archive, etc.), add to [ignore_paths] or [allow_uncovered]\n",
            );
        }

        // Reported separately, and never under the menu above: these
        // are hits the detector recognised but could not classify, so
        // "run --auto-detect again" would produce the same non-answer,
        // and "add it to [allow_uncovered]" would bury a real release
        // artifact. Each carries the specific fix from the detector
        // that raised it.
        if !needs_decision.is_empty() {
            if !classifiable.is_empty() {
                s.push('\n');
            }
            s.push_str(
                "The following paths are release artifacts belaf recognises but cannot version on its own. Re-running auto-detect will not resolve them — each needs one decision:\n",
            );
            for h in &needs_decision {
                s.push_str(&format!(
                    "  - {:50} ({})\n",
                    h.path.escaped(),
                    drift_shape_label(&h.shape),
                ));
                if let Some(note) = &h.note {
                    s.push_str(&format!("      → {note}\n"));
                }
            }
        }

        s.push_str("\nAborting prepare. No releases will be drafted.");
        s
    }
}

impl DriftReport {
    /// The uncovered paths, each listed once, in first-seen order.
    ///
    /// One directory can raise several hits — `sdks/kotlin` is both an
    /// SDK-cascade hint and an unversionable JVM project — so the raw
    /// `uncovered` vec can name the same path more than once. This is
    /// what `belaf prepare` POSTs to the dashboard's Drift tab, where
    /// one problem shown as two rows reads as two problems.
    pub fn unique_paths(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::with_capacity(self.uncovered.len());
        for h in &self.uncovered {
            let p = h.path.escaped().to_string();
            if !out.contains(&p) {
                out.push(p);
            }
        }
        out
    }
}

fn drift_shape_label(s: &DetectedShape) -> String {
    match s {
        DetectedShape::Bundle(b) => match b {
            BundleKind::HexagonalCargo { primary } => {
                format!("hexagonal cargo: crates/{primary}/Cargo.toml present")
            }
            BundleKind::Tauri {
                single_source,
                shared_workspace,
            } => format!(
                "tauri triplet ({}{})",
                if *single_source {
                    "single-source"
                } else {
                    "legacy multi-file"
                },
                match shared_workspace {
                    Some(ws) if ws.is_empty() => ", shared workspace at repo root",
                    Some(_) => ", shared cargo workspace",
                    None => "",
                }
            ),
            BundleKind::JvmLibrary { version_source } => {
                format!("jvm library ({})", jvm_label(version_source))
            }
        },
        DetectedShape::Hint(h) => match h {
            HintKind::NpmWorkspace => "nested npm workspace".to_string(),
            HintKind::SdkCascade => "generated SDK package under sdks/*".to_string(),
            HintKind::SingleProject { ecosystem } => {
                format!("single-project repo ({ecosystem})")
            }
            HintKind::NestedMonorepo => "nested submodule with its own monorepo".to_string(),
        },
        DetectedShape::ExternallyManaged(e) => match e {
            ExtKind::MobileIos => "iOS app — recommend Bitrise/fastlane".to_string(),
            ExtKind::MobileAndroid => "Android app — recommend Bitrise/Codemagic".to_string(),
            ExtKind::JvmPluginManaged => {
                "JVM (plugin-managed) — release flow owned by Gradle plugin".to_string()
            }
        },
        DetectedShape::NeedsDecision(d) => match d {
            DecisionKind::JvmVersionUnwritable { build_file, .. } => {
                format!("JVM library — no version belaf can write ({build_file})")
            }
        },
    }
}

/// Names the file the version is read from. Deliberately not the
/// wizard's phrasing ("gradle.properties (recommended)"): the drift
/// report states what was found, and nesting a recommendation inside
/// the label's own parentheses reads as
/// `jvm library (gradle.properties (recommended))`.
fn jvm_label(s: &JvmVersionSource) -> String {
    match s {
        JvmVersionSource::GradleProperties => "gradle.properties".to_string(),
        JvmVersionSource::BuildScriptLiteral { build_file } => build_file.filename().to_string(),
    }
}

/// Convenience wrapper that runs [`detect_drift`] and returns an error
/// with the formatted message when uncovered hits exist. Callers (the
/// prepare command) `?`-propagate.
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
/// directly. Equivalent in result; saves the caller from carrying a
/// full [`ConfigurationFile`] around.
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
/// want to avoid re-walking the filesystem.
pub fn detect_drift_from_report(
    report: &DetectionReport,
    resolved: &[ResolvedReleaseUnit],
    ignore_paths: &[String],
    allow_uncovered: &[String],
) -> DriftReport {
    let mut coverage = unit_coverage_paths(resolved);
    for p in ignore_paths {
        coverage.push(RepoPathBuf::new(p.trim_end_matches('/').as_bytes()));
    }
    for p in allow_uncovered {
        coverage.push(RepoPathBuf::new(p.trim_end_matches('/').as_bytes()));
    }

    let uncovered: Vec<UncoveredHit> = report
        .matches
        .iter()
        .filter(|m| is_drift_signal(&m.shape) && !is_covered(&m.path, &coverage))
        .map(|m| UncoveredHit {
            path: m.path.clone(),
            shape: m.shape.clone(),
            note: m.note.clone(),
        })
        .collect();

    DriftReport { uncovered }
}

/// Directory prefixes claimed by resolved release units: manifest
/// parent dirs, `paths = [...]` entries of manifest-less units, and
/// satellites. Shared between the drift check and init-time re-detect
/// suppression so both agree on what a `[release_unit.<name>]` block
/// covers.
pub fn unit_coverage_paths(resolved: &[ResolvedReleaseUnit]) -> Vec<RepoPathBuf> {
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
        // F1 — a manifest-less `paths = [...]` unit (internal/ignore) explicitly
        // declares the directories it owns; they count as covered so the drift
        // detector doesn't flag e.g. `proto/` or `apps/services/e2e`.
        if let super::VersionSource::PathsOnly(paths) = &r.unit.source {
            for p in paths {
                coverage.push(p.clone());
            }
        }
        for s in &r.unit.satellites {
            coverage.push(s.clone());
        }
    }
    coverage
}

/// Whether a detector hit should ever surface as a drift error.
/// Repo-shape Hints (`SingleProject`, `NestedMonorepo`) describe the
/// repo as a whole, not a missed bundle; they are wizard-only
/// signals. Everything else (bundles, sdk-cascade hints, npm-workspace
/// hints, externally-managed paths) does signal drift if uncovered.
fn is_drift_signal(shape: &DetectedShape) -> bool {
    !matches!(
        shape,
        DetectedShape::Hint(HintKind::SingleProject { .. } | HintKind::NestedMonorepo)
    )
}

// ---------------------------------------------------------------------------
// Top-level dispatch
// ---------------------------------------------------------------------------

/// Run every init-time detector against the repo's working tree.
/// Used by `belaf init` to seed the TUI list.
pub fn detect_all(repo: &Repository) -> DetectionReport {
    let workdir = match walk::workdir(repo) {
        Some(w) => w,
        None => return DetectionReport::default(),
    };

    let mut matches = Vec::new();
    // Bundles first — higher specificity scanners (hexagonal/tauri/jvm)
    // run before the broader Hint scanners so per-path dedup in the
    // wizard keeps the most useful label.
    matches.extend(super::bundle::detect_all(&workdir));
    // Hints + ExternallyManaged.
    matches.extend(scanners::mobile_app(&workdir));
    matches.extend(scanners::nested_npm_workspace(&workdir));
    matches.extend(scanners::sdk_cascade_members(&workdir));
    matches.extend(scanners::single_project_repo(&workdir));
    matches.extend(scanners::nested_monorepo(&workdir));

    DetectionReport { matches }
}

/// Drift check: find paths matching a detector pattern that are not
/// covered by any resolved ReleaseUnit, `[ignore_paths]`, or
/// `[allow_uncovered]`. Runs on every `belaf prepare`.
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

/// Whether a detector-hit path is covered by a raw config path list
/// (`[ignore_paths]` / `[allow_uncovered]` entries, trailing-slash
/// tolerant). Shares `is_covered` with the drift check so init-time
/// auto-detect suppression and prepare-time drift silence agree on
/// what "covered" means.
pub fn is_covered_by_config_paths(path: &RepoPathBuf, config_paths: &[String]) -> bool {
    let coverage: Vec<RepoPathBuf> = config_paths
        .iter()
        .map(|p| RepoPathBuf::new(p.trim_end_matches('/').as_bytes()))
        .collect();
    is_covered(path, &coverage)
}

/// A detector-hit path is "covered" iff one of:
///
///   - the hit is inside a coverage path (the standard case —
///     coverage is `apps/services/aura`, hit is `apps/services/aura/crates/bin`),
///     OR
///   - the hit is an ancestor of a coverage path (the inverted
///     case — coverage is `apps/services/aura/crates` (the satellite),
///     hit is `apps/services/aura` (the service dir the hexagonal-cargo
///     detector reports). The deeper satellite means the user has
///     definitely claimed the parent service even if no explicit
///     entry names it.
fn is_covered(path: &RepoPathBuf, coverage: &[RepoPathBuf]) -> bool {
    if crate::core::ecosystem::format_handler::is_path_inside_any(path, coverage) {
        return true;
    }
    let path_str = path.escaped();
    let path_prefix = format!("{path_str}/");
    coverage.iter().any(|c| {
        let c_str = c.escaped();
        c_str.starts_with(&*path_prefix) || *c_str == *path_str
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_report_helpers() {
        let mobile_only = DetectionReport {
            matches: vec![DetectorMatch {
                shape: DetectedShape::ExternallyManaged(ExtKind::MobileIos),
                path: RepoPathBuf::new(b"apps/ios"),
                note: None,
            }],
        };
        assert!(mobile_only.is_single_mobile_repo());
        assert_eq!(mobile_only.count_release_unit_candidates(), 0);

        let mixed = DetectionReport {
            matches: vec![
                DetectorMatch {
                    shape: DetectedShape::Bundle(BundleKind::HexagonalCargo {
                        primary: HexagonalPrimary::Bin,
                    }),
                    path: RepoPathBuf::new(b"apps/services/aura"),
                    note: None,
                },
                DetectorMatch {
                    shape: DetectedShape::ExternallyManaged(ExtKind::MobileIos),
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
                    shape: DetectedShape::Bundle(BundleKind::HexagonalCargo {
                        primary: HexagonalPrimary::Bin,
                    }),
                    note: None,
                },
                UncoveredHit {
                    path: RepoPathBuf::new(b"sdks/python"),
                    shape: DetectedShape::Bundle(BundleKind::JvmLibrary {
                        version_source: JvmVersionSource::GradleProperties,
                    }),
                    note: None,
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
    fn drift_needs_decision_carries_its_own_remedy_not_the_generic_menu() {
        let r = DriftReport {
            uncovered: vec![UncoveredHit {
                path: RepoPathBuf::new(b"sdks/kotlin"),
                shape: DetectedShape::NeedsDecision(DecisionKind::JvmVersionUnwritable {
                    build_file: GradleBuildFile::Kts,
                    reason: JvmUnwritableReason::NotAtLineStart { line: 142 },
                }),
                note: Some("move the version to gradle.properties".to_string()),
            }],
        };
        let msg = r.format_error();
        assert!(msg.contains("sdks/kotlin"));
        assert!(msg.contains("move the version to gradle.properties"));
        assert!(msg.contains("Aborting prepare"));
        // The generic menu is what tells a user to reach for
        // [allow_uncovered] — the exact move that drops the artifact.
        // It must not appear for a hit that has its own remedy.
        assert!(
            !msg.contains("[allow_uncovered]"),
            "needs-decision hits must not be offered the allow_uncovered escape: {msg}"
        );
        assert!(
            !msg.contains("belaf init --ci --auto-detect"),
            "re-running auto-detect cannot resolve an unclassifiable hit: {msg}"
        );
    }

    #[test]
    fn drift_mixes_both_sections_when_both_kinds_are_present() {
        let r = DriftReport {
            uncovered: vec![
                UncoveredHit {
                    path: RepoPathBuf::new(b"apps/services/foobar"),
                    shape: DetectedShape::Bundle(BundleKind::HexagonalCargo {
                        primary: HexagonalPrimary::Bin,
                    }),
                    note: None,
                },
                UncoveredHit {
                    path: RepoPathBuf::new(b"sdks/kotlin"),
                    shape: DetectedShape::NeedsDecision(DecisionKind::JvmVersionUnwritable {
                        build_file: GradleBuildFile::Kts,
                        reason: JvmUnwritableReason::Absent,
                    }),
                    note: Some("add version= to gradle.properties".to_string()),
                },
            ],
        };
        let msg = r.format_error();
        assert!(msg.contains("apps/services/foobar"));
        assert!(msg.contains("sdks/kotlin"));
        assert!(msg.contains("[allow_uncovered]"), "{msg}");
        assert!(msg.contains("add version= to gradle.properties"), "{msg}");
        assert!(msg.contains("needs one decision"), "{msg}");
    }

    #[test]
    fn a_path_with_both_hit_kinds_is_listed_once_under_the_decision() {
        // `sdks/kotlin` legitimately raises two hits: an SDK-cascade
        // hint (it sits under sdks/*) and the unversionable-JVM
        // decision. Printing both would put the generic menu — which
        // ends in "add to [allow_uncovered]" — directly above the fix
        // telling the user not to.
        let r = DriftReport {
            uncovered: vec![
                UncoveredHit {
                    path: RepoPathBuf::new(b"sdks/kotlin"),
                    shape: DetectedShape::Hint(HintKind::SdkCascade),
                    note: None,
                },
                UncoveredHit {
                    path: RepoPathBuf::new(b"sdks/kotlin"),
                    shape: DetectedShape::NeedsDecision(DecisionKind::JvmVersionUnwritable {
                        build_file: GradleBuildFile::Kts,
                        reason: JvmUnwritableReason::NotAtLineStart { line: 16 },
                    }),
                    note: Some("move the version to gradle.properties".to_string()),
                },
            ],
        };
        let msg = r.format_error();
        assert_eq!(
            msg.matches("sdks/kotlin").count(),
            1,
            "the path must be listed exactly once: {msg}"
        );
        assert!(!msg.contains("[allow_uncovered]"), "{msg}");
        assert!(
            msg.contains("move the version to gradle.properties"),
            "{msg}"
        );
    }

    #[test]
    fn unique_paths_lists_a_multi_hit_directory_once() {
        let r = DriftReport {
            uncovered: vec![
                UncoveredHit {
                    path: RepoPathBuf::new(b"sdks/kotlin"),
                    shape: DetectedShape::Hint(HintKind::SdkCascade),
                    note: None,
                },
                UncoveredHit {
                    path: RepoPathBuf::new(b"sdks/kotlin"),
                    shape: DetectedShape::NeedsDecision(DecisionKind::JvmVersionUnwritable {
                        build_file: GradleBuildFile::Kts,
                        reason: JvmUnwritableReason::Absent,
                    }),
                    note: None,
                },
                UncoveredHit {
                    path: RepoPathBuf::new(b"apps/services/foo"),
                    shape: DetectedShape::Bundle(BundleKind::HexagonalCargo {
                        primary: HexagonalPrimary::Bin,
                    }),
                    note: None,
                },
            ],
        };
        assert_eq!(
            r.unique_paths(),
            vec!["sdks/kotlin".to_string(), "apps/services/foo".to_string()],
            "first-seen order, no repeats"
        );
    }

    #[test]
    fn needs_decision_is_a_drift_signal_and_not_a_unit_candidate() {
        let shape = DetectedShape::NeedsDecision(DecisionKind::JvmVersionUnwritable {
            build_file: GradleBuildFile::Groovy,
            reason: JvmUnwritableReason::Absent,
        });
        assert!(
            is_drift_signal(&shape),
            "an unclassifiable artifact must keep being reported"
        );
        let report = DetectionReport {
            matches: vec![DetectorMatch {
                shape,
                path: RepoPathBuf::new(b"libs/jvm"),
                note: None,
            }],
        };
        assert_eq!(
            report.count_release_unit_candidates(),
            0,
            "nothing is emitted for it, so it must not be counted as a candidate"
        );
    }

    #[test]
    fn drift_empty_report_is_empty() {
        assert!(DriftReport::default().is_empty());
    }
}
