//! Welcome / acknowledgement step. Renders logo + project count +
//! warnings (uncommitted changes, existing config). ENTER starts the
//! flow; in interactive mode pressing ENTER implies `--force` —
//! pinned by `tests/test_dirty_repository.rs`.

use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use super::{
    preset::PresetSelectionStep,
    state::WizardState,
    step::{Step, StepResult, WizardOutcome},
    unified_selection::UnifiedSelectionStep,
};

#[derive(Default)]
pub struct WelcomeStep;

impl WelcomeStep {
    pub fn new() -> Self {
        Self
    }
}

impl Step for WelcomeStep {
    fn name(&self) -> &'static str {
        "welcome"
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, state: &WizardState) {
        render_welcome(frame, area, state);
    }

    fn handle_event(&mut self, event: &Event, state: &mut WizardState) -> StepResult {
        let Event::Key(key) = event else {
            return StepResult::Continue;
        };

        match (key.code, key.modifiers) {
            (KeyCode::Char('c'), KeyModifiers::CONTROL) | (KeyCode::Char('q'), _) => {
                StepResult::Exit(WizardOutcome::Cancelled)
            }
            (KeyCode::Enter, _) => {
                // Welcome IS the confirmation step. Any warnings shown
                // above (uncommitted changes, existing config) are
                // displayed there; pressing ENTER acknowledges them
                // and proceeds. The interactive flow does not require
                // --force on top of an explicit Enter.
                state.force = true;
                state.error_message = None;
                // UnifiedSelectionStep covers both auto-detected
                // bundles and manual project list. Skip it only when
                // both are empty (preset-only flow).
                if !state.detection.matches.is_empty() || !state.standalone_units.is_empty() {
                    StepResult::Next(Box::new(UnifiedSelectionStep::new()))
                } else {
                    StepResult::Next(Box::new(PresetSelectionStep::new(state)))
                }
            }
            _ => StepResult::Continue,
        }
    }
}

fn render_welcome(frame: &mut Frame, area: Rect, state: &WizardState) {
    let is_reconfigure = state.config_exists;

    let border_color = if is_reconfigure {
        Color::Red
    } else {
        Color::Cyan
    };

    let logo_color = if is_reconfigure {
        Color::Red
    } else {
        Color::Cyan
    };

    let unit_count = state.standalone_units.len();
    let unit_text = if unit_count == 1 {
        "1 project".to_string()
    } else {
        format!("{} projects", unit_count)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(if is_reconfigure {
            Span::styled(
                " ⚠ Reconfigure ",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(
                " Welcome ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
        });

    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    if is_reconfigure {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(8),
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(3),
            ])
            .margin(1)
            .split(inner_area);

        let logo = belaf_logo(logo_color);
        let logo_para = Paragraph::new(logo).alignment(Alignment::Center);
        frame.render_widget(logo_para, chunks[0]);

        let warning_header = vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                "⚠️  RECONFIGURE MODE  ⚠️",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )]),
        ];
        let warning_para = Paragraph::new(warning_header).alignment(Alignment::Center);
        frame.render_widget(warning_para, chunks[1]);

        let warning_text = vec![
            Line::from(Span::styled(
                "A configuration already exists in this repo.",
                Style::default().fg(Color::Yellow),
            )),
            Line::from(Span::styled(
                "Continuing will overwrite your settings.",
                Style::default().fg(Color::Yellow),
            )),
        ];
        let warning_text_para = Paragraph::new(warning_text).alignment(Alignment::Center);
        frame.render_widget(warning_text_para, chunks[2]);

        let mut info_lines = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("📦 ", Style::default()),
                Span::styled("Detected: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    unit_text,
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
        ];

        if let Some(ref warning) = state.dirty_warning {
            info_lines.push(Line::from(""));
            info_lines.push(Line::from(vec![
                Span::styled("⚠️  ", Style::default()),
                Span::styled(warning.clone(), Style::default().fg(Color::Yellow)),
            ]));
        }

        if let Some(ref error) = state.error_message {
            info_lines.push(Line::from(""));
            info_lines.push(Line::from(vec![
                Span::styled("❌ ", Style::default()),
                Span::styled(error.clone(), Style::default().fg(Color::Red)),
            ]));
        }
        let info_para = Paragraph::new(info_lines).alignment(Alignment::Center);
        frame.render_widget(info_para, chunks[3]);

        let has_warnings = state.dirty_warning.is_some() || state.error_message.is_some();
        let enter_label = if has_warnings {
            " to override and reconfigure  •  "
        } else {
            " to reconfigure  •  "
        };
        let action_text = vec![Line::from(vec![
            Span::styled("Press ", Style::default().fg(Color::Gray)),
            Span::styled(
                "ENTER",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::styled(enter_label, Style::default().fg(Color::Gray)),
            Span::styled(
                "Q",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" to quit", Style::default().fg(Color::Gray)),
        ])];
        let action_para = Paragraph::new(action_text).alignment(Alignment::Center);
        frame.render_widget(action_para, chunks[4]);
    } else {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(8),
                Constraint::Min(0),
                Constraint::Length(3),
            ])
            .margin(1)
            .split(inner_area);

        let logo = belaf_logo(logo_color);
        let logo_para = Paragraph::new(logo).alignment(Alignment::Center);
        frame.render_widget(logo_para, chunks[0]);

        let mut info_lines = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("📦 ", Style::default()),
                Span::styled("Detected: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    unit_text,
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(""),
        ];

        if let Some(ref warning) = state.dirty_warning {
            info_lines.push(Line::from(vec![
                Span::styled("⚠️  ", Style::default()),
                Span::styled(warning.clone(), Style::default().fg(Color::Yellow)),
            ]));
            info_lines.push(Line::from(""));
        }

        if let Some(ref error) = state.error_message {
            info_lines.push(Line::from(vec![
                Span::styled("❌ ", Style::default()),
                Span::styled(error.clone(), Style::default().fg(Color::Red)),
            ]));
            info_lines.push(Line::from(""));
        }

        info_lines.push(Line::from(""));
        info_lines.push(Line::from(Span::styled(
            "This wizard will guide you through:",
            Style::default().fg(Color::White),
        )));
        info_lines.push(Line::from(vec![
            Span::styled("  → ", Style::default().fg(Color::Cyan)),
            Span::styled(
                "Changelog preset selection",
                Style::default().fg(Color::Gray),
            ),
        ]));
        info_lines.push(Line::from(vec![
            Span::styled("  → ", Style::default().fg(Color::Cyan)),
            Span::styled(
                "ReleaseUnit configuration",
                Style::default().fg(Color::Gray),
            ),
        ]));
        info_lines.push(Line::from(vec![
            Span::styled("  → ", Style::default().fg(Color::Cyan)),
            Span::styled("Repository setup", Style::default().fg(Color::Gray)),
        ]));

        let info_para = Paragraph::new(info_lines).alignment(Alignment::Center);
        frame.render_widget(info_para, chunks[1]);

        let has_warnings = state.dirty_warning.is_some() || state.error_message.is_some();
        let enter_label = if has_warnings {
            " to override and start  •  "
        } else {
            " to start  •  "
        };
        let action_text = vec![Line::from(vec![
            Span::styled("Press ", Style::default().fg(Color::Gray)),
            Span::styled(
                "ENTER",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(enter_label, Style::default().fg(Color::Gray)),
            Span::styled(
                "Q",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" to quit", Style::default().fg(Color::Gray)),
        ])];
        let action_para = Paragraph::new(action_text).alignment(Alignment::Center);
        frame.render_widget(action_para, chunks[2]);
    }
}

