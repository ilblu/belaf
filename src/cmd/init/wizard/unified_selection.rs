//! Init-wizard adapter around [`crate::core::ui::release_unit_view`].
//!
//! Owns only:
//! - cursor + initialisation state
//! - key handling (Space/a/n/Enter/Esc)
//! - flush back to `WizardState` on confirm + routing to next step
//!
//! Classification, hint annotations, and rendering live in the
//! shared component — the same code path the prepare and dashboard
//! adapters consume.

use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::core::ui::glyphs;
use crate::core::ui::release_unit_view::{
    render_summary, PrepareOverlay, ReleaseUnitView, RenderMode, RowIdx, StandaloneEntry,
    ViewContext,
};

use super::{
    cascade_from::CascadeFromStep,
    preset::PresetSelectionStep,
    state::WizardState,
    step::{Step, StepResult, WizardOutcome},
    tag_format::TagFormatStep,
    upstream::UpstreamConfigStep,
};

#[derive(Default)]
pub struct UnifiedSelectionStep {
    /// Lazily-built view, regenerated on every initialisation so it
    /// stays in lockstep with state mutations.
    view: ReleaseUnitView,
    /// Sidecar carrying user-confirmed `cascade_from` overrides so
    /// `⇄ source · strategy` badges surface in the selection list.
    overlay: PrepareOverlay,
    /// Cursor index into the flat row order returned by
    /// [`ReleaseUnitView::flat_indices`].
    cursor: usize,
    initialised: bool,
}

impl UnifiedSelectionStep {
    pub fn new() -> Self {
        Self::default()
    }

    fn rebuild(&mut self, state: &WizardState) {
        use crate::core::ui::release_unit_view::CascadeOverrideBadge;

        let standalones: Vec<StandaloneEntry> = state
            .standalone_units
            .iter()
            .map(|u| StandaloneEntry {
                name: u.name.clone(),
                version: u.version.clone(),
                prefix: u.prefix.clone(),
                selected: u.selected,
                ecosystem: u.ecosystem.clone(),
            })
            .collect();
        self.view = ReleaseUnitView::from_detection(
            &state.detection,
            &standalones,
            &state.detector_excluded,
        );
        self.overlay = PrepareOverlay::default();
        for unit in &self.view.units {
            if let Some(ov) = state.cascade_overrides.get(&unit.name) {
                self.overlay.cascade_overrides.insert(
                    unit.backref,
                    CascadeOverrideBadge {
                        source: ov.source.clone(),
                        strategy_label: ov.strategy.as_wire().to_string(),
                    },
                );
            }
        }
        if self.cursor >= self.view.flat_indices().len() {
            self.cursor = 0;
        }
    }

    fn ensure_initialised(&mut self, state: &WizardState) {
        if !self.initialised {
            self.rebuild(state);
            self.initialised = true;
        }
    }

    fn current_idx(&self) -> Option<RowIdx> {
        self.view.flat_indices().get(self.cursor).copied()
    }

    fn flush_to_state(&self, state: &mut WizardState) {
        state.detector_accepted = !state.detection.matches.is_empty();
        state.detector_excluded.clear();
        for path in self.view.excluded_bundle_paths() {
            state.detector_excluded.insert(path);
        }
        for (idx, selected) in self.view.unit_backrefs() {
            if let Some(p) = state.standalone_units.get_mut(idx) {
                p.selected = selected;
            }
        }
    }
}

