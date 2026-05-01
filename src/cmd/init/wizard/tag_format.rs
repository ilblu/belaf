//! Phase I.3 — single-project tag-format sub-prompt.
//!
//! Shown after [`ProjectSelectionStep`](super::resolved_release_unit::ProjectSelectionStep)
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
use crate::cmd::init::toml_util::toml_quote;

/// Phase I.3 — when the user picked a tag-format override on the
/// single-project tag-format step, build the per-project override
/// snippet that gets appended to `belaf/config.toml`.
///
/// Returns `None` (no append) when:
///   - the user kept the ecosystem default (`tag_format_override == None`)
///   - or somehow finished the wizard without a single selected project
///
/// Both name and format are TOML-escaped via [`toml_quote`] so a
/// project named `foo"bar` can't break out of the table header.
pub fn build_tag_format_snippet(state: &WizardState) -> Option<String> {
    let format = state.tag_format_override.as_ref()?;
    let project = state.selected_units().into_iter().next()?;
    let name_q = toml_quote(&project.name);
    let format_q = toml_quote(format);
    Some(format!("\n[projects.{name_q}]\ntag_format = {format_q}\n",))
}

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
            (KeyCode::Char('c'), KeyModifiers::CONTROL) | (KeyCode::Char('q'), _) => {
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
        .selected_units()
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
        state::{DetectedUnit, WizardState},
        step::test_support::render_to_string,
    };
    use super::*;

    #[test]
    fn renders_tag_format_with_single_project() {
        let mut state = WizardState::new(false, None);
        state.standalone_units = vec![DetectedUnit {
            name: "alpha".into(),
            version: "0.1.0".into(),
            prefix: "crates/alpha".into(),
            selected: true,
            ecosystem: None,
        }];
        let mut step = TagFormatStep::new();
        let out = render_to_string(&mut step, &state, 80, 24);
        insta::assert_snapshot!("tag_format_single_project", out);
    }

    fn state_with_one_project(name: &str, tag_format: Option<&str>) -> WizardState {
        let mut state = WizardState::new(false, None);
        state.standalone_units = vec![DetectedUnit {
            name: name.to_string(),
            version: "1.0.0".to_string(),
            prefix: ".".to_string(),
            selected: true,
            ecosystem: None,
        }];
        state.tag_format_override = tag_format.map(String::from);
        state
    }

    #[test]
    fn build_snippet_returns_none_without_override() {
        let state = state_with_one_project("alpha", None);
        assert!(build_tag_format_snippet(&state).is_none());
    }

    #[test]
    fn build_snippet_emits_valid_toml_for_simple_v_format() {
        let state = state_with_one_project("tokio-like", Some("v{version}"));
        let snippet =
            build_tag_format_snippet(&state).expect("override present must yield snippet");
        // Round-trip through a TOML parser to confirm structural
        // validity AND that the values survive escaping.
        let parsed: toml::Value =
            toml::from_str(&snippet).expect("snippet must be valid TOML, got:\n{snippet}");
        let projects = parsed
            .get("projects")
            .and_then(|v| v.as_table())
            .expect("[projects] table must exist");
        let entry = projects
            .get("tokio-like")
            .and_then(|v| v.as_table())
            .expect("[projects.\"tokio-like\"] must exist");
        assert_eq!(
            entry.get("tag_format").and_then(|v| v.as_str()),
            Some("v{version}"),
            "tag_format must round-trip as the literal v{{version}} template"
        );
    }

    #[test]
    fn build_snippet_escapes_malicious_project_name() {
        // A project name containing a quote must NOT break out of the
        // table header; the snippet must still parse as TOML.
        let state = state_with_one_project(r#"a"b]]"#, Some("v{version}"));
        let snippet = build_tag_format_snippet(&state).expect("must yield snippet");
        let _: toml::Value = toml::from_str(&snippet).unwrap_or_else(|e| {
            panic!("escaped name must still produce valid TOML: {e}\n--- snippet ---\n{snippet}")
        });
    }
}