fn belaf_logo(color: Color) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            "██████╗ ███████╗██╗      █████╗ ███████╗",
            Style::default().fg(color),
        )),
        Line::from(Span::styled(
            "██╔══██╗██╔════╝██║     ██╔══██╗██╔════╝",
            Style::default().fg(color),
        )),
        Line::from(Span::styled(
            "██████╔╝█████╗  ██║     ███████║█████╗  ",
            Style::default().fg(color),
        )),
        Line::from(Span::styled(
            "██╔══██╗██╔══╝  ██║     ██╔══██║██╔══╝  ",
            Style::default().fg(color),
        )),
        Line::from(Span::styled(
            "██████╔╝███████╗███████╗██║  ██║██║     ",
            Style::default().fg(color),
        )),
        Line::from(Span::styled(
            "╚═════╝ ╚══════╝╚══════╝╚═╝  ╚═╝╚═╝     ",
            Style::default().fg(color),
        )),
        Line::from(Span::styled(
            "           Release Management",
            Style::default().fg(Color::Gray),
        )),
    ]
}

#[cfg(test)]
mod tests {
    use super::super::{state::WizardState, step::test_support::render_to_string};
    use super::*;

    fn fresh_state() -> WizardState {
        WizardState::new(false, None)
    }

    #[test]
    fn renders_first_run_welcome() {
        let mut step = WelcomeStep::new();
        let state = fresh_state();
        let out = render_to_string(&mut step, &state, 80, 24);
        insta::assert_snapshot!("welcome_first_run", out);
    }

    #[test]
    fn renders_reconfigure_welcome() {
        let mut step = WelcomeStep::new();
        let mut state = fresh_state();
        state.config_exists = true;
        let out = render_to_string(&mut step, &state, 80, 24);
        insta::assert_snapshot!("welcome_reconfigure", out);
    }

    #[test]
    fn renders_dirty_warning() {
        let mut step = WelcomeStep::new();
        let mut state = fresh_state();
        state.dirty_warning =
            Some("Warning: uncommitted changes detected (e.g.: src/foo.rs)".into());
        state.error_message =
            Some("Repository has uncommitted changes. Use --force to override.".into());
        let out = render_to_string(&mut step, &state, 80, 24);
        insta::assert_snapshot!("welcome_dirty_warning", out);
    }
}
