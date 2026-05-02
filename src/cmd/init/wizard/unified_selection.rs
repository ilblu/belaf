//! Unified ReleaseUnit selection step.
//!
//! One screen with three categories — auto-detected bundles, manual
//! standalone projects, externally-managed mobile apps — and one
//! Space-toggle interaction model:
//!
//!   🔍 Bundles               (multi-manifest auto-detected units)
//!   📦 Standalone            (single-manifest projects from loaders)
//!   📱 Externally-managed    (mobile apps; auto-allow_uncovered, read-only)
//!
//! Enter confirms; toggled-off Bundle items land in
//! `state.detector_excluded` (so they get neither a `[[release_unit]]`
//! block nor drift firing on them). Toggled-off Standalone items have
//! their `selected` field set to false.

use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

use crate::core::release_unit::detector::{DetectorKind, MobilePlatform};

use super::{
    preset::PresetSelectionStep,
    state::WizardState,
    step::{Step, StepResult, WizardOutcome},
    tag_format::TagFormatStep,
    upstream::UpstreamConfigStep,
};

/// Category a row belongs to. Drives the indicator emoji + which
/// underlying state slot the toggle writes through.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum RowCategory {
    Bundle,
    Standalone,
    ExternallyManaged,
}

/// One row in the unified view. Holds enough info to render and to
/// route the toggle action back to the right state field.
#[derive(Clone, Debug)]
struct Row {
    category: RowCategory,
    /// Display label shown to the user.
    label: String,
    /// Secondary text rendered next to the label (version / platform / kind).
    secondary: String,
    /// True if the row is currently selected (will be emitted /
    /// included on confirm). False rows go to detector_excluded
    /// (Bundle), or have `selected = false` (Standalone). Mobile
    /// (ExternallyManaged) is not togglable.
    selected: bool,
    /// Backref index — used on confirm to mutate the right
    /// `state.detection.matches[i]` or `state.standalone_units[i]`. Mobile
    /// rows do not need a backref because they're read-only.
    backref: BackRef,
    /// Loader ecosystem (`cargo`, `npm`, `pypa`, …) — drives the
    /// per-row glyph in `BELAF_ICONS=nerd` mode. Empty in the
    /// default Unicode mode (no ecosystem column rendered).
    ecosystem: Option<String>,
}

#[derive(Copy, Clone, Debug)]
enum BackRef {
    /// Index into `state.detection.matches`.
    Detection(usize),
    /// Index into `state.standalone_units`.
    Standalone(usize),
    /// Mobile app — no backref needed, lands in [allow_uncovered]
    /// automatically via auto_detect snippet emission.
    Mobile,
}

#[derive(Default)]
pub struct UnifiedSelectionStep {
    /// Lazily-built rows mirror, regenerated on every render so it
    /// stays in lockstep with state mutations.
    rows: Vec<Row>,
    /// Cursor index into rows.
    cursor: usize,
    initialised: bool,
}

impl UnifiedSelectionStep {
    pub fn new() -> Self {
        Self::default()
    }

