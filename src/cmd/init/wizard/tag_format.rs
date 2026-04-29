//! Phase I.3 — single-project tag-format sub-prompt.
//!
//! Shown after [`ProjectSelectionStep`](super::project::ProjectSelectionStep)
//! when the user selected exactly one project. Single-project repos
//! conventionally use `v{version}` tags (Cargo, semver-tagged
//! libraries) instead of the namespaced `{name}-v{version}` default
//! that's safer for monorepos. We surface the choice rather than
//! guess.

use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

use super::{
    state::WizardState,
    step::{Step, StepResult, WizardOutcome},
    upstream::UpstreamConfigStep,
};

/// Two presented options. `None` (no override) is equivalent to
/// "use ecosystem default", which is what monorepo projects want.
const OPTIONS: &[(&str, Option<&str>, &str)] = &[
    (
        "v{version}",
        Some("v{version}"),
        "Simple — recommended for single-project repos",
    ),
    (
        "{name}-v{version}",
        None,
        "Namespaced — ecosystem default; safer when more crates are added later",
    ),
];

#[derive(Default)]
pub struct TagFormatStep {
    cursor: usize,
}

impl TagFormatStep {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Step for TagFormatStep {
    fn name(&self) -> &'static str {
        "tag-format"
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, state: &WizardState) {
        render(frame, area, state, self.cursor);
    }

    fn handle_event(&mut self, event: &Event, state: &mut WizardState) -> StepResult {
        let Event::Key(key) = event else {
            return StepResult::Continue;
        };

        match (key.code, key.modifiers) {
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                StepResult::Exit(WizardOutcome::Cancelled)
            }
            (KeyCode::Down | KeyCode::Char('j'), _) => {
                self.cursor = (self.cursor + 1) % OPTIONS.len();
                StepResult::Continue
            }
            (KeyCode::Up | KeyCode::Char('k'), _) => {
                self.cursor = if self.cursor == 0 {
                    OPTIONS.len() - 1
                } else {
                    self.cursor - 1
                };
                StepResult::Continue
            }
            (KeyCode::Enter, _) => {
                let (_, override_value, _) = OPTIONS[self.cursor];
                state.tag_format_override = override_value.map(String::from);
                StepResult::Next(Box::new(UpstreamConfigStep::new()))
            }
            (KeyCode::Esc, _) => StepResult::Back,
            _ => StepResult::Continue,
        }
    }
}

fn render(frame: &mut Frame, area: Rect, state: &WizardState, cursor: usize) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " Tag Format (single-project repo) ",
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

    let project_name = state
        .selected_projects()
        .first()
        .map(|p| p.name.as_str())
        .unwrap_or("(none)");

    let header_lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("🏷  ", Style::default()),
            Span::styled(
                format!("Detected single project: {}", project_name),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(Span::styled(
            "   Choose how release tags should be formatted",
            Style::default().fg(Color::Gray),
        )),
    ];
    let header = Paragraph::new(header_lines).alignment(Alignment::Center);
    frame.render_widget(header, chunks[0]);

    let items: Vec<ListItem> = OPTIONS
        .iter()
        .enumerate()
        .map(|(idx, (label, _, description))| {
            let is_selected = idx == cursor;
            let lines = vec![
                Line::from(vec![
                    Span::styled(
                        if is_selected { " ▶ " } else { "   " },
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::styled(
                        (*label).to_string(),
                        if is_selected {
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::White)
                        },
                    ),
                ]),
                Line::from(Span::styled(
                    format!("       {}", description),
                    Style::default().fg(Color::Gray),
                )),
            ];
            let style = if is_selected {
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
            .title(Span::styled(" Options ", Style::default().fg(Color::White))),
    );
    frame.render_widget(list, chunks[1]);

    let hints = Line::from(vec![
        Span::styled("↑↓", Style::default().fg(Color::Cyan)),
        Span::styled(" select  ", Style::default().fg(Color::Gray)),
        Span::styled("Enter", Style::default().fg(Color::Green)),
        Span::styled(" continue  ", Style::default().fg(Color::Gray)),
        Span::styled("Esc", Style::default().fg(Color::Cyan)),
        Span::styled(" back  ", Style::default().fg(Color::Gray)),
        Span::styled("q", Style::default().fg(Color::Red)),
        Span::styled(" quit", Style::default().fg(Color::Gray)),
    ]);
    let hints_para = Paragraph::new(hints).alignment(Alignment::Center);
    frame.render_widget(hints_para, chunks[2]);
}

#[cfg(test)]
mod tests {
    use super::super::{
        state::{DetectedProject, WizardState},
        step::test_support::render_to_string,
    };
    use super::*;

    #[test]
    fn renders_tag_format_with_single_project() {
        let mut state = WizardState::new(false, None);
        state.projects = vec![DetectedProject {
            name: "alpha".into(),
            version: "0.1.0".into(),
            prefix: "crates/alpha".into(),
            selected: true,
        }];
        let mut step = TagFormatStep::new();
        let out = render_to_string(&mut step, &state, 80, 24);
        insta::assert_snapshot!("tag_format_single_project", out);
    }
}
