//! Phase I.1 — DetectorReviewStep.
//!
//! Shown after the welcome screen when the auto-detector found at
//! least one bundle. Renders every match plus the mobile-warnings as
//! a read-only list and asks the user to accept or skip. Per-item
//! editing is intentionally out of scope: detected bundles are
//! best-effort heuristics, the user can always hand-tune
//! `belaf/config.toml` after bootstrap.

use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

use crate::core::release_unit::detector::{
    DetectorKind, DetectorMatch, HexagonalPrimary, JvmVersionSource, MobilePlatform,
};

use super::{
    preset::PresetSelectionStep,
    project::ProjectSelectionStep,
    state::WizardState,
    step::{Step, StepResult, WizardOutcome},
};

#[derive(Default)]
pub struct DetectorReviewStep;

impl DetectorReviewStep {
    pub fn new() -> Self {
        Self
    }
}

impl Step for DetectorReviewStep {
    fn name(&self) -> &'static str {
        "detector-review"
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
            (KeyCode::Enter | KeyCode::Char('y'), _) => {
                state.detector_accepted = true;
                next_step(state)
            }
            (KeyCode::Char('s' | 'n'), _) => {
                // Skip — accept nothing; subsequent steps proceed
                // without auto-detect snippet append.
                state.detector_accepted = false;
                next_step(state)
            }
            (KeyCode::Esc, _) => StepResult::Back,
            _ => StepResult::Continue,
        }
    }
}

fn next_step(state: &WizardState) -> StepResult {
    if state.preset_from_cli {
        StepResult::Next(Box::new(ProjectSelectionStep::new()))
    } else {
        StepResult::Next(Box::new(PresetSelectionStep::new(state)))
    }
}

fn render(frame: &mut Frame, area: Rect, state: &WizardState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " Auto-detected ReleaseUnits ",
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
            Constraint::Length(3),
            Constraint::Length(2),
        ])
        .split(inner_area);

    let candidate_count = state.detection.count_release_unit_candidates();
    let mobile_count = state
        .detection
        .matches_of(|k| matches!(k, DetectorKind::MobileApp { .. }))
        .len();

    let header_lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("🔍 ", Style::default()),
            Span::styled(
                "belaf detected the following bundles in your repo",
                Style::default().fg(Color::White),
            ),
        ]),
    ];
    let header = Paragraph::new(header_lines).alignment(Alignment::Center);
    frame.render_widget(header, chunks[0]);

    let items: Vec<ListItem> = state.detection.matches.iter().map(format_match).collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Gray))
            .title(Span::styled(
                " Detected bundles ",
                Style::default().fg(Color::White),
            )),
    );
    frame.render_widget(list, chunks[1]);

    let summary_line = Line::from(vec![
        Span::styled(
            format!("{candidate_count}"),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " ReleaseUnit candidate(s)",
            Style::default().fg(Color::White),
        ),
        if mobile_count > 0 {
            Span::styled(
                format!(", {mobile_count} mobile warning(s) → [allow_uncovered]"),
                Style::default().fg(Color::Yellow),
            )
        } else {
            Span::raw("")
        },
    ]);
    let summary_para = Paragraph::new(summary_line).alignment(Alignment::Center);
    frame.render_widget(summary_para, chunks[2]);

    let hints = Line::from(vec![
        Span::styled("Enter/y", Style::default().fg(Color::Green)),
        Span::styled(
            " accept and append to config  ",
            Style::default().fg(Color::Gray),
        ),
        Span::styled("s/n", Style::default().fg(Color::Yellow)),
        Span::styled(" skip  ", Style::default().fg(Color::Gray)),
        Span::styled("Esc", Style::default().fg(Color::Cyan)),
        Span::styled(" back  ", Style::default().fg(Color::Gray)),
        Span::styled("q", Style::default().fg(Color::Red)),
        Span::styled(" quit", Style::default().fg(Color::Gray)),
    ]);
    let hints_para = Paragraph::new(hints).alignment(Alignment::Center);
    frame.render_widget(hints_para, chunks[3]);
}

