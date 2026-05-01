//! Project-selection step. Owns the cursor and the toggle/select-all/
//! deselect-all keybindings.

use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

use crate::core::ui::utils::centered_rect;

use super::{
    state::WizardState,
    step::{Step, StepResult, WizardOutcome},
    tag_format::TagFormatStep,
    upstream::UpstreamConfigStep,
};

#[derive(Default)]
pub struct ProjectSelectionStep {
    cursor: usize,
}

impl ProjectSelectionStep {
    pub fn new() -> Self {
        Self::default()
    }

    fn toggle_current(&mut self, state: &mut WizardState) {
        if let Some(proj) = state.projects.get_mut(self.cursor) {
            proj.selected = !proj.selected;
        }
    }

    fn select_all(state: &mut WizardState) {
        for proj in &mut state.projects {
            proj.selected = true;
        }
    }

    fn deselect_all(state: &mut WizardState) {
        for proj in &mut state.projects {
            proj.selected = false;
        }
    }
}

impl Step for ProjectSelectionStep {
    fn name(&self) -> &'static str {
        "project"
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, state: &WizardState) {
        render(frame, area, state, self.cursor);
    }

    fn handle_event(&mut self, event: &Event, state: &mut WizardState) -> StepResult {
        let Event::Key(key) = event else {
            return StepResult::Continue;
        };

        match (key.code, key.modifiers) {
            (KeyCode::Char('c'), KeyModifiers::CONTROL) | (KeyCode::Char('q'), _) => {
                StepResult::Exit(WizardOutcome::Cancelled)
            }
            (KeyCode::Down | KeyCode::Char('j'), _) => {
                if !state.projects.is_empty() {
                    self.cursor = (self.cursor + 1) % state.projects.len();
                }
                StepResult::Continue
            }
            (KeyCode::Up | KeyCode::Char('k'), _) => {
                if !state.projects.is_empty() {
                    self.cursor = if self.cursor == 0 {
                        state.projects.len() - 1
                    } else {
                        self.cursor - 1
                    };
                }
                StepResult::Continue
            }
            (KeyCode::Char(' '), _) => {
                self.toggle_current(state);
                StepResult::Continue
            }
            (KeyCode::Char('a'), _) => {
                Self::select_all(state);
                StepResult::Continue
            }
            (KeyCode::Char('n'), _) => {
                Self::deselect_all(state);
                StepResult::Continue
            }
            (KeyCode::Enter, _) => {
                let count = state.selected_projects().len();
                if count == 0 {
                    state.error_message = Some("Please select at least one project".to_string());
                    StepResult::Continue
                } else {
                    state.error_message = None;
                    if count == 1 {
                        StepResult::Next(Box::new(TagFormatStep::new()))
                    } else {
                        StepResult::Next(Box::new(UpstreamConfigStep::new()))
                    }
                }
            }
            (KeyCode::Esc, _) => StepResult::Back,
            _ => StepResult::Continue,
        }
    }
}

fn render(frame: &mut Frame, area: Rect, state: &WizardState, cursor: usize) {
    let selected_count = state.projects.iter().filter(|p| p.selected).count();
    let total_count = state.projects.len();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " Step 2: Project Selection ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(inner_area);

    let header_lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("📦 ", Style::default()),
            Span::styled(
                "Select which projects to include in release management",
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("   Selected: ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("{}", selected_count),
                Style::default()
                    .fg(if selected_count > 0 {
                        Color::Green
                    } else {
                        Color::Yellow
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" / {}", total_count),
                Style::default().fg(Color::Gray),
            ),
        ]),
    ];
    let header = Paragraph::new(header_lines).alignment(Alignment::Center);
    frame.render_widget(header, chunks[0]);

    let items: Vec<ListItem> = state
        .projects
        .iter()
        .enumerate()
        .map(|(idx, proj)| {
            let is_current = idx == cursor;
            let checkbox = if proj.selected { "✅" } else { "⬜" };
            let lines = vec![Line::from(vec![
                Span::styled(format!(" {} ", checkbox), Style::default()),
                Span::styled(
                    proj.name.clone(),
                    if is_current {
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else if proj.selected {
                        Style::default().fg(Color::Green)
                    } else {
                        Style::default().fg(Color::White)
                    },
                ),
                Span::styled(
                    format!(" @ {}", proj.version),
                    Style::default().fg(Color::Gray),
                ),
                Span::styled(
                    format!("  ({})", proj.prefix),
                    Style::default().fg(Color::Rgb(100, 100, 100)),
                ),
            ])];
            let style = if is_current {
                Style::default().bg(Color::Rgb(40, 40, 50))
            } else {
                Style::default()
            };
            ListItem::new(lines).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Gray))
            .title(Span::styled(
                " Projects ",
                Style::default().fg(Color::White),
            )),
    );

    frame.render_widget(list, chunks[1]);

    let hints = Line::from(vec![
        Span::styled("↑↓", Style::default().fg(Color::Cyan)),
        Span::styled(" navigate  ", Style::default().fg(Color::Gray)),
        Span::styled("Space", Style::default().fg(Color::Cyan)),
        Span::styled(" toggle  ", Style::default().fg(Color::Gray)),
        Span::styled("a", Style::default().fg(Color::Green)),
        Span::styled(" all  ", Style::default().fg(Color::Gray)),
        Span::styled("n", Style::default().fg(Color::Yellow)),
        Span::styled(" none  ", Style::default().fg(Color::Gray)),
        Span::styled("Enter", Style::default().fg(Color::Green)),
        Span::styled(" continue  ", Style::default().fg(Color::Gray)),
        Span::styled("q", Style::default().fg(Color::Red)),
        Span::styled(" quit", Style::default().fg(Color::Gray)),
    ]);
    let hints_para = Paragraph::new(hints).alignment(Alignment::Center);
    frame.render_widget(hints_para, chunks[2]);

    if let Some(ref error) = state.error_message {
        let popup_area = centered_rect(60, 20, area);
        frame.render_widget(Clear, popup_area);
        let popup = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(error.clone(), Style::default().fg(Color::Red))),
        ])
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red))
                .title(Span::styled(
                    " ⚠ Error ",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                )),
        );
        frame.render_widget(popup, popup_area);
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        state::{DetectedProject, WizardState},
        step::test_support::render_to_string,
    };
    use super::*;

    fn state_with_projects() -> WizardState {
        let mut state = WizardState::new(false, None);
        state.projects = vec![
            DetectedProject {
                name: "alpha".into(),
                version: "0.1.0".into(),
                prefix: "crates/alpha".into(),
                selected: true,
            },
            DetectedProject {
                name: "beta".into(),
                version: "0.2.3".into(),
                prefix: "crates/beta".into(),
                selected: false,
            },
        ];
        state
    }

    #[test]
    fn renders_project_step() {
        let state = state_with_projects();
        let mut step = ProjectSelectionStep::new();
        let out = render_to_string(&mut step, &state, 80, 24);
        insta::assert_snapshot!("project_selection", out);
    }
}
