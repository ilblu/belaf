//! Phase I.1 — DetectorReviewStep.
//!
//! Shown after the welcome screen when the auto-detector found at
//! least one bundle. Users navigate the list with `↑↓ / j k`, toggle
//! individual items with `Space`, accept the (possibly filtered) set
//! with `Enter`, or skip the whole auto-detect with `s` / `n`.
//!
//! Toggled-OFF items don't get a `[[release_unit]]` block AND land in
//! `[ignore_paths]` so the drift detector stays silent on them. Users
//! who want different semantics (`allow_uncovered` for externally-
//! managed releases, or a custom hand-written `[[release_unit]]`)
//! edit `belaf/config.toml` post-init.

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
pub struct DetectorReviewStep {
    /// Index of the currently-highlighted row.
    cursor: usize,
    /// Per-row include flag. `true` = emit a `[[release_unit]]`
    /// for this match. Indexed in lockstep with
    /// `state.detection.matches`. Lazily initialised on first render.
    selected: Vec<bool>,
}

impl DetectorReviewStep {
    pub fn new() -> Self {
        Self::default()
    }

    /// Lazily resize `self.selected` to match the detection report's
    /// length. All matches default to selected (current 2.1.0
    /// "accept all" behaviour). Called from both `render` and
    /// `handle_event` because either could be the first to see the
    /// detection vector.
    fn ensure_initialised(&mut self, n: usize) {
        if self.selected.len() != n {
            self.selected = vec![true; n];
            if self.cursor >= n {
                self.cursor = 0;
            }
        }
    }
}

impl Step for DetectorReviewStep {
    fn name(&self) -> &'static str {
        "detector-review"
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, state: &WizardState) {
        self.ensure_initialised(state.detection.matches.len());
        render(frame, area, state, self.cursor, &self.selected);
    }

    fn handle_event(&mut self, event: &Event, state: &mut WizardState) -> StepResult {
        let Event::Key(key) = event else {
            return StepResult::Continue;
        };
        self.ensure_initialised(state.detection.matches.len());
        let n = self.selected.len();

        match (key.code, key.modifiers) {
            (KeyCode::Char('c'), KeyModifiers::CONTROL) | (KeyCode::Char('q'), _) => {
                StepResult::Exit(WizardOutcome::Cancelled)
            }
            (KeyCode::Down | KeyCode::Char('j'), _) => {
                if n > 0 {
                    self.cursor = (self.cursor + 1) % n;
                }
                StepResult::Continue
            }
            (KeyCode::Up | KeyCode::Char('k'), _) => {
                if n > 0 {
                    self.cursor = if self.cursor == 0 {
                        n - 1
                    } else {
                        self.cursor - 1
                    };
                }
                StepResult::Continue
            }
            (KeyCode::Char(' '), _) => {
                if let Some(slot) = self.selected.get_mut(self.cursor) {
                    *slot = !*slot;
                }
                StepResult::Continue
            }
            (KeyCode::Char('a'), _) => {
                for slot in &mut self.selected {
                    *slot = true;
                }
                StepResult::Continue
            }
            (KeyCode::Char('n'), _) => {
                // 'n' here means "deselect all", not "no/skip" — the
                // skip path is on 's'. The Project step uses the same
                // pairing (a all, n none) so the muscle memory carries.
                for slot in &mut self.selected {
                    *slot = false;
                }
                StepResult::Continue
            }
            (KeyCode::Enter | KeyCode::Char('y'), _) => {
                state.detector_accepted = true;
                state.detector_excluded.clear();
                for (idx, m) in state.detection.matches.iter().enumerate() {
                    if !self.selected.get(idx).copied().unwrap_or(true) {
                        state.detector_excluded.insert(m.path.clone());
                    }
                }
                next_step(state)
            }
            (KeyCode::Char('s'), _) => {
                // Skip the whole auto-detect step — no snippet
                // appended. Ignore the selection vector.
                state.detector_accepted = false;
                state.detector_excluded.clear();
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

fn render(frame: &mut Frame, area: Rect, state: &WizardState, cursor: usize, selected: &[bool]) {
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
    // Count only non-mobile included items so the displayed
    // "X / N selected" matches the candidate denominator (mobile
    // apps are not togglable — they always go to allow_uncovered
    // and are reported separately in the suffix).
    let included = state
        .detection
        .matches
        .iter()
        .enumerate()
        .filter(|(_, m)| !matches!(m.kind, DetectorKind::MobileApp { .. }))
        .filter(|(idx, _)| selected.get(*idx).copied().unwrap_or(true))
        .count();

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

    let items: Vec<ListItem> = state
        .detection
        .matches
        .iter()
        .enumerate()
        .map(|(idx, m)| {
            let is_selected = selected.get(idx).copied().unwrap_or(true);
            let is_cursor = idx == cursor;
            format_match(m, is_selected, is_cursor)
        })
        .collect();

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
            format!("{included}"),
            Style::default()
                .fg(if included > 0 {
                    Color::Green
                } else {
                    Color::Yellow
                })
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" / {candidate_count} ReleaseUnit candidate(s) selected"),
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
        Span::styled("↑↓", Style::default().fg(Color::Cyan)),
        Span::styled(" navigate  ", Style::default().fg(Color::Gray)),
        Span::styled("Space", Style::default().fg(Color::Cyan)),
        Span::styled(" toggle  ", Style::default().fg(Color::Gray)),
        Span::styled("a", Style::default().fg(Color::Green)),
        Span::styled(" all  ", Style::default().fg(Color::Gray)),
        Span::styled("n", Style::default().fg(Color::Yellow)),
        Span::styled(" none  ", Style::default().fg(Color::Gray)),
        Span::styled("Enter", Style::default().fg(Color::Green)),
        Span::styled(" accept  ", Style::default().fg(Color::Gray)),
        Span::styled("s", Style::default().fg(Color::Yellow)),
        Span::styled(" skip  ", Style::default().fg(Color::Gray)),
        Span::styled("Esc", Style::default().fg(Color::Cyan)),
        Span::styled(" back  ", Style::default().fg(Color::Gray)),
        Span::styled("q", Style::default().fg(Color::Red)),
        Span::styled(" quit", Style::default().fg(Color::Gray)),
    ]);
    let hints_para = Paragraph::new(hints).alignment(Alignment::Center);
    frame.render_widget(hints_para, chunks[3]);
}