impl Step for UnifiedSelectionStep {
    fn name(&self) -> &'static str {
        "unified-selection"
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, state: &WizardState) {
        self.ensure_initialised(state);
        render(frame, area, &self.view, &self.overlay, self.cursor);
    }

    fn handle_event(&mut self, event: &Event, state: &mut WizardState) -> StepResult {
        self.ensure_initialised(state);
        let Event::Key(key) = event else {
            return StepResult::Continue;
        };
        let n = self.view.flat_indices().len();

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
                if let Some(idx) = self.current_idx() {
                    self.view.toggle(idx);
                }
                StepResult::Continue
            }
            (KeyCode::Char('a'), _) => {
                self.view.set_all_togglable(true);
                StepResult::Continue
            }
            (KeyCode::Char('n'), _) => {
                self.view.set_all_togglable(false);
                StepResult::Continue
            }
            (KeyCode::Char('c'), _) => {
                // Open the cascade-from sub-step for the unit currently
                // under the cursor. Bundles aggregate manifests under
                // one tag (no per-member cascade) and Ext rows are
                // read-only — both ignore `c`.
                if let Some(RowIdx::Unit(i)) = self.current_idx() {
                    if let Some(unit_row) = self.view.units.get(i) {
                        let target = unit_row.name.clone();
                        return StepResult::Next(Box::new(CascadeFromStep::new(target, state)));
                    }
                }
                StepResult::Continue
            }
            (KeyCode::Enter | KeyCode::Char('y'), _) => {
                let count = self.view.selected_togglable_count();
                if count == 0 {
                    state.error_message =
                        Some("Please select at least one ReleaseUnit".to_string());
                    return StepResult::Continue;
                }
                state.error_message = None;
                self.flush_to_state(state);

                // Routing: --preset on the CLI skips PresetSelection.
                if state.preset_from_cli {
                    if count == 1 {
                        StepResult::Next(Box::new(TagFormatStep::new()))
                    } else {
                        StepResult::Next(Box::new(UpstreamConfigStep::new()))
                    }
                } else {
                    StepResult::Next(Box::new(PresetSelectionStep::new(state)))
                }
            }
            (KeyCode::Esc, _) => StepResult::Back,
            _ => StepResult::Continue,
        }
    }
}

