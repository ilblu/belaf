//! Upstream-URL configuration step.

use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::{
    chrome::{self, palette, step_index, STEP_TOTAL},
    confirmation::ConfirmationStep,
    state::WizardState,
    step::{Step, StepResult, WizardOutcome},
};

#[derive(Default)]
pub struct UpstreamConfigStep;

impl UpstreamConfigStep {
    pub fn new() -> Self {
        Self
    }
}

impl Step for UpstreamConfigStep {
    fn name(&self) -> &'static str {
        "upstream"
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, state: &WizardState) {
        render(frame, area, state);
    }

    fn handle_event(&mut self, event: &Event, state: &mut WizardState) -> StepResult {
        let Event::Key(key) = event else {
            return StepResult::Continue;
        };

        match (key.code, key.modifiers) {
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                StepResult::Exit(WizardOutcome::Cancelled)
            }
            (KeyCode::Esc, _) => StepResult::Back,
            (KeyCode::Enter, _) => {
                if state.upstream_url.trim().is_empty() {
                    state.error_message = Some("Upstream URL is required".to_string());
                    StepResult::Continue
                } else {
                    state.error_message = None;
                    StepResult::Next(Box::new(ConfirmationStep::new()))
                }
            }
            (KeyCode::Backspace, _) => {
                state.upstream_url.pop();
                state.error_message = None;
                StepResult::Continue
            }
            (KeyCode::Char(c), _) => {
                state.upstream_url.push(c);
                state.error_message = None;
                StepResult::Continue
            }
            _ => StepResult::Continue,
        }
    }
}

fn render(frame: &mut Frame, area: Rect, state: &WizardState) {
    let body = chrome::render_chrome(
        frame,
        area,
        "Repository URL",
        step_index::UPSTREAM,
        STEP_TOTAL,
    );
    let (content, hints_area) = chrome::split_body_with_hints(body);

    let body_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // intro
            Constraint::Length(1), // blank
            Constraint::Length(1), // divider
            Constraint::Length(1), // blank
            Constraint::Length(2), // URL section header
            Constraint::Length(1), // blank
            Constraint::Length(2), // URL input line + underline
            Constraint::Length(1), // blank
            Constraint::Length(1), // status (error or hint)
            Constraint::Min(0),    // filler
        ])
        .split(content);

    let intro = Paragraph::new(Line::from(Span::styled(
        "Used for changelog links and release references",
        Style::default().fg(palette::VALUE),
    )));
    frame.render_widget(intro, body_chunks[0]);

    frame.render_widget(chrome::divider(), body_chunks[2]);

    let url_header = Paragraph::new(vec![
        Line::from(chrome::section_label("URL")),
        Line::from(Span::styled(
            "Enter the upstream Git remote — typing edits in place",
            Style::default().fg(palette::MUTED),
        )),
    ]);
    frame.render_widget(url_header, body_chunks[4]);

    render_input(frame, body_chunks[6], &state.upstream_url);

    if let Some(err) = &state.error_message {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    "✗ ",
                    Style::default()
                        .fg(palette::ERROR)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(err.clone(), Style::default().fg(palette::ERROR)),
            ])),
            body_chunks[8],
        );
    } else {
        let hint_text = if state.upstream_url.is_empty() {
            "Examples: git@github.com:owner/repo.git  ·  https://github.com/owner/repo"
        } else {
            ""
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::raw("  "),
                Span::styled(hint_text.to_string(), Style::default().fg(palette::MUTED)),
            ])),
            body_chunks[8],
        );
    }

    chrome::hint_bar(
        frame,
        hints_area,
        &[
            ("type", " edit"),
            ("Backspace", " delete"),
            ("Enter", " continue"),
            ("Esc", " back"),
        ],
    );
}

fn render_input(frame: &mut Frame, area: Rect, value: &str) {
    let input_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    let display = if value.is_empty() {
        vec![
            Span::raw("  "),
            Span::styled(
                "https://github.com/owner/repo",
                Style::default().fg(palette::PLACEHOLDER),
            ),
            Span::styled(
                "▌",
                Style::default()
                    .fg(palette::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
        ]
    } else {
        vec![
            Span::raw("  "),
            Span::styled(
                value.to_string(),
                Style::default()
                    .fg(palette::VALUE)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "▌",
                Style::default()
                    .fg(palette::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
        ]
    };
    frame.render_widget(Paragraph::new(Line::from(display)), input_chunks[0]);

    // Underline below the input — visual focus indicator without a box.
    let underline_width = (area.width as usize).saturating_sub(4);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "─".repeat(underline_width),
                Style::default().fg(palette::ACCENT),
            ),
        ])),
        input_chunks[1],
    );
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
        let mut step = UpstreamConfigStep::new();
        let out = render_to_string(&mut step, &state, 80, 24);
        insta::assert_snapshot!("upstream_input_active", out);
    }
}
