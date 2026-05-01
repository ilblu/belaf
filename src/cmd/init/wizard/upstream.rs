//! Upstream-URL configuration step. Owns the input-active toggle.

use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::core::ui::utils::centered_rect;

use super::{
    confirmation::ConfirmationStep,
    state::WizardState,
    step::{Step, StepResult, WizardOutcome},
};

#[derive(Default)]
pub struct UpstreamConfigStep {
    input_active: bool,
}

impl UpstreamConfigStep {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Step for UpstreamConfigStep {
    fn name(&self) -> &'static str {
        "upstream"
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, state: &WizardState) {
        render(frame, area, state, self.input_active);
    }

    fn handle_event(&mut self, event: &Event, state: &mut WizardState) -> StepResult {
        let Event::Key(key) = event else {
            return StepResult::Continue;
        };

        match (key.code, key.modifiers) {
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                StepResult::Exit(WizardOutcome::Cancelled)
            }
            // Catch typing FIRST so `q` lands in the URL when the
            // input field is active. Outside input mode, the next
            // arm makes `q` a quit shortcut.
            (KeyCode::Char(c), _) if self.input_active => {
                state.upstream_url.push(c);
                StepResult::Continue
            }
            (KeyCode::Char('q'), _) => StepResult::Exit(WizardOutcome::Cancelled),
            (KeyCode::Backspace, _) if self.input_active => {
                state.upstream_url.pop();
                StepResult::Continue
            }
            (KeyCode::Enter, _) => {
                if state.upstream_url.is_empty() {
                    state.error_message = Some("Upstream URL is required".to_string());
                    StepResult::Continue
                } else {
                    state.error_message = None;
                    StepResult::Next(Box::new(ConfirmationStep::new()))
                }
            }
            (KeyCode::Tab, _) => {
                self.input_active = !self.input_active;
                StepResult::Continue
            }
            (KeyCode::Esc, _) => StepResult::Back,
            _ => StepResult::Continue,
        }
    }
}

fn render(frame: &mut Frame, area: Rect, state: &WizardState, input_active: bool) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " Step 3: Repository Configuration ",
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
            Span::styled("🔗 ", Style::default()),
            Span::styled(
                "Configure the upstream Git repository URL",
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(Span::styled(
            "   Used for changelog links and release references",
            Style::default().fg(Color::Gray),
        )),
    ];
    let header = Paragraph::new(header_lines).alignment(Alignment::Center);
    frame.render_widget(header, chunks[0]);

    let content_area = centered_rect(80, 50, chunks[1]);

    let input_border_color = if input_active {
        Color::Yellow
    } else {
        Color::Gray
    };

    let url_display = if state.upstream_url.is_empty() {
        Span::styled(
            "https://github.com/user/repo",
            Style::default().fg(Color::Rgb(80, 80, 80)),
        )
    } else {
        Span::styled(
            state.upstream_url.clone(),
            Style::default()
                .fg(if input_active {
                    Color::Yellow
                } else {
                    Color::White
                })
                .add_modifier(if input_active {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        )
    };

    let cursor = if input_active {
        Span::styled("▌", Style::default().fg(Color::Yellow))
    } else {
        Span::raw("")
    };

    let input_lines = vec![
        Line::from(""),
        Line::from(vec![Span::raw("  "), url_display, cursor]),
        Line::from(""),
    ];

    let input_block = Paragraph::new(input_lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(input_border_color))
            .title(Span::styled(
                if input_active {
                    " ✏️  Editing URL "
                } else {
                    " URL "
                },
                Style::default().fg(if input_active {
                    Color::Yellow
                } else {
                    Color::White
                }),
            )),
    );

    frame.render_widget(input_block, content_area);

    let hints = if input_active {
        Line::from(vec![
            Span::styled("Type", Style::default().fg(Color::Yellow)),
            Span::styled(" to enter URL  ", Style::default().fg(Color::Gray)),
            Span::styled("Backspace", Style::default().fg(Color::Yellow)),
            Span::styled(" delete  ", Style::default().fg(Color::Gray)),
            Span::styled("Tab/Esc", Style::default().fg(Color::Cyan)),
            Span::styled(" finish editing", Style::default().fg(Color::Gray)),
        ])
    } else {
        Line::from(vec![
            Span::styled("Tab", Style::default().fg(Color::Cyan)),
            Span::styled(" edit URL  ", Style::default().fg(Color::Gray)),
            Span::styled("Enter", Style::default().fg(Color::Green)),
            Span::styled(" continue  ", Style::default().fg(Color::Gray)),
            Span::styled("Backspace", Style::default().fg(Color::Yellow)),
            Span::styled(" back  ", Style::default().fg(Color::Gray)),
            Span::styled("q", Style::default().fg(Color::Red)),
            Span::styled(" quit", Style::default().fg(Color::Gray)),
        ])
    };
    let hints_para = Paragraph::new(hints).alignment(Alignment::Center);
    frame.render_widget(hints_para, chunks[2]);
}

#[cfg(test)]
mod tests {
    use super::super::{state::WizardState, step::test_support::render_to_string};
    use super::*;

    #[test]
    fn renders_upstream_with_url() {
        let mut state = WizardState::new(false, None);
        state.upstream_url = "https://github.com/example/repo".into();
        let mut step = UpstreamConfigStep::new();
        let out = render_to_string(&mut step, &state, 80, 24);
        insta::assert_snapshot!("upstream_with_url", out);
    }

    #[test]
    fn renders_upstream_input_active() {
        let mut state = WizardState::new(false, None);
        state.upstream_url = "git@github.com:example/repo.git".into();
        let mut step = UpstreamConfigStep { input_active: true };
        let out = render_to_string(&mut step, &state, 80, 24);
        insta::assert_snapshot!("upstream_input_active", out);
    }
}
