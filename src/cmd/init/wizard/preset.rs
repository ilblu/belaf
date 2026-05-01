//! Changelog-preset selection step. Owns its cursor and the
//! source-vs-preview toggle panel.

use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Padding, Paragraph, Wrap},
    Frame,
};

use crate::core::ui::components::toggle_panel::TogglePanel;

use super::{
    project::ProjectSelectionStep,
    state::WizardState,
    step::{MouseClick, Step, StepResult, WizardOutcome},
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
                StepResult::Next(Box::new(ProjectSelectionStep::new()))
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

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " Step 1: Changelog Preset ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    let outer_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(inner_area);

    let header_text = vec![Line::from(vec![
        Span::styled("📋 ", Style::default()),
        Span::styled(
            "Choose a changelog format that fits your project",
            Style::default().fg(Color::White),
        ),
    ])];
    let header = Paragraph::new(header_text).alignment(Alignment::Center);
    frame.render_widget(header, outer_chunks[0]);

    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(outer_chunks[1]);

    let items: Vec<ListItem> = state
        .available_presets
        .iter()
        .enumerate()
        .map(|(idx, name)| {
            let (icon, description) = match name.as_str() {
                "default" => ("📦", "Conventional Commits grouped by type"),
                "keepachangelog" => ("📝", "Keep a Changelog specification"),
                "flat" => ("📄", "Simple flat list - What's Changed"),
                "minimal" => ("✨", "Minimal - just version and date"),
                _ => ("📋", "Custom preset"),
            };
            let is_selected = idx == selected_idx;
            let lines = vec![
                Line::from(vec![
                    Span::styled(
                        format!(" {} ", icon),
                        if is_selected {
                            Style::default().fg(Color::Cyan)
                        } else {
                            Style::default()
                        },
                    ),
                    Span::styled(
                        name.clone(),
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
                    format!("    {}", description),
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
            .title(Span::styled(" Presets ", Style::default().fg(Color::White))),
    );

    frame.render_widget(list, main_chunks[0]);

    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(main_chunks[1]);

    let preset_name = state
        .available_presets
        .get(selected_idx)
        .cloned()
        .unwrap_or_else(|| "default".to_string());
    toggle.render(frame, right_chunks[0], &preset_name);

    let config_content = if preset_name == "default" {
        EmbeddedConfig::get_config_string().unwrap_or_else(|_| "Config not available".to_string())
    } else {
        EmbeddedPresets::get_preset_string(&preset_name)
            .unwrap_or_else(|_| "Preset not available".to_string())
    };

    if toggle.is_right() {
        let source_text = config_content
            .lines()
            .take(50)
            .collect::<Vec<_>>()
            .join("\n");

        let paragraph = Paragraph::new(source_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Gray))
                    .title(Span::styled(
                        " TOML Source ",
                        Style::default().fg(Color::Magenta),
                    ))
                    .padding(Padding::horizontal(1)),
            )
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(Color::Rgb(180, 180, 180)));
        frame.render_widget(paragraph, right_chunks[1]);
    } else {
        let example_changelog = generate_preset_example(&preset_name);
        let markdown_text = crate::core::ui::markdown::render_markdown(&example_changelog);

        let paragraph = Paragraph::new(markdown_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Gray))
                    .title(Span::styled(
                        " Changelog Preview ",
                        Style::default().fg(Color::Cyan),
                    ))
                    .padding(Padding::horizontal(1)),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(paragraph, right_chunks[1]);
    }

    let hints = Line::from(vec![
        Span::styled("↑↓", Style::default().fg(Color::Cyan)),
        Span::styled(" select  ", Style::default().fg(Color::Gray)),
        Span::styled("m", Style::default().fg(Color::Cyan)),
        Span::styled(" toggle view  ", Style::default().fg(Color::Gray)),
        Span::styled("Enter", Style::default().fg(Color::Green)),
        Span::styled(" continue  ", Style::default().fg(Color::Gray)),
        Span::styled("q", Style::default().fg(Color::Red)),
        Span::styled(" quit", Style::default().fg(Color::Gray)),
    ]);
    let hints_para = Paragraph::new(hints).alignment(Alignment::Center);
    frame.render_widget(hints_para, outer_chunks[2]);
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
