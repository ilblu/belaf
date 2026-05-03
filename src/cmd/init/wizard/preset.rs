//! Changelog-preset selection step. Owns its cursor and the
//! source-vs-preview toggle panel.

use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::core::ui::components::toggle_panel::TogglePanel;

use super::{
    chrome::{self, palette, step_index, STEP_TOTAL},
    state::WizardState,
    step::{MouseClick, Step, StepResult, WizardOutcome},
    upstream::UpstreamConfigStep,
};

pub struct PresetSelectionStep {
    selected_idx: usize,
    toggle: TogglePanel,
}

impl PresetSelectionStep {
    pub fn new(state: &WizardState) -> Self {
        let selected_idx = state
            .preset
            .as_ref()
            .and_then(|p| state.available_presets.iter().position(|x| x == p))
            .unwrap_or(0);

        Self {
            selected_idx,
            toggle: TogglePanel::default(),
        }
    }

    fn selected_name<'a>(&self, state: &'a WizardState) -> &'a str {
        state
            .available_presets
            .get(self.selected_idx)
            .map(|s| s.as_str())
            .unwrap_or("default")
    }
}

impl Step for PresetSelectionStep {
    fn name(&self) -> &'static str {
        "preset"
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, state: &WizardState) {
        render(frame, area, state, self.selected_idx, &mut self.toggle);
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
                if !state.available_presets.is_empty() {
                    self.selected_idx = (self.selected_idx + 1) % state.available_presets.len();
                }
                StepResult::Continue
            }
            (KeyCode::Up | KeyCode::Char('k'), _) => {
                if !state.available_presets.is_empty() {
                    self.selected_idx = if self.selected_idx == 0 {
                        state.available_presets.len() - 1
                    } else {
                        self.selected_idx - 1
                    };
                }
                StepResult::Continue
            }
            (KeyCode::Char('m'), _) => {
                self.toggle.toggle();
                StepResult::Continue
            }
            (KeyCode::Enter, _) => {
                let selected = self.selected_name(state).to_string();
                state.preset = if selected == "default" {
                    None
                } else {
                    Some(selected)
                };
                // Unit selection happened upstream in
                // UnifiedSelectionStep — route straight to upstream
                // config.
                StepResult::Next(Box::new(UpstreamConfigStep::new()))
            }
            (KeyCode::Esc, _) => StepResult::Back,
            _ => StepResult::Continue,
        }
    }

    fn handle_click(&mut self, click: &MouseClick, _state: &mut WizardState) -> Option<StepResult> {
        if self.toggle.handle_click(click.column, click.row) {
            Some(StepResult::Continue)
        } else {
            None
        }
    }
}

fn render(
    frame: &mut Frame,
    area: Rect,
    state: &WizardState,
    selected_idx: usize,
    toggle: &mut TogglePanel,
) {
    use crate::core::embed::{EmbeddedConfig, EmbeddedPresets};

    let body = chrome::render_chrome(
        frame,
        area,
        "Changelog Preset",
        step_index::PRESET,
        STEP_TOTAL,
    );
    let (content, hints_area) = chrome::split_body_with_hints(body);

    // Body sections.
    let body_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // intro line
            Constraint::Length(1), // blank
            Constraint::Length(1), // divider
            Constraint::Length(1), // blank
            Constraint::Min(0),    // two-column body
        ])
        .split(content);

    let intro = Paragraph::new(Line::from(Span::styled(
        "Choose a changelog format that fits your project",
        Style::default().fg(palette::VALUE),
    )));
    frame.render_widget(intro, body_chunks[0]);

    frame.render_widget(chrome::divider(), body_chunks[2]);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
        .split(body_chunks[4]);

    // ─ left: PRESETS list ─
    render_presets_column(frame, cols[0], state, selected_idx);

    // ─ right: PREVIEW with toggle ─
    let preset_name = state
        .available_presets
        .get(selected_idx)
        .cloned()
        .unwrap_or_else(|| "default".to_string());
    let config_content = if preset_name == "default" {
        EmbeddedConfig::get_config_string().unwrap_or_else(|_| "Config not available".to_string())
    } else {
        EmbeddedPresets::get_preset_string(&preset_name)
            .unwrap_or_else(|_| "Preset not available".to_string())
    };
    render_preview_column(frame, cols[1], &preset_name, &config_content, toggle);

    chrome::hint_bar(
        frame,
        hints_area,
        &[
            ("↑↓", " select"),
            ("m", " toggle preview"),
            ("Enter", " continue"),
            ("Esc", " back"),
            ("q", " quit"),
        ],
    );
}

