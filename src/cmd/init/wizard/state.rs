//! Shared state across all wizard steps.
//!
//! Phase I refactor — fields that the previous monolith stored on
//! `WizardState` and that more than one step reads/writes live here.
//! Step-private state (cursor positions, scroll offsets, sub-toggles)
//! lives on each step's struct in `welcome.rs` / `preset.rs` /
//! `project.rs` / `upstream.rs` / `confirmation.rs`.

#[derive(Clone, Debug)]
pub struct DetectedProject {
    pub name: String,
    pub version: String,
    pub prefix: String,
    pub selected: bool,
}

pub struct WizardState {
    pub projects: Vec<DetectedProject>,
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
}

impl WizardState {
    pub fn new(force: bool, preset: Option<String>) -> Self {
        use crate::core::embed::EmbeddedPresets;

        let preset_from_cli = preset.is_some();
        let mut available_presets = vec!["default".to_string()];
        available_presets.extend(EmbeddedPresets::list_presets());

        Self {
            projects: Vec::new(),
            upstream_url: String::new(),
            error_message: None,
            force,
            dirty_warning: None,
            preset,
            preset_from_cli,
            available_presets,
            config_exists: false,
        }
    }

    pub fn selected_projects(&self) -> Vec<&DetectedProject> {
        self.projects.iter().filter(|p| p.selected).collect()
    }
}