    fn rebuild_rows(&mut self, state: &WizardState) {
        self.rows.clear();

        // Bundles: every detector match except mobile.  Per path we
        // keep only the first hit — the detector emission order in
        // `detect_all` runs higher-specificity scanners (hexagonal /
        // tauri / jvm-library) before the broader `sdk_cascade_member`,
        // so the first hit per path is automatically the most useful
        // label for the user.  Without this dedup `sdks/kotlin`
        // appears twice (jvm-library + sdk-cascade-member) for the
        // same physical bundle.
        let mut bundle_seen: std::collections::HashSet<crate::core::git::repository::RepoPathBuf> =
            std::collections::HashSet::new();
        // Paths covered by an actual bundle — used below to filter the
        // standalone list so we don't render the inner manifests of a
        // bundle as separate units.  `SingleProject` / `NestedMonorepo`
        // hits are informational only (they describe the repo shape,
        // not a multi-manifest bundle) so we don't add them here.
        let mut bundle_paths: Vec<String> = Vec::new();
        for (idx, m) in state.detection.matches.iter().enumerate() {
            if matches!(m.kind, DetectorKind::MobileApp { .. }) {
                continue;
            }
            if !bundle_seen.insert(m.path.clone()) {
                continue;
            }
            let kind_label = detector_kind_label(&m.kind);
            let ecosystem = ecosystem_for_detector_kind(&m.kind);
            self.rows.push(Row {
                category: RowCategory::Bundle,
                label: m.path.escaped().to_string(),
                secondary: kind_label,
                selected: !state.detector_excluded.contains(&m.path),
                backref: BackRef::Detection(idx),
                ecosystem,
            });
            if !matches!(
                m.kind,
                DetectorKind::SingleProject { .. } | DetectorKind::NestedMonorepo
            ) {
                bundle_paths.push(m.path.escaped().to_string());
            }
        }

        // Standalone: manual project list.  Skip units whose path is
        // already covered by a Bundle — e.g. for a Tauri triplet at
        // `apps/clients/desktop/` the loader picks up both the outer
        // `package.json` (npm:desktop) and inner `src-tauri/Cargo.toml`
        // (cargo:desktop), which would otherwise show up as duplicates
        // of the Tauri Bundle row.
        for (idx, p) in state.standalone_units.iter().enumerate() {
            let covered_by_bundle = bundle_paths
                .iter()
                .any(|bp| p.prefix == *bp || p.prefix.starts_with(&format!("{bp}/")));
            if covered_by_bundle {
                continue;
            }
            self.rows.push(Row {
                category: RowCategory::Standalone,
                label: p.name.clone(),
                secondary: format!("@ {} ({})", p.version, p.prefix),
                selected: p.selected,
                backref: BackRef::Standalone(idx),
                ecosystem: p.ecosystem.clone(),
            });
        }

        // Externally managed: mobile apps.
        for m in state.detection.matches.iter() {
            if let DetectorKind::MobileApp { platform } = m.kind {
                let plat = match platform {
                    MobilePlatform::Ios => "iOS",
                    MobilePlatform::Android => "Android",
                };
                self.rows.push(Row {
                    category: RowCategory::ExternallyManaged,
                    label: m.path.escaped().to_string(),
                    secondary: format!("{plat} app — auto [allow_uncovered]"),
                    // Mobile rows are always "selected" in the visual
                    // sense (they go to allow_uncovered) but the user
                    // can't toggle them off — that lives in config
                    // post-init.
                    selected: true,
                    backref: BackRef::Mobile,
                    ecosystem: Some(match platform {
                        MobilePlatform::Ios => "swift".to_string(),
                        MobilePlatform::Android => "kotlin".to_string(),
                    }),
                });
            }
        }

        if self.cursor >= self.rows.len() {
            self.cursor = 0;
        }
    }

    fn ensure_initialised(&mut self, state: &WizardState) {
        if !self.initialised {
            self.rebuild_rows(state);
            self.initialised = true;
        }
    }

    fn toggle_current(&mut self) {
        let Some(row) = self.rows.get_mut(self.cursor) else {
            return;
        };
        if matches!(row.category, RowCategory::ExternallyManaged) {
            return; // Mobile is read-only.
        }
        row.selected = !row.selected;
    }

    fn select_all_togglable(&mut self) {
        for row in &mut self.rows {
            if !matches!(row.category, RowCategory::ExternallyManaged) {
                row.selected = true;
            }
        }
    }

    fn deselect_all_togglable(&mut self) {
        for row in &mut self.rows {
            if !matches!(row.category, RowCategory::ExternallyManaged) {
                row.selected = false;
            }
        }
    }

    /// Apply the row selection vector back to WizardState — call on
    /// Enter before returning the routing StepResult.
    fn flush_to_state(&self, state: &mut WizardState) {
        state.detector_accepted = !state.detection.matches.is_empty();
        state.detector_excluded.clear();

        for row in &self.rows {
            match (row.category, row.backref) {
                (RowCategory::Bundle, BackRef::Detection(idx)) => {
                    if !row.selected {
                        if let Some(m) = state.detection.matches.get(idx) {
                            state.detector_excluded.insert(m.path.clone());
                        }
                    }
                }
                (RowCategory::Standalone, BackRef::Standalone(idx)) => {
                    if let Some(p) = state.standalone_units.get_mut(idx) {
                        p.selected = row.selected;
                    }
                }
                (RowCategory::ExternallyManaged, _) => { /* mobile: snippet handles allow_uncovered */
                }
                _ => { /* mismatched — skip silently */ }
            }
        }
    }

    /// Count togglable rows currently selected (excludes mobile).
    fn selected_togglable_count(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| !matches!(r.category, RowCategory::ExternallyManaged))
            .filter(|r| r.selected)
            .count()
    }
}