fn render_presets_column(frame: &mut Frame, area: Rect, state: &WizardState, selected_idx: usize) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(0)])
        .split(area);

    let header = Paragraph::new(vec![
        Line::from(chrome::section_label("PRESETS")),
        Line::from(Span::styled(
            format!("{} available", state.available_presets.len()),
            Style::default().fg(palette::MUTED),
        )),
    ]);
    frame.render_widget(header, chunks[0]);

    let items: Vec<ListItem> = state
        .available_presets
        .iter()
        .enumerate()
        .map(|(idx, name)| {
            let is_selected = idx == selected_idx;
            let description = preset_description(name);
            let arrow = if is_selected {
                Span::styled(
                    "▸ ",
                    Style::default()
                        .fg(palette::ACCENT)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled("  ", Style::default())
            };
            let name_style = if is_selected {
                Style::default()
                    .fg(palette::ACCENT)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette::VALUE)
            };
            let lines = vec![
                Line::from(vec![arrow, Span::styled(name.clone(), name_style)]),
                Line::from(Span::styled(
                    format!("    {}", description),
                    Style::default().fg(palette::SUBTLE),
                )),
                Line::from(""),
            ];
            let style = if is_selected {
                Style::default().bg(palette::ROW_HIGHLIGHT)
            } else {
                Style::default()
            };
            ListItem::new(lines).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, chunks[1]);
}

fn render_preview_column(
    frame: &mut Frame,
    area: Rect,
    preset_name: &str,
    config_content: &str,
    toggle: &mut TogglePanel,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // section header
            Constraint::Length(3), // toggle bar
            Constraint::Min(0),    // preview body
        ])
        .split(area);

    let header = Paragraph::new(vec![
        Line::from(chrome::section_label("PREVIEW")),
        Line::from(Span::styled(
            format!("{} → CHANGELOG.md", preset_name),
            Style::default().fg(palette::MUTED),
        )),
    ]);
    frame.render_widget(header, chunks[0]);

    toggle.render(frame, chunks[1], preset_name);

    if toggle.is_right() {
        let source_text = config_content
            .lines()
            .take(60)
            .collect::<Vec<_>>()
            .join("\n");
        let paragraph = Paragraph::new(source_text)
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(palette::SUBTLE));
        frame.render_widget(paragraph, chunks[2]);
    } else {
        let example_changelog = generate_preset_example(preset_name);
        let markdown_text = crate::core::ui::markdown::render_markdown(&example_changelog);
        let paragraph = Paragraph::new(markdown_text).wrap(Wrap { trim: false });
        frame.render_widget(paragraph, chunks[2]);
    }
}

fn preset_description(name: &str) -> &'static str {
    match name {
        "default" => "Conventional Commits grouped by type",
        "keepachangelog" => "Keep a Changelog specification",
        "flat" => "Simple flat list — What's Changed",
        "minimal" => "Minimal — version and date only",
        _ => "Custom preset",
    }
}

fn generate_preset_example(preset_name: &str) -> String {
    match preset_name {
        "default" => r#"## [1.2.0] - 2025-01-15

### ✨ Features

- **cli:** Add `--verbose` flag for detailed output
- **api:** Implement rate limiting for requests

### 🐛 Bug Fixes

- **auth:** Resolve token refresh race condition
- **parser:** Handle unicode characters in paths

### ⚡ Performance

- Optimize database queries

### 📚 Documentation

- Update installation guide
"#
        .to_string(),
        "keepachangelog" => r#"## [1.2.0] - 2025-01-15

### Added

- Add `--verbose` flag for detailed output
- Implement rate limiting for requests

### Fixed

- Resolve token refresh race condition
- Handle unicode characters in paths

### Changed

- Optimize database queries
- Update installation guide
"#
        .to_string(),
        "flat" => r#"## What's Changed

* Add `--verbose` flag for detailed output by @nyxb
* Implement rate limiting for requests by @nyxb
* Resolve token refresh race condition by @nyxb
* Handle unicode characters in paths by @nyxb
* Optimize database queries by @nyxb
* Update installation guide by @nyxb

**Full Changelog**: https://github.com/user/repo/compare/v1.1.0...v1.2.0
"#
        .to_string(),
        "minimal" => r#"## 1.2.0 (2025-01-15)

- Add `--verbose` flag for detailed output
- Implement rate limiting for requests
- Resolve token refresh race condition
- Handle unicode characters in paths
- Optimize database queries
- Update installation guide
"#
        .to_string(),
        _ => "Preview not available for this preset.".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::{state::WizardState, step::test_support::render_to_string};
    use super::*;

    #[test]
    fn renders_preset_step_with_default_selected() {
        let state = WizardState::new(false, None);
        let mut step = PresetSelectionStep::new(&state);
        let out = render_to_string(&mut step, &state, 100, 30);
        insta::assert_snapshot!("preset_default_selected", out);
    }
}
