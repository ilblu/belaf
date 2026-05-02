//! Shared state across all wizard steps.
//!
//! Phase I refactor — fields that the previous monolith stored on
//! `WizardState` and that more than one step reads/writes live here.
//! Step-private state (cursor positions, scroll offsets, sub-toggles)
//! lives on each step's struct in `welcome.rs` / `preset.rs` /
//! `project.rs` / `upstream.rs` / `confirmation.rs`.

use std::collections::HashSet;

use crate::core::git::repository::RepoPathBuf;
use crate::core::release_unit::detector::DetectionReport;

#[derive(Clone, Debug)]
pub struct DetectedUnit {
    pub name: String,
    pub version: String,
    pub prefix: String,
    pub selected: bool,
    /// Ecosystem identifier (`cargo`, `npm`, `pypa`, …) sourced from
    /// the loader. Used by the wizard to pick a per-row icon when
    /// `BELAF_ICONS=nerd` is set.
    pub ecosystem: Option<String>,
}

#[derive(Debug)]
pub struct WizardState {
    pub standalone_units: Vec<DetectedUnit>,
    pub upstream_url: String,
    /// Transient error / validation feedback. Each step owns when to
    /// set this and when to clear it. Re-used across steps because the
    /// previous monolith already did and the existing tests pin the
    /// behaviour (welcome shows the dirty-repo error, then ENTER
    /// clears it before pushing into the preset step).
    pub error_message: Option<String>,
    pub force: bool,
    pub dirty_warning: Option<String>,
    pub preset: Option<String>,
    pub preset_from_cli: bool,
    pub available_presets: Vec<String>,
    pub config_exists: bool,

    /// Phase I.1 — auto-detected bundles. Populated once at the start
    /// of the wizard run from `release_unit::detector::detect_all`.
    /// Empty in tests / repos with nothing matching the heuristics.
    pub detection: DetectionReport,

    /// Phase I.1 — set to `true` by [`DetectorReviewStep`](super::detector_review::DetectorReviewStep)
    /// when the user accepts the detected bundles. The orchestrator
    /// reads this after a successful bootstrap and appends the
    /// auto_detect snippet to `belaf/config.toml`.
    pub detector_accepted: bool,

    /// Per-item exclusions chosen by the user in
    /// [`DetectorReviewStep`](super::detector_review::DetectorReviewStep).
    /// Each entry is a detector-match path the user toggled OFF;
    /// the orchestrator passes this to
    /// [`auto_detect::run_filtered`](crate::cmd::init::auto_detect::run_filtered)
    /// so excluded paths get no `[release_unit.<name>]` block AND land in
    /// `[ignore_paths]` (silences drift on subsequent prepares).
    pub detector_excluded: HashSet<RepoPathBuf>,

    /// Phase I.3 — tag-format override picked by the user via
    /// [`TagFormatStep`](super::tag_format::TagFormatStep) when the
    /// repo is a single-project bundle. `None` means "fall back to
    /// each ecosystem's `tag_format_default`". `Some` is appended as
    /// a `[projects."<name>"]` block to `belaf/config.toml` after
    /// bootstrap.
    pub tag_format_override: Option<String>,
}

impl WizardState {
    pub fn new(force: bool, preset: Option<String>) -> Self {
        use crate::core::embed::EmbeddedPresets;

        let preset_from_cli = preset.is_some();
        let mut available_presets = vec!["default".to_string()];
        available_presets.extend(EmbeddedPresets::list_presets());

        Self {
            standalone_units: Vec::new(),
            upstream_url: String::new(),
            error_message: None,
            force,
            dirty_warning: None,
            preset,
            preset_from_cli,
            available_presets,
            config_exists: false,
            detection: DetectionReport::default(),
            detector_accepted: false,
            detector_excluded: HashSet::new(),
            tag_format_override: None,
        }
    }

    pub fn selected_units(&self) -> Vec<&DetectedUnit> {
        self.standalone_units
            .iter()
            .filter(|p| p.selected)
            .collect()
    }
}
