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
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::core::release_unit::detector::{DetectedShape, ExtKind};

use super::{
    chrome::{self, palette},
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
    // Stand-alone screen — no progress dots.
    let body = chrome::render_chrome(frame, area, "Mobile-Only Repo Detected", 0, 0);
    let (content, hints_area) = chrome::split_body_with_hints(body);

    let body_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // intro
            Constraint::Length(1), // blank
            Constraint::Length(1), // divider
            Constraint::Length(1), // blank
            Constraint::Min(0),    // body
        ])
        .split(content);

    let intro = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(
                "▲ ",
                Style::default()
                    .fg(palette::WARN)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "This repo only contains mobile-app bundles",
                Style::default().fg(palette::WARN),
            ),
        ]),
        Line::from(Span::styled(
            "belaf manages source releases — not signed mobile binaries",
            Style::default().fg(palette::SUBTLE),
        )),
    ]);
    frame.render_widget(intro, body_chunks[0]);

    frame.render_widget(chrome::divider(), body_chunks[2]);

    let mut body = vec![Line::from(chrome::section_label("DETECTED"))];
    for m in &state.detection.matches {
        if let DetectedShape::ExternallyManaged(ext) = &m.shape {
            let label = match ext {
                ExtKind::MobileIos => "iOS",
                ExtKind::MobileAndroid => "Android",
                ExtKind::JvmPluginManaged => "JVM (plugin-managed)",
            };
            body.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("• ", Style::default().fg(palette::WARN)),
                Span::styled(
                    m.path.escaped().to_string(),
                    Style::default().fg(palette::VALUE),
                ),
                Span::styled(format!("  ({label})"), Style::default().fg(palette::MUTED)),
            ]));
        }
    }
    body.push(Line::from(""));
    body.push(Line::from(chrome::section_label("WHAT BELAF DOES")));
    body.push(Line::from(chrome::action_row(
        "Changelog generation, semver bumps, multi-package release coordination",
    )));
    body.push(Line::from(""));
    body.push(Line::from(chrome::section_label("WHAT BELAF DOESN'T DO")));
    body.push(Line::from(vec![
        Span::raw("  "),
        Span::styled("✗ ", Style::default().fg(palette::ERROR)),
        Span::styled(
            "Sign, build, or upload mobile binaries",
            Style::default().fg(palette::VALUE),
        ),
    ]));
    body.push(Line::from(""));
    body.push(Line::from(chrome::section_label(
        "RECOMMENDED ALTERNATIVES",
    )));
    for (name, desc) in [
        ("Bitrise", "hosted CI tailored to iOS / Android"),
        (
            "fastlane",
            "local automation: signing, screenshots, store upload",
        ),
        ("Codemagic", "Flutter-friendly hosted CI"),
    ] {
        body.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "▸ ",
                Style::default()
                    .fg(palette::ACTION)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                name.to_string(),
                Style::default()
                    .fg(palette::OK)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  · ", Style::default().fg(palette::MUTED)),
            Span::styled(desc.to_string(), Style::default().fg(palette::SUBTLE)),
        ]));
    }

    frame.render_widget(Paragraph::new(body), body_chunks[4]);

    chrome::hint_bar(
        frame,
        hints_area,
        &[("Enter", " exit with suggestion"), ("q", " cancel quietly")],
    );
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
                    shape: DetectedShape::ExternallyManaged(ExtKind::MobileIos),
                    path: RepoPathBuf::new(b"ios"),
                    note: None,
                },
                DetectorMatch {
                    shape: DetectedShape::ExternallyManaged(ExtKind::MobileAndroid),
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
