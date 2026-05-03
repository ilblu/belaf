//! Welcome / acknowledgement step. Renders logo + project count +
//! warnings (uncommitted changes, existing config). ENTER starts the
//! flow; in interactive mode pressing ENTER implies `--force` —
//! pinned by `tests/test_dirty_repository.rs`.

use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::{
    chrome::{self, palette, step_index, STEP_TOTAL},
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

    let title = if is_reconfigure {
        "Reconfigure"
    } else {
        "Welcome"
    };
    let body = chrome::render_chrome(frame, area, title, step_index::WELCOME, STEP_TOTAL);
    let (content, hints_area) = chrome::split_body_with_hints(body);

    let body_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7), // logo
            Constraint::Length(1), // tagline
            Constraint::Length(1), // blank
            Constraint::Length(1), // divider
            Constraint::Length(1), // blank
            Constraint::Length(2), // STATUS section
            Constraint::Length(1), // blank
            Constraint::Min(0),    // warnings + flow
        ])
        .split(content);

    let logo_color = if is_reconfigure {
        palette::WARN
    } else {
        palette::ACCENT
    };
    frame.render_widget(
        Paragraph::new(belaf_logo(logo_color)).alignment(Alignment::Center),
        body_chunks[0],
    );

    let tagline = if is_reconfigure {
        Line::from(vec![
            Span::styled(
                "▲ ",
                Style::default()
                    .fg(palette::WARN)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "A configuration already exists — continuing will overwrite it",
                Style::default().fg(palette::WARN),
            ),
        ])
    } else {
        Line::from(Span::styled(
            "Bootstrap your repository for PR-based releases",
            Style::default().fg(palette::SUBTLE),
        ))
    };
    frame.render_widget(
        Paragraph::new(tagline).alignment(Alignment::Center),
        body_chunks[1],
    );

    frame.render_widget(chrome::divider(), body_chunks[3]);

    let unit_count = state.standalone_units.len();
    let bundle_count = state.detection.matches.len();
    let status_lines = vec![
        Line::from(chrome::section_label("STATUS")),
        Line::from(status_spans(unit_count, bundle_count)),
    ];
    frame.render_widget(Paragraph::new(status_lines), body_chunks[5]);

    render_flow_or_warnings(frame, body_chunks[7], state);

    chrome::hint_bar(frame, hints_area, &[("Enter", " continue"), ("q", " quit")]);
}

fn status_spans(units: usize, bundles: usize) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    spans.push(Span::styled(
        format!("{}", units),
        Style::default()
            .fg(palette::VALUE)
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(
        if units == 1 { " project" } else { " projects" },
        Style::default().fg(palette::MUTED),
    ));
    if bundles > 0 {
        spans.push(Span::styled("    ", Style::default()));
        spans.push(Span::styled(
            format!("{}", bundles),
            Style::default()
                .fg(palette::VALUE)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            if bundles == 1 {
                " bundle detected"
            } else {
                " bundles detected"
            },
            Style::default().fg(palette::MUTED),
        ));
    }
    spans
}

fn render_flow_or_warnings(frame: &mut Frame, area: Rect, state: &WizardState) {
    let mut lines: Vec<Line> = Vec::new();

    if let Some(warning) = &state.dirty_warning {
        lines.push(Line::from(vec![
            Span::styled(
                "▲ ",
                Style::default()
                    .fg(palette::WARN)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(warning.clone(), Style::default().fg(palette::WARN)),
        ]));
        lines.push(Line::from(""));
    }

    if let Some(error) = &state.error_message {
        lines.push(Line::from(vec![
            Span::styled(
                "✗ ",
                Style::default()
                    .fg(palette::ERROR)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(error.clone(), Style::default().fg(palette::ERROR)),
        ]));
        lines.push(Line::from(""));
    }

    lines.push(Line::from(chrome::section_label("WHAT'S NEXT")));
    for label in [
        "Select projects and detected bundles",
        "Pick a changelog preset",
        "Confirm the upstream repository URL",
        "Bootstrap belaf/config.toml + baseline tags",
    ] {
        lines.push(Line::from(chrome::action_row(label)));
    }

    frame.render_widget(Paragraph::new(lines), area);
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
        let out = render_to_string(&mut step, &state, 80, 30);
        insta::assert_snapshot!("welcome_first_run", out);
    }

    #[test]
    fn renders_reconfigure_welcome() {
        let mut step = WelcomeStep::new();
        let mut state = fresh_state();
        state.config_exists = true;
        let out = render_to_string(&mut step, &state, 80, 30);
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
        let out = render_to_string(&mut step, &state, 80, 30);
        insta::assert_snapshot!("welcome_dirty_warning", out);
    }
}
