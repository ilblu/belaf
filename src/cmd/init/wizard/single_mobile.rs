//! Phase I.4 — single-mobile-repo exit suggestion.
//!
//! Pushed by [`WelcomeStep`](super::welcome::WelcomeStep) when the
//! detector reports that the repo contains only mobile-app bundles
//! and nothing belaf can manage. Belaf doesn't ship release plumbing
//! for `.xcodeproj` / `Info.plist` / `gradle versionCode-versionName`
//! — those are owned by Bitrise / fastlane / Codemagic. We surface
//! the alternative so the user doesn't end up bootstrapping an empty
//! config.

use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::core::release_unit::detector::{DetectorKind, MobilePlatform};

use super::{
    state::WizardState,
    step::{Step, StepResult, WizardOutcome},
};

const SUGGESTION_MESSAGE: &str = "belaf doesn't manage iOS / Android app releases. \
Use Bitrise, fastlane, or Codemagic to ship the binary; \
then add belaf to a sibling repo if you want changelog automation.";

#[derive(Default)]
pub struct SingleMobileStep;

impl SingleMobileStep {
    pub fn new() -> Self {
        Self
    }
}

impl Step for SingleMobileStep {
    fn name(&self) -> &'static str {
        "single-mobile"
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
            (KeyCode::Enter, _) => StepResult::Exit(WizardOutcome::SuggestedAlternative(
                SUGGESTION_MESSAGE.to_string(),
            )),
            _ => StepResult::Continue,
        }
    }
}

fn render(frame: &mut Frame, area: Rect, state: &WizardState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(Span::styled(
            " ⚠ Mobile-only repo detected ",
            Style::default()
                .fg(Color::Yellow)
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
            Span::styled("📱 ", Style::default()),
            Span::styled(
                "Detected only mobile-app bundles in this repo",
                Style::default().fg(Color::White),
            ),
        ]),
    ];
    let header = Paragraph::new(header_lines).alignment(Alignment::Center);
    frame.render_widget(header, chunks[0]);

    let mut body = vec![Line::from("")];
    for m in &state.detection.matches {
        if let DetectorKind::MobileApp { platform } = &m.kind {
            let label = match platform {
                MobilePlatform::Ios => "iOS",
                MobilePlatform::Android => "Android",
            };
            body.push(Line::from(vec![
                Span::styled("   • ", Style::default().fg(Color::Yellow)),
                Span::styled(
                    m.path.escaped().to_string(),
                    Style::default().fg(Color::White),
                ),
                Span::styled(format!("  ({label})"), Style::default().fg(Color::Gray)),
            ]));
        }
    }
    body.push(Line::from(""));
    body.push(Line::from(""));
    body.push(Line::from(Span::styled(
        "What belaf does:",
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )));
    body.push(Line::from(Span::styled(
        "  - changelog generation, semver bumps, multi-package release coordination",
        Style::default().fg(Color::Gray),
    )));
    body.push(Line::from(""));
    body.push(Line::from(Span::styled(
        "What belaf doesn't do:",
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )));
    body.push(Line::from(Span::styled(
        "  - sign / build / upload mobile binaries",
        Style::default().fg(Color::Gray),
    )));
    body.push(Line::from(""));
    body.push(Line::from(Span::styled(
        "Recommended alternatives for mobile releases:",
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )));
    body.push(Line::from(vec![
        Span::styled("  → ", Style::default().fg(Color::Cyan)),
        Span::styled("Bitrise", Style::default().fg(Color::Green)),
        Span::styled(
            "   — hosted CI tailored to iOS / Android",
            Style::default().fg(Color::Gray),
        ),
    ]));
    body.push(Line::from(vec![
        Span::styled("  → ", Style::default().fg(Color::Cyan)),
        Span::styled("fastlane", Style::default().fg(Color::Green)),
        Span::styled(
            "  — local automation: signing, screenshots, store upload",
            Style::default().fg(Color::Gray),
        ),
    ]));
    body.push(Line::from(vec![
        Span::styled("  → ", Style::default().fg(Color::Cyan)),
        Span::styled("Codemagic", Style::default().fg(Color::Green)),
        Span::styled(
            " — Flutter-friendly hosted CI",
            Style::default().fg(Color::Gray),
        ),
    ]));

    let body_para = Paragraph::new(body).alignment(Alignment::Left);
    frame.render_widget(body_para, chunks[1]);

    let hints = Line::from(vec![
        Span::styled("Enter", Style::default().fg(Color::Yellow)),
        Span::styled(" exit with suggestion  ", Style::default().fg(Color::Gray)),
        Span::styled("q", Style::default().fg(Color::Red)),
        Span::styled(" cancel quietly", Style::default().fg(Color::Gray)),
    ]);
    let hints_para = Paragraph::new(hints).alignment(Alignment::Center);
    frame.render_widget(hints_para, chunks[2]);
}

#[cfg(test)]
mod tests {
    use crate::core::git::repository::RepoPathBuf;
    use crate::core::release_unit::detector::{DetectionReport, DetectorMatch};

    use super::super::{state::WizardState, step::test_support::render_to_string};
    use super::*;

    #[test]
    fn renders_single_mobile_warning() {
        let mut state = WizardState::new(false, None);
        state.detection = DetectionReport {
            matches: vec![
                DetectorMatch {
                    kind: DetectorKind::MobileApp {
                        platform: MobilePlatform::Ios,
                    },
                    path: RepoPathBuf::new(b"ios"),
                    note: None,
                },
                DetectorMatch {
                    kind: DetectorKind::MobileApp {
                        platform: MobilePlatform::Android,
                    },
                    path: RepoPathBuf::new(b"android"),
                    note: None,
                },
            ],
        };
        let mut step = SingleMobileStep::new();
        let out = render_to_string(&mut step, &state, 90, 30);
        insta::assert_snapshot!("single_mobile_warning", out);
    }
}