impl Step for UnifiedSelectionStep {
    fn name(&self) -> &'static str {
        "unified-selection"
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, state: &WizardState) {
        self.ensure_initialised(state);
        render(frame, area, &self.rows, self.cursor);
    }

    fn handle_event(&mut self, event: &Event, state: &mut WizardState) -> StepResult {
        self.ensure_initialised(state);
        let Event::Key(key) = event else {
            return StepResult::Continue;
        };
        let n = self.rows.len();

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
                self.toggle_current();
                StepResult::Continue
            }
            (KeyCode::Char('a'), _) => {
                self.select_all_togglable();
                StepResult::Continue
            }
            (KeyCode::Char('n'), _) => {
                self.deselect_all_togglable();
                StepResult::Continue
            }
            (KeyCode::Enter | KeyCode::Char('y'), _) => {
                let count = self.selected_togglable_count();
                if count == 0 {
                    state.error_message =
                        Some("Please select at least one ReleaseUnit".to_string());
                    return StepResult::Continue;
                }
                state.error_message = None;
                self.flush_to_state(state);

                // Routing rule: if the user passed --preset on the CLI
                // we skip PresetSelection and head straight to
                // tag-format (single) or upstream-config (multi).
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

/// Map a detector match to the ecosystem identifier used by the
/// glyph module. Returns `None` when the kind doesn't pin an
/// ecosystem (e.g. nested-monorepo).
fn ecosystem_for_detector_kind(kind: &DetectorKind) -> Option<String> {
    use crate::core::release_unit::detector::SingleProjectEcosystem;
    Some(match kind {
        DetectorKind::HexagonalCargo { .. } => "hexagonal-cargo".to_string(),
        DetectorKind::Tauri { .. } => "tauri".to_string(),
        DetectorKind::JvmLibrary { .. } => "kotlin".to_string(),
        DetectorKind::MobileApp { .. } => return None,
        DetectorKind::NestedNpmWorkspace => "npm".to_string(),
        DetectorKind::SdkCascadeMember => "cascade".to_string(),
        DetectorKind::SingleProject { ecosystem } => match ecosystem {
            SingleProjectEcosystem::Cargo => "cargo".to_string(),
            SingleProjectEcosystem::Npm => "npm".to_string(),
            SingleProjectEcosystem::Pypa => "pypa".to_string(),
            SingleProjectEcosystem::Go => "go".to_string(),
            SingleProjectEcosystem::Maven => "maven".to_string(),
            SingleProjectEcosystem::Swift => "swift".to_string(),
            SingleProjectEcosystem::Elixir => "elixir".to_string(),
        },
        DetectorKind::NestedMonorepo => return None,
    })
}

fn detector_kind_label(kind: &DetectorKind) -> String {
    use crate::core::release_unit::detector::{HexagonalPrimary, JvmVersionSource};
    match kind {
        DetectorKind::HexagonalCargo { primary } => {
            let p = match primary {
                HexagonalPrimary::Bin => "bin",
                HexagonalPrimary::Lib => "lib",
                HexagonalPrimary::Workers => "workers",
                HexagonalPrimary::BaseName => "basename",
            };
            format!("hexagonal-cargo/{p}")
        }
        DetectorKind::Tauri { single_source } => {
            if *single_source {
                "tauri (single-source)".to_string()
            } else {
                "tauri (legacy multi-file)".to_string()
            }
        }
        DetectorKind::JvmLibrary { version_source } => {
            let v = match version_source {
                JvmVersionSource::GradleProperties => "gradle.properties",
                JvmVersionSource::BuildGradleKtsLiteral => "build.gradle.kts",
                JvmVersionSource::PluginManaged => "plugin-managed",
            };
            format!("jvm-library/{v}")
        }
        DetectorKind::MobileApp { .. } => "mobile-app".to_string(),
        DetectorKind::NestedNpmWorkspace => "nested-npm-workspace".to_string(),
        DetectorKind::SdkCascadeMember => "sdk-cascade-member".to_string(),
        DetectorKind::SingleProject { ecosystem } => {
            format!("single-project/{ecosystem}")
        }
        DetectorKind::NestedMonorepo => "nested-monorepo".to_string(),
    }
}

fn render(frame: &mut Frame, area: Rect, rows: &[Row], cursor: usize) {
    use super::glyphs;

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " ReleaseUnit Selection ",
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

    // Header.
    let total_togglable = rows
        .iter()
        .filter(|r| !matches!(r.category, RowCategory::ExternallyManaged))
        .count();
    let selected_togglable = rows
        .iter()
        .filter(|r| !matches!(r.category, RowCategory::ExternallyManaged))
        .filter(|r| r.selected)
        .count();
    let mobile_count = rows
        .iter()
        .filter(|r| matches!(r.category, RowCategory::ExternallyManaged))
        .count();

    let mut header = vec![
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
                format!("{}", selected_togglable),
                Style::default()
                    .fg(if selected_togglable > 0 {
                        Color::Green
                    } else {
                        Color::Yellow
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" / {} selected", total_togglable),
                Style::default().fg(Color::Gray),
            ),
        ]),
    ];
    if mobile_count > 0 {
        let suffix = format!(" • {} mobile-app(s) → [allow_uncovered]", mobile_count);
        header.push(Line::from(Span::styled(
            suffix,
            Style::default().fg(Color::Yellow),
        )));
    }
    frame.render_widget(
        Paragraph::new(header).alignment(Alignment::Center),
        chunks[0],
    );

    // Compute the longest visible label per category so the secondary
    // column lines up across rows.  Width is in *display columns* —
    // we approximate via UnicodeSegmentation graphemes.  Good enough
    // for the ASCII paths/names we render here.
    let label_width = rows
        .iter()
        .map(|r| r.label.chars().count())
        .max()
        .unwrap_or(0);

    let mut items: Vec<ListItem> = Vec::with_capacity(rows.len() + 8);
    let mut last_cat: Option<RowCategory> = None;
    for (idx, row) in rows.iter().enumerate() {
        if last_cat != Some(row.category) {
            // Insert a blank visual separator before every category
            // header except the first — gives the eye a clear break
            // between Bundles / Standalone / Externally-managed.
            if last_cat.is_some() {
                items.push(ListItem::new(Line::from("")));
            }
            items.push(ListItem::new(Line::from(vec![
                Span::styled(" ", Style::default()),
                Span::styled(
                    format!("{}  ", glyphs::category_glyph(category_name(row.category))),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    category_name(row.category).to_string(),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
            ])));
            last_cat = Some(row.category);
        }

        let is_current = idx == cursor;
        let indicator = match row.category {
            RowCategory::ExternallyManaged => glyphs::locked(),
            _ => glyphs::checkbox(row.selected),
        };
        let indicator_color = match row.category {
            RowCategory::ExternallyManaged => Color::DarkGray,
            _ => {
                if row.selected {
                    Color::Green
                } else {
                    Color::DarkGray
                }
            }
        };

        let label_style = if is_current {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else if row.selected && !matches!(row.category, RowCategory::ExternallyManaged) {
            Style::default().fg(Color::Green)
        } else if matches!(row.category, RowCategory::ExternallyManaged) {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::White)
        };

        let bg = if is_current {
            Style::default().bg(Color::Rgb(40, 40, 50))
        } else {
            Style::default()
        };

        // Build the row spans.  Indent + checkbox + ecosystem (nerd
        // mode only) + padded label + secondary.
        let eco_glyph = row
            .ecosystem
            .as_deref()
            .map(glyphs::ecosystem)
            .unwrap_or("");
        let pad = label_width.saturating_sub(row.label.chars().count());
        let padded_label = format!("{}{}", row.label, " ".repeat(pad));

        items.push(
            ListItem::new(Line::from(vec![
                Span::styled("    ", Style::default()),
                Span::styled(
                    format!("{} ", indicator),
                    Style::default().fg(indicator_color),
                ),
                Span::styled(eco_glyph, Style::default().fg(Color::DarkGray)),
                Span::styled(padded_label, label_style),
                Span::styled("  ", Style::default()),
                Span::styled(row.secondary.clone(), Style::default().fg(Color::Gray)),
            ]))
            .style(bg),
        );
    }
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Gray)),
    );
    frame.render_widget(list, chunks[1]);

    // Hints.
    let hints = Line::from(vec![
        Span::styled("↑↓", Style::default().fg(Color::Cyan)),
        Span::styled(" navigate  ", Style::default().fg(Color::Gray)),
        Span::styled("Space", Style::default().fg(Color::Cyan)),
        Span::styled(" toggle  ", Style::default().fg(Color::Gray)),
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

fn category_name(c: RowCategory) -> &'static str {
    match c {
        RowCategory::Bundle => "Bundles",
        RowCategory::Standalone => "Standalone",
        RowCategory::ExternallyManaged => "Externally-managed",
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        state::{DetectedUnit, WizardState},
        step::test_support::render_to_string,
    };
    use super::*;
    use crate::core::release_unit::detector::{DetectionReport, DetectorKind, DetectorMatch};

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
            kind: DetectorKind::Tauri {
                single_source: true,
            },
            path: crate::core::git::repository::RepoPathBuf::new(b"apps/desktop"),
            note: None,
        });
        report.matches.push(DetectorMatch {
            kind: DetectorKind::MobileApp {
                platform: MobilePlatform::Ios,
            },
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
        assert_eq!(step.rows.len(), 4);
        step.cursor = 1;
        assert!(matches!(
            step.rows[step.cursor].category,
            RowCategory::Standalone
        ));
        // Toggle off: alpha (was selected=true) goes to false.
        step.toggle_current();
        assert!(!step.rows[1].selected);
        step.flush_to_state(&mut state);
        assert!(!state.standalone_units[0].selected);
    }

    /// Regression for the "Tauri triplet shows up three times" bug
    /// reported against 3.0.1: the bundle hit at `apps/clients/desktop`
    /// covers both the outer `package.json` (npm:desktop) and the
    /// inner `src-tauri/Cargo.toml` (cargo:desktop), so neither
    /// should appear in the Standalone section.
    #[test]
    fn tauri_bundle_hides_inner_and_outer_standalones() {
        let mut state = WizardState::new(false, None);
        // Standalone units the loaders would normally pick up:
        //   * npm:desktop  → outer package.json at apps/clients/desktop
        //   * cargo:desktop → src-tauri/Cargo.toml at apps/clients/desktop/src-tauri
        // Plus an unrelated standalone that must NOT be hidden.
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
            kind: DetectorKind::Tauri {
                single_source: true,
            },
            path: crate::core::git::repository::RepoPathBuf::new(b"apps/clients/desktop"),
            note: None,
        });
        state.detection = report;

        let mut step = UnifiedSelectionStep::new();
        step.ensure_initialised(&state);

        // Expect: 1 Tauri Bundle + 1 Standalone (`elsewhere`) = 2 rows.
        // The two desktop standalones should be filtered out.
        assert_eq!(step.rows.len(), 2);
        assert!(matches!(step.rows[0].category, RowCategory::Bundle));
        assert_eq!(step.rows[0].label, "apps/clients/desktop");
        assert!(matches!(step.rows[1].category, RowCategory::Standalone));
        assert_eq!(step.rows[1].label, "elsewhere");
    }

    /// Regression for the "sdks/kotlin appears twice in Bundles" bug:
    /// jvm_library + sdk_cascade_member both hit the same path. The
    /// wizard should only show one row per path; emission order in
    /// `detect_all` puts jvm_library first, so that's the label kept.
    #[test]
    fn same_path_bundles_dedup_keeping_first_emission() {
        use crate::core::release_unit::detector::JvmVersionSource;

        let mut state = WizardState::new(false, None);
        let mut report = DetectionReport::default();
        let kotlin_path = crate::core::git::repository::RepoPathBuf::new(b"sdks/kotlin");
        // jvm_library emits first in detect_all.
        report.matches.push(DetectorMatch {
            kind: DetectorKind::JvmLibrary {
                version_source: JvmVersionSource::GradleProperties,
            },
            path: kotlin_path.clone(),
            note: None,
        });
        // sdk_cascade_member emits later for the same path.
        report.matches.push(DetectorMatch {
            kind: DetectorKind::SdkCascadeMember,
            path: kotlin_path.clone(),
            note: None,
        });
        state.detection = report;

        let mut step = UnifiedSelectionStep::new();
        step.ensure_initialised(&state);

        let bundle_rows: Vec<&Row> = step
            .rows
            .iter()
            .filter(|r| matches!(r.category, RowCategory::Bundle))
            .collect();
        assert_eq!(
            bundle_rows.len(),
            1,
            "expected exactly one bundle row for sdks/kotlin"
        );
        assert!(
            bundle_rows[0].secondary.contains("jvm-library"),
            "expected the jvm-library label to win; got: {}",
            bundle_rows[0].secondary
        );
    }

    #[test]
    fn mobile_row_not_togglable() {
        let mut state = state_with_mix();
        let mut step = UnifiedSelectionStep::new();
        step.ensure_initialised(&state);
        // Last row is the mobile one.
        let mobile_idx = step.rows.len() - 1;
        assert!(matches!(
            step.rows[mobile_idx].category,
            RowCategory::ExternallyManaged
        ));
        let was = step.rows[mobile_idx].selected;
        step.cursor = mobile_idx;
        step.toggle_current();
        assert_eq!(
            step.rows[mobile_idx].selected, was,
            "mobile must stay locked"
        );
        step.flush_to_state(&mut state);
    }

    #[test]
    fn flush_writes_excluded_paths_for_off_bundles() {
        let mut state = state_with_mix();
        let mut step = UnifiedSelectionStep::new();
        step.ensure_initialised(&state);
        // Toggle the tauri bundle off.
        assert!(matches!(step.rows[0].category, RowCategory::Bundle));
        step.cursor = 0;
        step.toggle_current();
        step.flush_to_state(&mut state);
        assert!(state
            .detector_excluded
            .iter()
            .any(|p| p.escaped() == "apps/desktop"));
    }
}
