//! `[c]` interactive cascade-from setup — pushed by
//! [`UnifiedSelectionStep`](super::unified_selection::UnifiedSelectionStep)
//! when the user cursors on a Standalone row carrying an
//! `sdk-cascade` annotation and presses `c`.
//!
//! Two sub-screens, transitioned by `Enter` and reverted by `Esc`:
//!
//! 1. **Source picker** — list of all *other* standalone units; user
//!    picks which unit's bumps cascade into this one.
//! 2. **Strategy picker** — fixed list of 4 strategies (mirror /
//!    floor_patch / floor_minor / floor_major).
//!
//! On final Enter the choice is written into
//! [`WizardState::cascade_overrides`] and the step pops back into the
//! parent unified-selection list, which redraws with the new
//! annotation visible. The auto_detect snippet emitter consumes the
//! map at bootstrap time and writes a `cascade_from = { ... }` field
//! into the unit's `[release_unit.<name>]` block.

use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

use super::state::{CascadeOverride, CascadeStrategy, WizardState};
use super::step::{Step, StepResult, WizardOutcome};

/// Which sub-screen we're rendering.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum SubMode {
    SourcePicker,
    StrategyPicker,
}

pub struct CascadeFromStep {
    /// The unit name we're configuring cascade-from for.
    target_unit: String,
    mode: SubMode,
    cursor: usize,
    /// Pinned at construction so cursor indices stay stable while the
    /// user navigates (we can't borrow state during render-and-handle).
    sources: Vec<String>,
    chosen_source: Option<String>,
}

const STRATEGIES: [CascadeStrategy; 4] = [
    CascadeStrategy::FloorMinor,
    CascadeStrategy::FloorPatch,
    CascadeStrategy::FloorMajor,
    CascadeStrategy::Mirror,
];

impl CascadeFromStep {
    /// Build a new step. `target_unit` is the unit name for which
    /// cascade-from is being configured. `state.standalone_units` is
    /// snapshot at construction time so the list stays stable through
    /// the sub-flow.
    pub fn new(target_unit: String, state: &WizardState) -> Self {
        let sources: Vec<String> = state
            .standalone_units
            .iter()
            .filter(|u| u.name != target_unit)
            .map(|u| u.name.clone())
            .collect();
        // Pre-seed cursor at any existing override's source so
        // re-opening the step lands on the previously-chosen row.
        let existing = state.cascade_overrides.get(&target_unit);
        let cursor = existing
            .and_then(|o| sources.iter().position(|s| *s == o.source))
            .unwrap_or(0);
        let chosen_source = existing.map(|o| o.source.clone());
        Self {
            target_unit,
            mode: SubMode::SourcePicker,
            cursor,
            sources,
            chosen_source,
        }
    }
}

