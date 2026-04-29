//! Step trait + dispatch types for the modular init wizard.
//!
//! Phase I refactor (BELAF_MASTER_PLAN.md). The wizard is a stack-based
//! state machine: the orchestrator holds `Vec<Box<dyn Step>>`, draws
//! the top-of-stack, dispatches input, and reacts to the returned
//! [`StepResult`] (continue / next / back / exit).
//!
//! Each concrete step lives in its own module
//! (`welcome.rs` / `preset.rs` / `project.rs` / `upstream.rs` /
//! `confirmation.rs`) and `impl Step`s the trait below. The
//! pre-refactor monolithic match-on-`WizardStep`-enum is gone;
//! state-transitions live next to the step that produces them.

use crossterm::event::{Event, MouseEvent};
use ratatui::{layout::Rect, Frame};

use super::state::WizardState;

/// One screen of the wizard. Each `impl Step` owns its
/// step-private state (cursor positions, scroll offsets,
/// sub-classification choices) and reads/mutates the shared
/// [`WizardState`] for cross-step data.
pub trait Step {
    /// Stable identifier for diagnostics / snapshot tests.
    #[expect(dead_code, reason = "wired up by snapshot tests in a later commit")]
    fn name(&self) -> &'static str;

    /// Render this step's UI. `&mut self` because some embedded
    /// widgets (e.g. [`TogglePanel`](crate::core::ui::components::toggle_panel::TogglePanel))
    /// stash the rendered area on themselves so that mouse hits can
    /// be matched on the next event tick. Renderers should still be
    /// effectively pure of business logic — input handling lives in
    /// [`Step::handle_event`].
    fn render(&mut self, frame: &mut Frame, area: Rect, state: &WizardState);

    /// Handle one keyboard / mouse / resize event. Returns the
    /// result the orchestrator acts on.
    fn handle_event(&mut self, event: &Event, state: &mut WizardState) -> StepResult;

    /// Optional: handle an explicit mouse-click hit-test before the
    /// generic dispatcher. Used by [`crate::cmd::init::wizard::preset::PresetSelectionStep`]
    /// for the toggle-panel-click. Default returns `None` (use
    /// generic event flow).
    fn handle_click(
        &mut self,
        _click: &MouseClick,
        _state: &mut WizardState,
    ) -> Option<StepResult> {
        None
    }
}

/// What the orchestrator should do after this step returned.
pub enum StepResult {
    /// Stay on this step, redraw on next frame.
    Continue,

    /// Push the named successor step onto the stack. The orchestrator
    /// owns the step's lifetime; on `Back`, it pops back to this one.
    Next(Box<dyn Step>),

    /// Pop this step off and resume the previous one. If the stack
    /// would empty, the orchestrator treats this as `Exit(Cancelled)`.
    Back,

    /// Terminate the wizard with the given outcome.
    Exit(WizardOutcome),
}

/// Why the wizard finished.
#[derive(Debug)]
#[expect(
    dead_code,
    reason = "SuggestedAlternative used by the single-mobile-repo step in a later commit"
)]
pub enum WizardOutcome {
    /// User confirmed at the final step. Caller should run
    /// `execute_bootstrap_with_output(state, repo)`.
    Confirmed,

    /// User pressed `q` / Ctrl-C. Caller returns exit code 1
    /// without bootstrapping.
    Cancelled,

    /// User detected a single-mobile-repo (Phase I.4) and the wizard
    /// suggested an alternative tool. Carries a brief message to
    /// display before exit.
    SuggestedAlternative(String),
}

/// Minimal mouse-click descriptor passed to
/// [`Step::handle_click`]. We don't pass the full crossterm event
/// because most step impls don't need it.
#[derive(Debug, Clone, Copy)]
pub struct MouseClick {
    pub column: u16,
    pub row: u16,
}

impl From<&MouseEvent> for MouseClick {
    fn from(e: &MouseEvent) -> Self {
        Self {
            column: e.column,
            row: e.row,
        }
    }
}

#[cfg(test)]
pub(super) mod test_support {
    //! Snapshot harness for step renderers.
    //!
    //! Each step's tests build a [`WizardState`], hand the step to
    //! [`render_to_string`], and pipe the result through
    //! `insta::assert_snapshot!` so a render-output regression is
    //! reviewable as a snapshot diff.
    use ratatui::{backend::TestBackend, Terminal};

    use super::{Step, WizardState};

    pub fn render_to_string(
        step: &mut dyn Step,
        state: &WizardState,
        width: u16,
        height: u16,
    ) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test terminal init");
        terminal
            .draw(|frame| {
                let area = frame.area();
                step.render(frame, area, state);
            })
            .expect("test draw");
        format!("{}", terminal.backend())
    }
}