fn render(
    frame: &mut Frame,
    area: Rect,
    view: &ReleaseUnitView,
    overlay: &PrepareOverlay,
    cursor: usize,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " ReleaseUnit Selection ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(inner);

    // Header with summary line.
    let selected = view.selected_togglable_count();
    let total = view.bundles.len() + view.units.len();
    let header = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(
                format!("{} ", glyphs::header_clipboard()),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(
                "Review and toggle each ReleaseUnit you want belaf to manage",
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("   ", Style::default()),
            Span::styled(
                format!("{}", selected),
                Style::default()
                    .fg(if selected > 0 {
                        Color::Green
                    } else {
                        Color::Yellow
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" / {} selected · {}", total, render_summary(view)),
                Style::default().fg(Color::Gray),
            ),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(header).alignment(Alignment::Center),
        chunks[0],
    );

    // Body — delegated to the shared component.
    let ctx = ViewContext {
        mode: RenderMode::Init,
        cursor: Some(cursor),
    };
    view.render_with_overlay(frame, chunks[1], &ctx, overlay);

    // Hints footer.
    let hints = Line::from(vec![
        Span::styled("↑↓", Style::default().fg(Color::Cyan)),
        Span::styled(" navigate  ", Style::default().fg(Color::Gray)),
        Span::styled("Space", Style::default().fg(Color::Cyan)),
        Span::styled(" toggle  ", Style::default().fg(Color::Gray)),
        Span::styled("c", Style::default().fg(Color::Magenta)),
        Span::styled(" cascade-from  ", Style::default().fg(Color::Gray)),
        Span::styled("a/n", Style::default().fg(Color::Green)),
        Span::styled(" all/none  ", Style::default().fg(Color::Gray)),
        Span::styled("Enter", Style::default().fg(Color::Green)),
        Span::styled(" continue  ", Style::default().fg(Color::Gray)),
        Span::styled("Esc", Style::default().fg(Color::Yellow)),
        Span::styled(" back  ", Style::default().fg(Color::Gray)),
        Span::styled("q", Style::default().fg(Color::Red)),
        Span::styled(" quit", Style::default().fg(Color::Gray)),
    ]);
    frame.render_widget(
        Paragraph::new(hints).alignment(Alignment::Center),
        chunks[2],
    );
}

#[cfg(test)]
mod tests {
    use super::super::{
        state::{DetectedUnit, WizardState},
        step::test_support::render_to_string,
    };
    use super::*;
    use crate::core::release_unit::detector::{
        BundleKind, DetectedShape, DetectionReport, DetectorMatch, ExtKind, HintKind,
    };

    fn state_with_mix() -> WizardState {
        let mut state = WizardState::new(false, None);
        state.standalone_units = vec![
            DetectedUnit {
                name: "alpha".into(),
                version: "0.1.0".into(),
                prefix: "crates/alpha".into(),
                selected: true,
                ecosystem: None,
            },
            DetectedUnit {
                name: "beta".into(),
                version: "0.2.3".into(),
                prefix: "crates/beta".into(),
                selected: false,
                ecosystem: None,
            },
        ];
        let mut report = DetectionReport::default();
        report.matches.push(DetectorMatch {
            shape: DetectedShape::Bundle(BundleKind::Tauri {
                single_source: true,
            }),
            path: crate::core::git::repository::RepoPathBuf::new(b"apps/desktop"),
            note: None,
        });
        report.matches.push(DetectorMatch {
            shape: DetectedShape::ExternallyManaged(ExtKind::MobileIos),
            path: crate::core::git::repository::RepoPathBuf::new(b"apps/ios"),
            note: None,
        });
        state.detection = report;
        state
    }

    #[test]
    fn renders_unified_categories() {
        let state = state_with_mix();
        let mut step = UnifiedSelectionStep::new();
        let out = render_to_string(&mut step, &state, 100, 24);
        insta::assert_snapshot!("unified_categories", out);
    }

    #[test]
    fn cursor_navigates_rows() {
        let mut state = state_with_mix();
        let mut step = UnifiedSelectionStep::new();
        step.ensure_initialised(&state);
        // 1 tauri bundle + 2 standalone + 1 mobile = 4 rows
        assert_eq!(step.view.flat_indices().len(), 4);
        // Toggle off the first standalone (alpha was selected=true).
        step.cursor = 1; // first standalone after the bundle
        if let Some(idx) = step.current_idx() {
            step.view.toggle(idx);
        }
        step.flush_to_state(&mut state);
        assert!(!state.standalone_units[0].selected);
    }

    #[test]
    fn tauri_bundle_hides_inner_and_outer_standalones() {
        let mut state = WizardState::new(false, None);
        state.standalone_units = vec![
            DetectedUnit {
                name: "npm:desktop".into(),
                version: "0.1.0".into(),
                prefix: "apps/clients/desktop".into(),
                selected: true,
                ecosystem: Some("npm".into()),
            },
            DetectedUnit {
                name: "cargo:desktop".into(),
                version: "0.0.0".into(),
                prefix: "apps/clients/desktop/src-tauri".into(),
                selected: true,
                ecosystem: Some("cargo".into()),
            },
            DetectedUnit {
                name: "elsewhere".into(),
                version: "1.0.0".into(),
                prefix: "crates/elsewhere".into(),
                selected: true,
                ecosystem: Some("cargo".into()),
            },
        ];
        let mut report = DetectionReport::default();
        report.matches.push(DetectorMatch {
            shape: DetectedShape::Bundle(BundleKind::Tauri {
                single_source: true,
            }),
            path: crate::core::git::repository::RepoPathBuf::new(b"apps/clients/desktop"),
            note: None,
        });
        state.detection = report;

        let mut step = UnifiedSelectionStep::new();
        step.ensure_initialised(&state);

        // Expect: 1 bundle + 1 standalone (`elsewhere`) = 2 rows.
        assert_eq!(step.view.flat_indices().len(), 2);
        assert_eq!(step.view.bundles.len(), 1);
        assert_eq!(step.view.units.len(), 1);
        assert_eq!(step.view.units[0].name, "elsewhere");
    }

    #[test]
    fn hint_annotates_matching_standalone() {
        // SdkCascade hint at sdks/typescript decorates the standalone
        // whose prefix matches.
        use crate::core::ui::release_unit_view::HintAnnotation;

        let mut state = WizardState::new(false, None);
        state.standalone_units = vec![DetectedUnit {
            name: "@org/sdk-ts".into(),
            version: "1.0.0".into(),
            prefix: "sdks/typescript".into(),
            selected: true,
            ecosystem: Some("npm".into()),
        }];
        let mut report = DetectionReport::default();
        report.matches.push(DetectorMatch {
            shape: DetectedShape::Hint(HintKind::SdkCascade),
            path: crate::core::git::repository::RepoPathBuf::new(b"sdks/typescript"),
            note: None,
        });
        state.detection = report;

        let mut step = UnifiedSelectionStep::new();
        step.ensure_initialised(&state);

        assert_eq!(step.view.units.len(), 1);
        assert!(step.view.units[0]
            .annotations
            .contains(&HintAnnotation::SdkCascade));
    }
}