fn format_match(m: &DetectorMatch) -> ListItem<'static> {
    let (icon, label_color, label) = match &m.kind {
        DetectorKind::HexagonalCargo { primary } => (
            "🦀",
            Color::Green,
            format!("hexagonal cargo ({})", hexagonal_label(*primary)),
        ),
        DetectorKind::Tauri { single_source } => (
            "🖥",
            Color::Green,
            format!(
                "tauri ({})",
                if *single_source {
                    "single-source"
                } else {
                    "legacy multi-file"
                }
            ),
        ),
        DetectorKind::JvmLibrary { version_source } => (
            "☕",
            Color::Green,
            format!("jvm library ({})", jvm_label(version_source)),
        ),
        DetectorKind::MobileApp { platform } => (
            "📱",
            Color::Yellow,
            format!(
                "mobile {} (managed externally)",
                match platform {
                    MobilePlatform::Ios => "iOS",
                    MobilePlatform::Android => "Android",
                }
            ),
        ),
        DetectorKind::NestedNpmWorkspace => ("📦", Color::Cyan, "nested npm workspace".to_string()),
        DetectorKind::SdkCascadeMember => ("🔗", Color::Cyan, "sdk cascade member".to_string()),
    };

    let path = m.path.escaped().to_string();
    let path_display = if path.is_empty() {
        "(repo root)".to_string()
    } else {
        path
    };

    ListItem::new(Line::from(vec![
        Span::styled(format!(" {icon}  "), Style::default()),
        Span::styled(
            format!("{path_display:<32}"),
            Style::default().fg(Color::White),
        ),
        Span::styled(format!("  {label}"), Style::default().fg(label_color)),
    ]))
}

fn hexagonal_label(p: HexagonalPrimary) -> &'static str {
    match p {
        HexagonalPrimary::Bin => "bin",
        HexagonalPrimary::Lib => "lib",
        HexagonalPrimary::Workers => "workers",
        HexagonalPrimary::BaseName => "basename",
    }
}

fn jvm_label(s: &JvmVersionSource) -> &'static str {
    match s {
        JvmVersionSource::GradleProperties => "gradle.properties",
        JvmVersionSource::BuildGradleKtsLiteral => "build.gradle.kts literal",
        JvmVersionSource::PluginManaged => "plugin-managed",
    }
}

#[cfg(test)]
mod tests {
    use crate::core::git::repository::RepoPathBuf;
    use crate::core::release_unit::detector::DetectionReport;

    use super::super::{state::WizardState, step::test_support::render_to_string};
    use super::*;

    fn state_with_detection(matches: Vec<DetectorMatch>) -> WizardState {
        let mut state = WizardState::new(false, None);
        state.detection = DetectionReport { matches };
        state
    }

    #[test]
    fn renders_detector_review_with_mixed_matches() {
        let matches = vec![
            DetectorMatch {
                kind: DetectorKind::HexagonalCargo {
                    primary: HexagonalPrimary::Bin,
                },
                path: RepoPathBuf::new(b"apps/services/foo"),
                note: None,
            },
            DetectorMatch {
                kind: DetectorKind::Tauri {
                    single_source: true,
                },
                path: RepoPathBuf::new(b"apps/desktop"),
                note: None,
            },
            DetectorMatch {
                kind: DetectorKind::JvmLibrary {
                    version_source: JvmVersionSource::GradleProperties,
                },
                path: RepoPathBuf::new(b"sdks/kotlin"),
                note: None,
            },
            DetectorMatch {
                kind: DetectorKind::MobileApp {
                    platform: MobilePlatform::Ios,
                },
                path: RepoPathBuf::new(b"apps/mobile-ios"),
                note: None,
            },
            DetectorMatch {
                kind: DetectorKind::SdkCascadeMember,
                path: RepoPathBuf::new(b"sdks/typescript"),
                note: None,
            },
        ];
        let state = state_with_detection(matches);
        let mut step = DetectorReviewStep::new();
        let out = render_to_string(&mut step, &state, 90, 24);
        insta::assert_snapshot!("detector_review_mixed", out);
    }
}
