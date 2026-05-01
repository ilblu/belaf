//! Final review screen. ENTER/y confirms (orchestrator runs the
//! bootstrap), n/Esc goes back to upstream config.

use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use super::{
    state::WizardState,
    step::{Step, StepResult, WizardOutcome},
};

#[derive(Default)]
pub struct ConfirmationStep;

impl ConfirmationStep {
    pub fn new() -> Self {
        Self
    }
}

impl Step for ConfirmationStep {
    fn name(&self) -> &'static str {
        "confirmation"
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, state: &WizardState) {
        render(frame, area, state);
    }

    fn handle_event(&mut self, event: &Event, _state: &mut WizardState) -> StepResult {
        let Event::Key(key) = event else {
            return StepResult::Continue;
        };

        match (key.code, key.modifiers) {
            (KeyCode::Char('c'), KeyModifiers::CONTROL) | (KeyCode::Char('q'), _) => {
                StepResult::Exit(WizardOutcome::Cancelled)
            }
            (KeyCode::Enter | KeyCode::Char('y'), _) => StepResult::Exit(WizardOutcome::Confirmed),
            (KeyCode::Char('n') | KeyCode::Esc, _) => StepResult::Back,
            _ => StepResult::Continue,
        }
    }
}

fn render(frame: &mut Frame, area: Rect, state: &WizardState) {
    let selected = state.selected_units();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " Step 4: Confirmation ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(inner_area);

    let header_lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("📋 ", Style::default()),
            Span::styled(
                "Review your configuration before initializing",
                Style::default().fg(Color::White),
            ),
        ]),
    ];
    let header = Paragraph::new(header_lines).alignment(Alignment::Center);
    frame.render_widget(header, chunks[0]);

    let content_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .margin(1)
        .split(chunks[1]);

    let mut summary_lines = vec![
        Line::from(vec![
            Span::styled("🔗 ", Style::default()),
            Span::styled("Repository", Style::default().fg(Color::White)),
        ]),
        Line::from(Span::styled(
            format!("   {}", state.upstream_url),
            Style::default().fg(Color::Gray),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("📦 ", Style::default()),
            Span::styled(
                format!("Projects ({})", selected.len()),
                Style::default().fg(Color::White),
            ),
        ]),
    ];

    for unit in selected.iter().take(8) {
        summary_lines.push(Line::from(vec![
            Span::styled("   ✅ ", Style::default().fg(Color::Green)),
            Span::styled(unit.name.clone(), Style::default().fg(Color::White)),
            Span::styled(
                format!(" @ {}", unit.version),
                Style::default().fg(Color::Gray),
            ),
        ]));
    }

    if selected.len() > 8 {
        summary_lines.push(Line::from(Span::styled(
            format!("   ... and {} more", selected.len() - 8),
            Style::default().fg(Color::Gray),
        )));
    }

    let summary_block = Paragraph::new(summary_lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Gray))
            .title(Span::styled(" Summary ", Style::default().fg(Color::White))),
    );
    frame.render_widget(summary_block, content_chunks[0]);

    let action_lines = vec![
        Line::from(vec![
            Span::styled("⚡ ", Style::default()),
            Span::styled("Actions", Style::default().fg(Color::White)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("   📄 ", Style::default().fg(Color::Cyan)),
            Span::styled("Create belaf/config.toml", Style::default().fg(Color::Gray)),
        ]),
        Line::from(vec![
            Span::styled("   ✏️  ", Style::default().fg(Color::Yellow)),
            Span::styled(
                "Update project version files",
                Style::default().fg(Color::Gray),
            ),
        ]),
        Line::from(vec![
            Span::styled("   🏷️  ", Style::default().fg(Color::Green)),
            Span::styled("Create baseline Git tags", Style::default().fg(Color::Gray)),
        ]),
    ];

    let action_block = Paragraph::new(action_lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Gray))
            .title(Span::styled(
                " Will Execute ",
                Style::default().fg(Color::White),
            )),
    );
    frame.render_widget(action_block, content_chunks[1]);

    let hints = Line::from(vec![
        Span::styled("Enter/y", Style::default().fg(Color::Green)),
        Span::styled(" confirm  ", Style::default().fg(Color::Gray)),
        Span::styled("Backspace/n", Style::default().fg(Color::Yellow)),
        Span::styled(" go back  ", Style::default().fg(Color::Gray)),
        Span::styled("q", Style::default().fg(Color::Red)),
        Span::styled(" quit", Style::default().fg(Color::Gray)),
    ]);
    let hints_para = Paragraph::new(hints).alignment(Alignment::Center);
    frame.render_widget(hints_para, chunks[2]);
}

#[cfg(test)]
mod tests {
    use super::super::{
        state::{DetectedUnit, WizardState},
        step::test_support::render_to_string,
    };
    use super::*;

    #[test]
    fn renders_confirmation_with_projects() {
        let mut state = WizardState::new(false, None);
        state.upstream_url = "https://github.com/example/repo".into();
        state.standalone_units = vec![
            DetectedUnit {
                name: "alpha".into(),
                version: "0.1.0".into(),
                prefix: "crates/alpha".into(),
                selected: true,
                ecosystem: None,
            },
            DetectedUnit {
                name: "beta".into(),
                version: "0.2.3".into(),
                prefix: "crates/beta".into(),
                selected: true,
                ecosystem: None,
            },
        ];
        let mut step = ConfirmationStep::new();
        let out = render_to_string(&mut step, &state, 100, 24);
        insta::assert_snapshot!("confirmation_two_projects", out);
    }
}