impl Step for CascadeFromStep {
    fn name(&self) -> &'static str {
        "cascade-from"
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, _state: &WizardState) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(Span::styled(
                format!(" cascade-from: {} ", self.target_unit),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(4),
                Constraint::Min(0),
                Constraint::Length(2),
            ])
            .split(inner);

        let header = match self.mode {
            SubMode::SourcePicker => vec![
                Line::from(""),
                Line::from(Span::styled(
                    "Pick the source unit whose bumps cascade into",
                    Style::default().fg(Color::White),
                )),
                Line::from(Span::styled(
                    format!("`{}`", self.target_unit),
                    Style::default().fg(Color::Cyan),
                )),
            ],
            SubMode::StrategyPicker => vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled("Source: ", Style::default().fg(Color::Gray)),
                    Span::styled(
                        self.chosen_source.as_deref().unwrap_or("?"),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(Span::styled(
                    "Pick a bump strategy:",
                    Style::default().fg(Color::White),
                )),
            ],
        };
        frame.render_widget(
            Paragraph::new(header).alignment(Alignment::Center),
            chunks[0],
        );

        let items: Vec<ListItem> = match self.mode {
            SubMode::SourcePicker => self
                .sources
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    let style = if i == self.cursor {
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                            .bg(Color::Rgb(40, 40, 50))
                    } else {
                        Style::default().fg(Color::White)
                    };
                    ListItem::new(Line::from(vec![
                        Span::styled("    ", Style::default()),
                        Span::styled(name.clone(), style),
                    ]))
                })
                .collect(),
            SubMode::StrategyPicker => STRATEGIES
                .iter()
                .enumerate()
                .map(|(i, strat)| {
                    let style = if i == self.cursor {
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                            .bg(Color::Rgb(40, 40, 50))
                    } else {
                        Style::default().fg(Color::White)
                    };
                    ListItem::new(Line::from(vec![
                        Span::styled("    ", Style::default()),
                        Span::styled(strat.label().to_string(), style),
                    ]))
                })
                .collect(),
        };
        frame.render_widget(List::new(items), chunks[1]);

        let hints = match self.mode {
            SubMode::SourcePicker => Line::from(vec![
                Span::styled("↑↓", Style::default().fg(Color::Cyan)),
                Span::styled(" navigate  ", Style::default().fg(Color::Gray)),
                Span::styled("Enter", Style::default().fg(Color::Green)),
                Span::styled(" pick source  ", Style::default().fg(Color::Gray)),
                Span::styled("Esc", Style::default().fg(Color::Yellow)),
                Span::styled(" cancel  ", Style::default().fg(Color::Gray)),
                Span::styled("d", Style::default().fg(Color::Red)),
                Span::styled(" delete override", Style::default().fg(Color::Gray)),
            ]),
            SubMode::StrategyPicker => Line::from(vec![
                Span::styled("↑↓", Style::default().fg(Color::Cyan)),
                Span::styled(" navigate  ", Style::default().fg(Color::Gray)),
                Span::styled("Enter", Style::default().fg(Color::Green)),
                Span::styled(" save  ", Style::default().fg(Color::Gray)),
                Span::styled("Esc", Style::default().fg(Color::Yellow)),
                Span::styled(" back to source picker", Style::default().fg(Color::Gray)),
            ]),
        };
        frame.render_widget(
            Paragraph::new(hints).alignment(Alignment::Center),
            chunks[2],
        );
    }

    fn handle_event(&mut self, event: &Event, state: &mut WizardState) -> StepResult {
        let Event::Key(key) = event else {
            return StepResult::Continue;
        };
        let n = match self.mode {
            SubMode::SourcePicker => self.sources.len(),
            SubMode::StrategyPicker => STRATEGIES.len(),
        };

        match (key.code, key.modifiers, self.mode) {
            (KeyCode::Char('c'), KeyModifiers::CONTROL, _) | (KeyCode::Char('q'), _, _) => {
                StepResult::Exit(WizardOutcome::Cancelled)
            }
            (KeyCode::Down | KeyCode::Char('j'), _, _) => {
                if n > 0 {
                    self.cursor = (self.cursor + 1) % n;
                }
                StepResult::Continue
            }
            (KeyCode::Up | KeyCode::Char('k'), _, _) => {
                if n > 0 {
                    self.cursor = if self.cursor == 0 {
                        n - 1
                    } else {
                        self.cursor - 1
                    };
                }
                StepResult::Continue
            }
            (KeyCode::Char('d'), _, SubMode::SourcePicker) => {
                // Remove any existing cascade-override and pop back.
                state.cascade_overrides.remove(&self.target_unit);
                StepResult::Back
            }
            (KeyCode::Enter, _, SubMode::SourcePicker) => {
                if let Some(name) = self.sources.get(self.cursor).cloned() {
                    self.chosen_source = Some(name);
                    self.mode = SubMode::StrategyPicker;
                    self.cursor = 0;
                }
                StepResult::Continue
            }
            (KeyCode::Enter, _, SubMode::StrategyPicker) => {
                if let (Some(source), Some(&strategy)) =
                    (self.chosen_source.clone(), STRATEGIES.get(self.cursor))
                {
                    state.cascade_overrides.insert(
                        self.target_unit.clone(),
                        CascadeOverride { source, strategy },
                    );
                }
                StepResult::Back
            }
            (KeyCode::Esc, _, SubMode::SourcePicker) => StepResult::Back,
            (KeyCode::Esc, _, SubMode::StrategyPicker) => {
                // Step back into the source-picker without committing.
                self.mode = SubMode::SourcePicker;
                self.cursor = 0;
                StepResult::Continue
            }
            _ => StepResult::Continue,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::state::DetectedUnit;
    use super::*;

    fn state_with_three_units() -> WizardState {
        let mut state = WizardState::new(false, None);
        state.standalone_units = vec![
            DetectedUnit {
                name: "schema".into(),
                version: "1.0.0".into(),
                prefix: "schemas/".into(),
                selected: true,
                ecosystem: Some("npm".into()),
            },
            DetectedUnit {
                name: "@org/sdk-ts".into(),
                version: "0.1.0".into(),
                prefix: "sdks/typescript".into(),
                selected: true,
                ecosystem: Some("npm".into()),
            },
            DetectedUnit {
                name: "@org/sdk-py".into(),
                version: "0.1.0".into(),
                prefix: "sdks/python".into(),
                selected: true,
                ecosystem: Some("pypa".into()),
            },
        ];
        state
    }

    #[test]
    fn source_picker_excludes_self() {
        let state = state_with_three_units();
        let step = CascadeFromStep::new("@org/sdk-ts".into(), &state);
        assert_eq!(step.sources, vec!["schema", "@org/sdk-py"]);
    }

    #[test]
    fn enter_picks_source_then_strategy_persists_override() {
        use crossterm::event::{KeyCode, KeyEvent};

        let mut state = state_with_three_units();
        let mut step = CascadeFromStep::new("@org/sdk-ts".into(), &state);

        // Enter on cursor=0 (schema) → mode flips to StrategyPicker.
        let r = step.handle_event(&Event::Key(KeyEvent::from(KeyCode::Enter)), &mut state);
        assert!(matches!(r, StepResult::Continue));
        assert_eq!(step.mode, SubMode::StrategyPicker);
        assert_eq!(step.chosen_source.as_deref(), Some("schema"));

        // Enter on cursor=0 (FloorMinor — first in STRATEGIES) → Back.
        let r = step.handle_event(&Event::Key(KeyEvent::from(KeyCode::Enter)), &mut state);
        assert!(matches!(r, StepResult::Back));
        let ov = state
            .cascade_overrides
            .get("@org/sdk-ts")
            .expect("override persisted");
        assert_eq!(ov.source, "schema");
        assert_eq!(ov.strategy, CascadeStrategy::FloorMinor);
    }

    #[test]
    fn d_key_deletes_existing_override() {
        use crossterm::event::{KeyCode, KeyEvent};

        let mut state = state_with_three_units();
        state.cascade_overrides.insert(
            "@org/sdk-ts".to_string(),
            CascadeOverride {
                source: "schema".into(),
                strategy: CascadeStrategy::Mirror,
            },
        );
        let mut step = CascadeFromStep::new("@org/sdk-ts".into(), &state);

        let r = step.handle_event(&Event::Key(KeyEvent::from(KeyCode::Char('d'))), &mut state);
        assert!(matches!(r, StepResult::Back));
        assert!(!state.cascade_overrides.contains_key("@org/sdk-ts"));
    }

    #[test]
    fn esc_in_strategy_returns_to_source_picker() {
        use crossterm::event::{KeyCode, KeyEvent};

        let mut state = state_with_three_units();
        let mut step = CascadeFromStep::new("@org/sdk-ts".into(), &state);
        step.mode = SubMode::StrategyPicker;
        step.chosen_source = Some("schema".into());
        step.cursor = 2;

        let r = step.handle_event(&Event::Key(KeyEvent::from(KeyCode::Esc)), &mut state);
        assert!(matches!(r, StepResult::Continue));
        assert_eq!(step.mode, SubMode::SourcePicker);
        assert_eq!(step.cursor, 0);
        assert!(state.cascade_overrides.is_empty());
    }
}
