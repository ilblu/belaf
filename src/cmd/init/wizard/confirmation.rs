//! Final review screen. ENTER/y confirms (orchestrator runs the
//! bootstrap), n/Esc goes back to upstream config.

use std::collections::BTreeMap;

use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph},
    Frame,
};

use super::{
    chrome::{self, palette, step_index, STEP_TOTAL},
    state::{DetectedUnit, WizardState},
    step::{Step, StepResult, WizardOutcome},
};

pub struct ConfirmationStep {
    list_state: ListState,
}

impl Default for ConfirmationStep {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfirmationStep {
    pub fn new() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self { list_state }
    }
}

impl Step for ConfirmationStep {
    fn name(&self) -> &'static str {
        "confirmation"
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, state: &WizardState) {
        render(frame, area, state, &mut self.list_state);
    }

    fn handle_event(&mut self, event: &Event, state: &mut WizardState) -> StepResult {
        let Event::Key(key) = event else {
            return StepResult::Continue;
        };

        match (key.code, key.modifiers) {
            (KeyCode::Char('c'), KeyModifiers::CONTROL) | (KeyCode::Char('q'), _) => {
                StepResult::Exit(WizardOutcome::Cancelled)
            }
            (KeyCode::Enter | KeyCode::Char('y'), _) => StepResult::Exit(WizardOutcome::Confirmed),
            (KeyCode::Char('n') | KeyCode::Esc, _) => StepResult::Back,
            (KeyCode::Up | KeyCode::Char('k'), _) => {
                let total = state.selected_units().len();
                if total > 0 {
                    let cur = self.list_state.selected().unwrap_or(0);
                    self.list_state.select(Some(cur.saturating_sub(1)));
                }
                StepResult::Continue
            }
            (KeyCode::Down | KeyCode::Char('j'), _) => {
                let total = state.selected_units().len();
                if total > 0 {
                    let cur = self.list_state.selected().unwrap_or(0);
                    let next = (cur + 1).min(total - 1);
                    self.list_state.select(Some(next));
                }
                StepResult::Continue
            }
            (KeyCode::Home | KeyCode::Char('g'), _) => {
                self.list_state.select(Some(0));
                StepResult::Continue
            }
            (KeyCode::End | KeyCode::Char('G'), _) => {
                let total = state.selected_units().len();
                if total > 0 {
                    self.list_state.select(Some(total - 1));
                }
                StepResult::Continue
            }
            _ => StepResult::Continue,
        }
    }
}