fn format_match(m: &DetectorMatch, is_selected: bool, is_cursor: bool) -> ListItem<'static> {
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

    // Mobile apps are warnings — they're not configurable as
    // ReleaseUnits, and their selection state has no effect on
    // the emitted snippet (they always go to [allow_uncovered]).
    // Render with a fixed indicator so users don't think they're
    // toggling something useful.
    let is_mobile = matches!(m.kind, DetectorKind::MobileApp { .. });
    let checkbox = if is_mobile {
        "—".to_string()
    } else if is_selected {
        "✅".to_string()
    } else {
        "⬜".to_string()
    };

    let path_style = if is_cursor {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else if is_selected || is_mobile {
        Style::default().fg(Color::White)
    } else {
        Style::default()
            .fg(Color::Rgb(120, 120, 120))
            .add_modifier(Modifier::DIM)
    };

    let row_bg = if is_cursor {
        Style::default().bg(Color::Rgb(40, 40, 50))
    } else {
        Style::default()
    };

    ListItem::new(Line::from(vec![
        Span::styled(format!(" {checkbox} {icon}  "), Style::default()),
        Span::styled(format!("{path_display:<32}"), path_style),
        Span::styled(format!("  {label}"), Style::default().fg(label_color)),
    ]))
    .style(row_bg)
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
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    fn state_with_detection(matches: Vec<DetectorMatch>) -> WizardState {
        let mut state = WizardState::new(false, None);
        state.detection = DetectionReport { matches };
        state
    }

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        })
    }

    fn three_match_state() -> WizardState {
        state_with_detection(vec![
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
                kind: DetectorKind::SdkCascadeMember,
                path: RepoPathBuf::new(b"sdks/typescript"),
                note: None,
            },
        ])
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
        let out = render_to_string(&mut step, &state, 100, 24);
        insta::assert_snapshot!("detector_review_mixed", out);
    }

    #[test]
    fn space_toggles_current_row() {
        let mut state = three_match_state();
        let mut step = DetectorReviewStep::new();
        // Force initialisation by faking a render-tick.
        step.ensure_initialised(state.detection.matches.len());
        assert_eq!(step.selected, vec![true, true, true]);

        // Cursor starts at 0 → toggle row 0
        step.handle_event(&key(KeyCode::Char(' ')), &mut state);
        assert_eq!(step.selected, vec![false, true, true]);

        // Move down + toggle row 1
        step.handle_event(&key(KeyCode::Char('j')), &mut state);
        step.handle_event(&key(KeyCode::Char(' ')), &mut state);
        assert_eq!(step.selected, vec![false, false, true]);
    }

    #[test]
    fn a_selects_all_n_deselects_all() {
        let mut state = three_match_state();
        let mut step = DetectorReviewStep::new();
        step.ensure_initialised(state.detection.matches.len());

        step.handle_event(&key(KeyCode::Char('n')), &mut state);
        assert_eq!(step.selected, vec![false, false, false]);

        step.handle_event(&key(KeyCode::Char('a')), &mut state);
        assert_eq!(step.selected, vec![true, true, true]);
    }

    #[test]
    fn enter_writes_excluded_paths_to_state() {
        let mut state = three_match_state();
        let mut step = DetectorReviewStep::new();
        step.ensure_initialised(state.detection.matches.len());

        // Toggle off rows 0 + 2; row 1 stays included.
        step.handle_event(&key(KeyCode::Char(' ')), &mut state);
        step.handle_event(&key(KeyCode::Char('j')), &mut state);
        step.handle_event(&key(KeyCode::Char('j')), &mut state);
        step.handle_event(&key(KeyCode::Char(' ')), &mut state);

        let result = step.handle_event(&key(KeyCode::Enter), &mut state);
        assert!(matches!(result, StepResult::Next(_)));
        assert!(state.detector_accepted);

        let excluded: std::collections::HashSet<String> = state
            .detector_excluded
            .iter()
            .map(|p| p.escaped().to_string())
            .collect();
        assert!(excluded.contains("apps/services/foo"));
        assert!(excluded.contains("sdks/typescript"));
        assert!(!excluded.contains("apps/desktop"));
    }

    #[test]
    fn skip_clears_excluded_paths() {
        let mut state = three_match_state();
        let mut step = DetectorReviewStep::new();
        step.ensure_initialised(state.detection.matches.len());

        // Mess with the selection set first.
        step.handle_event(&key(KeyCode::Char('n')), &mut state);

        // Then skip — exclusions must be cleared, accepted=false.
        step.handle_event(&key(KeyCode::Char('s')), &mut state);
        assert!(!state.detector_accepted);
        assert!(state.detector_excluded.is_empty());
    }

    #[test]
    fn q_quits_with_cancel_outcome() {
        let mut state = three_match_state();
        let mut step = DetectorReviewStep::new();
        let result = step.handle_event(&key(KeyCode::Char('q')), &mut state);
        match result {
            StepResult::Exit(WizardOutcome::Cancelled) => {}
            _ => panic!("q must exit with Cancelled, got non-cancel result"),
        }
    }
}
