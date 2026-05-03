//! Init-wizard adapter around [`crate::core::ui::release_unit_view`].

use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::{layout::Rect, Frame};

use crate::core::ui::release_unit_view::{
    CascadeOverrideBadge, PrepareOverlay, ReleaseUnitView, RowIdx, StandaloneEntry,
};

use super::{
    cascade_from::CascadeFromStep,
    preset::PresetSelectionStep,
    state::WizardState,
    step::{Step, StepResult, WizardOutcome},
    tag_format::TagFormatStep,
    upstream::UpstreamConfigStep,
};

mod render;
#[cfg(test)]
mod tests;

#[derive(Default)]
pub struct UnifiedSelectionStep {
    view: ReleaseUnitView,
    overlay: PrepareOverlay,
    cursor: usize,
    initialised: bool,
}

impl UnifiedSelectionStep {
    pub fn new() -> Self {
        Self::default()
    }

    fn rebuild(&mut self, state: &WizardState) {
        let standalones: Vec<StandaloneEntry> = state
            .standalone_units
            .iter()
            .map(|u| StandaloneEntry {
                name: u.name.clone(),
                version: u.version.clone(),
                prefix: u.prefix.clone(),
                selected: u.selected,
                ecosystem: u.ecosystem.clone(),
            })
            .collect();
        self.view = ReleaseUnitView::from_detection(
            &state.detection,
            &standalones,
            &state.detector_excluded,
        );
        self.overlay = PrepareOverlay::default();
        for unit in &self.view.units {
            if let Some(ov) = state.cascade_overrides.get(&unit.name) {
                self.overlay.cascade_overrides.insert(
                    unit.backref,
                    CascadeOverrideBadge {
                        source: ov.source.clone(),
                        strategy_label: ov.strategy.as_wire().to_string(),
                    },
                );
            }
        }
        if self.cursor >= self.view.flat_indices().len() {
            self.cursor = 0;
        }
    }

    fn ensure_initialised(&mut self, state: &WizardState) {
        if !self.initialised {
            self.rebuild(state);
            self.initialised = true;
        }
    }

    fn current_idx(&self) -> Option<RowIdx> {
        self.view.flat_indices().get(self.cursor).copied()
    }

    fn flush_to_state(&self, state: &mut WizardState) {
        state.detector_accepted = !state.detection.matches.is_empty();
        state.detector_excluded.clear();
        for path in self.view.excluded_bundle_paths() {
            state.detector_excluded.insert(path);
        }
        for (idx, selected) in self.view.unit_backrefs() {
            if let Some(p) = state.standalone_units.get_mut(idx) {
                p.selected = selected;
            }
        }
    }
}

impl Step for UnifiedSelectionStep {
    fn name(&self) -> &'static str {
        "unified-selection"
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, state: &WizardState) {
        self.ensure_initialised(state);
        render::render(frame, area, &self.view, &self.overlay, self.cursor);
    }

    fn handle_event(&mut self, event: &Event, state: &mut WizardState) -> StepResult {
        self.ensure_initialised(state);
        let Event::Key(key) = event else {
            return StepResult::Continue;
        };
        let n = self.view.flat_indices().len();
        match (key.code, key.modifiers) {
            (KeyCode::Char('c'), KeyModifiers::CONTROL) | (KeyCode::Char('q'), _) => {
                StepResult::Exit(WizardOutcome::Cancelled)
            }
            (KeyCode::Down | KeyCode::Char('j'), _) if n > 0 => {
                self.cursor = (self.cursor + 1) % n;
                StepResult::Continue
            }
            (KeyCode::Up | KeyCode::Char('k'), _) if n > 0 => {
                self.cursor = if self.cursor == 0 {
                    n - 1
                } else {
                    self.cursor - 1
                };
                StepResult::Continue
            }
            (KeyCode::Char(' '), _) => {
                if let Some(idx) = self.current_idx() {
                    self.view.toggle(idx);
                }
                StepResult::Continue
            }
            (KeyCode::Char('a'), _) => {
                self.view.set_all_togglable(true);
                StepResult::Continue
            }
            (KeyCode::Char('n'), _) => {
                self.view.set_all_togglable(false);
                StepResult::Continue
            }
            (KeyCode::Char('c'), _) => {
                if let Some(RowIdx::Unit(i)) = self.current_idx() {
                    if let Some(unit_row) = self.view.units.get(i) {
                        return StepResult::Next(Box::new(CascadeFromStep::new(
                            unit_row.name.clone(),
                            state,
                        )));
                    }
                }
                StepResult::Continue
            }
            (KeyCode::Enter | KeyCode::Char('y'), _) => {
                let count = self.view.selected_togglable_count();
                if count == 0 {
                    state.error_message =
                        Some("Please select at least one ReleaseUnit".to_string());
                    return StepResult::Continue;
                }
                state.error_message = None;
                self.flush_to_state(state);
                if state.preset_from_cli {
                    if count == 1 {
                        StepResult::Next(Box::new(TagFormatStep::new()))
                    } else {
                        StepResult::Next(Box::new(UpstreamConfigStep::new()))
                    }
                } else {
                    StepResult::Next(Box::new(PresetSelectionStep::new(state)))
                }
            }
            (KeyCode::Esc, _) => StepResult::Back,
            _ => StepResult::Continue,
        }
    }
}
