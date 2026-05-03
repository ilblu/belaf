//! Shared state across all wizard steps.
//!
//! Phase I refactor — fields that the previous monolith stored on
//! `WizardState` and that more than one step reads/writes live here.
//! Step-private state (cursor positions, scroll offsets, sub-toggles)
//! lives on each step's struct in `welcome.rs` / `preset.rs` /
//! `project.rs` / `upstream.rs` / `confirmation.rs`.

use std::collections::{HashMap, HashSet};

use crate::core::git::repository::RepoPathBuf;
use crate::core::release_unit::detector::DetectionReport;

/// User-chosen cascade rule for one Standalone unit, picked
/// interactively via the `[c]` keybinding in
/// [`UnifiedSelectionStep`](super::unified_selection::UnifiedSelectionStep).
/// Persisted into the emitted `[release_unit.<name>]` block as
/// `cascade_from = { source = "<source>", bump = "<strategy>" }`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CascadeOverride {
    pub source: String,
    pub strategy: CascadeStrategy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CascadeStrategy {
    Mirror,
    FloorPatch,
    FloorMinor,
    FloorMajor,
}

impl CascadeStrategy {
    pub fn as_wire(&self) -> &'static str {
        match self {
            Self::Mirror => "mirror",
            Self::FloorPatch => "floor_patch",
            Self::FloorMinor => "floor_minor",
            Self::FloorMajor => "floor_major",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Mirror => "mirror — exact same version as source",
            Self::FloorPatch => "floor_patch — at least patch when source bumps",
            Self::FloorMinor => "floor_minor — at least minor when source bumps",
            Self::FloorMajor => "floor_major — at least major when source bumps",
        }
    }
}

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

    /// Per-unit cascade rules chosen interactively via the `[c]`
    /// keybinding in [`UnifiedSelectionStep`]. Keyed by the
    /// standalone-unit's display name (matches `DetectedUnit::name`).
    /// Appended to the emitted `[release_unit.<name>]` block as a
    /// `cascade_from = ...` field.
    pub cascade_overrides: HashMap<String, CascadeOverride>,
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
            cascade_overrides: HashMap::new(),
        }
    }

    pub fn selected_units(&self) -> Vec<&DetectedUnit> {
        self.standalone_units
            .iter()
            .filter(|p| p.selected)
            .collect()
    }
}