fn render(frame: &mut Frame, area: Rect, state: &WizardState, list_state: &mut ListState) {
    let body = chrome::render_chrome(
        frame,
        area,
        "Confirmation",
        step_index::CONFIRMATION,
        STEP_TOTAL,
    );
    let (content, hints_area) = chrome::split_body_with_hints(body);

    let selected = state.selected_units();

    let body_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // repo section: label + value
            Constraint::Length(1), // blank
            Constraint::Length(1), // divider
            Constraint::Length(1), // blank
            Constraint::Length(2), // projects label + stats
            Constraint::Length(1), // blank
            Constraint::Min(3),    // project list
            Constraint::Length(1), // blank
            Constraint::Length(1), // divider
            Constraint::Length(1), // blank
            Constraint::Length(5), // actions section
        ])
        .split(content);

    // ─ REPOSITORY ─
    let repo_lines = vec![
        Line::from(chrome::section_label("REPOSITORY")),
        Line::from(Span::styled(
            state.upstream_url.clone(),
            Style::default()
                .fg(palette::VALUE)
                .add_modifier(Modifier::BOLD),
        )),
    ];
    frame.render_widget(Paragraph::new(repo_lines), body_chunks[0]);

    frame.render_widget(chrome::divider(), body_chunks[2]);

    // ─ PROJECTS ─
    let stats = ecosystem_stats(&selected);
    let projects_header = vec![
        Line::from(vec![
            chrome::section_label("PROJECTS"),
            Span::raw("  "),
            Span::styled(
                format!("{}", selected.len()),
                Style::default()
                    .fg(palette::VALUE)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" total", Style::default().fg(palette::MUTED)),
        ]),
        Line::from(stats_spans(&stats)),
    ];
    frame.render_widget(Paragraph::new(projects_header), body_chunks[4]);

    let pill_width = stats.keys().map(|k| k.len()).max().unwrap_or(4).max(4);
    let name_width = selected
        .iter()
        .map(|u| u.name.chars().count())
        .max()
        .unwrap_or(20)
        .min(40);
    let items: Vec<ListItem> = selected
        .iter()
        .map(|u| project_row(u, name_width, pill_width))
        .collect();

    let list = List::new(items).highlight_style(Style::default().bg(palette::ROW_HIGHLIGHT));
    frame.render_stateful_widget(list, body_chunks[6], list_state);

    frame.render_widget(chrome::divider(), body_chunks[8]);

    // ─ ON CONFIRM ─
    let action_lines = vec![
        Line::from(chrome::section_label("ON CONFIRM")),
        Line::from(chrome::action_row("Write belaf/config.toml")),
        Line::from(chrome::action_row(&format!(
            "Update version files in {} project{}",
            selected.len(),
            if selected.len() == 1 { "" } else { "s" }
        ))),
        Line::from(chrome::action_row(
            "Create baseline tags for all release units",
        )),
    ];
    frame.render_widget(Paragraph::new(action_lines), body_chunks[10]);

    chrome::hint_bar(
        frame,
        hints_area,
        &[
            ("↑↓", " scroll"),
            ("Enter", " confirm"),
            ("Esc", " back"),
            ("q", " quit"),
        ],
    );
}

fn ecosystem_stats(selected: &[&DetectedUnit]) -> BTreeMap<String, usize> {
    let mut out: BTreeMap<String, usize> = BTreeMap::new();
    for u in selected {
        let key = u.ecosystem.clone().unwrap_or_else(|| "other".to_string());
        *out.entry(key).or_insert(0) += 1;
    }
    out
}

fn stats_spans(stats: &BTreeMap<String, usize>) -> Vec<Span<'static>> {
    if stats.is_empty() {
        return vec![Span::styled(
            "no ecosystem detected",
            Style::default().fg(palette::MUTED),
        )];
    }
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (i, (eco, count)) in stats.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("   ", Style::default().fg(palette::MUTED)));
        }
        spans.push(Span::styled(
            format!("{} ", eco),
            Style::default().fg(chrome::ecosystem_color(eco)),
        ));
        spans.push(Span::styled(
            format!("{}", count),
            Style::default()
                .fg(palette::VALUE)
                .add_modifier(Modifier::BOLD),
        ));
    }
    spans
}

fn project_row(u: &DetectedUnit, name_width: usize, pill_width: usize) -> ListItem<'static> {
    let eco = u.ecosystem.as_deref().unwrap_or("other");
    let eco_color = chrome::ecosystem_color(eco);

    let display_name = if u.name.chars().count() > name_width {
        let mut s: String = u.name.chars().take(name_width.saturating_sub(1)).collect();
        s.push('…');
        s
    } else {
        let pad = name_width - u.name.chars().count();
        format!("{}{}", u.name, " ".repeat(pad))
    };

    let pill = format!("{:<width$}", eco, width = pill_width);

    ListItem::new(Line::from(vec![
        Span::raw("  "),
        Span::styled(display_name, Style::default().fg(palette::VALUE)),
        Span::raw("   "),
        Span::styled(pill, Style::default().fg(eco_color)),
        Span::raw("   "),
        Span::styled(u.version.clone(), Style::default().fg(palette::SUBTLE)),
    ]))
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
                ecosystem: Some("cargo".into()),
            },
            DetectedUnit {
                name: "beta".into(),
                version: "0.2.3".into(),
                prefix: "crates/beta".into(),
                selected: true,
                ecosystem: Some("cargo".into()),
            },
        ];
        let mut step = ConfirmationStep::new();
        let out = render_to_string(&mut step, &state, 100, 24);
        insta::assert_snapshot!("confirmation_two_projects", out);
    }
}
