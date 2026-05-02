//! Central rendering + selection model for ReleaseUnit lists.
//!
//! Three wizards used to ship bespoke selection-table code: `init`,
//! `prepare`, and (formerly) `dashboard`. They drifted in subtle ways
//! and the 3.0.x bug class — Hint matches accidentally rendered as
//! Bundle rows — appeared in one wizard at a time.
//!
//! [`ReleaseUnitView`] is the one place that turns
//! `(DetectionReport, &[DetectedUnit])` (or, for prepare, the
//! resolved-unit list) into rows, classified into:
//!
//! - [`BundleRow`] — togglable; emits a `[release_unit.<name>]` block
//!   in the init wizard, listed for bump selection in prepare.
//! - [`UnitRow`] — togglable Standalone; can carry a vector of
//!   [`HintAnnotation`] decorations (sdk-cascade, npm-workspace, …).
//! - [`ExtRow`] — read-only externally-managed (mobile apps); listed
//!   for context but never togglable.
//!
//! Adding a new ReleaseUnit shape = update [`from_detection`] (one
//! match arm). Adding a new render mode = one [`RenderMode`] variant
//! plus a branch in `render`. The wizard adapters never touch
//! classification logic.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem};
use ratatui::Frame;

use super::glyphs;
use crate::core::git::repository::RepoPathBuf;
use crate::core::release_unit::detector::{
    BundleKind, DetectedShape, DetectionReport, ExtKind, HexagonalPrimary, HintKind,
    JvmVersionSource,
};

// ---------------------------------------------------------------------------
// Row data
// ---------------------------------------------------------------------------

/// Multi-manifest auto-detected bundle (Tauri, hexagonal-cargo, JVM
/// library). Togglable; deselected bundles land in
/// `WizardState::detector_excluded` for the init wizard.
#[derive(Clone, Debug)]
pub struct BundleRow {
    pub label: String,
    pub kind_label: String,
    pub ecosystem: String,
    pub path: RepoPathBuf,
    pub selected: bool,
}

/// Single-manifest standalone unit (one entry per ecosystem loader
/// hit). Togglable; carries optional hint annotations that decorate
/// the row's secondary text.
#[derive(Clone, Debug)]
pub struct UnitRow {
    pub name: String,
    pub version: String,
    pub prefix: String,
    pub ecosystem: Option<String>,
    pub annotations: Vec<HintAnnotation>,
    pub selected: bool,
    /// Index back into the original `WizardState::standalone_units`
    /// (init wizard) or the resolved-unit list (prepare). Used to
    /// route toggles back to the right slot.
    pub backref: usize,
}

/// Read-only externally-managed path (iOS / Android app). Auto-added
/// to `[allow_uncovered]` by the init wizard's snippet emission.
#[derive(Clone, Debug)]
pub struct ExtRow {
    pub label: String,
    pub kind_label: String,
    pub ecosystem: String,
    pub path: RepoPathBuf,
}

/// Hint metadata that decorates a Standalone row. Rendered as
/// `↳ <label>` after the row's secondary text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HintAnnotation {
    SdkCascade,
    NpmWorkspace,
    SingleProject,
    NestedMonorepo,
}

impl HintAnnotation {
    pub fn label(&self) -> &'static str {
        match self {
            Self::SdkCascade => "sdk-cascade",
            Self::NpmWorkspace => "npm-workspace",
            Self::SingleProject => "single-project",
            Self::NestedMonorepo => "nested-monorepo",
        }
    }

    fn from_hint(h: &HintKind) -> Self {
        match h {
            HintKind::SdkCascade => Self::SdkCascade,
            HintKind::NpmWorkspace => Self::NpmWorkspace,
            HintKind::SingleProject { .. } => Self::SingleProject,
            HintKind::NestedMonorepo => Self::NestedMonorepo,
        }
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// The classified, render-ready view of a repo's ReleaseUnits.
#[derive(Clone, Debug, Default)]
pub struct ReleaseUnitView {
    pub bundles: Vec<BundleRow>,
    pub units: Vec<UnitRow>,
    pub externally_managed: Vec<ExtRow>,
}

/// Stable index into the view, used to route toggle / cursor events
/// without exposing the internal Vec layout.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RowIdx {
    Bundle(usize),
    Unit(usize),
    Ext(usize),
}

/// Standalone-loader output, matches `init::wizard::state::DetectedUnit`.
/// Re-declared here to keep `release_unit_view` agnostic of the wizard
/// state struct. The `init` adapter does the trivial copy.
#[derive(Clone, Debug)]
pub struct StandaloneEntry {
    pub name: String,
    pub version: String,
    pub prefix: String,
    pub selected: bool,
    pub ecosystem: Option<String>,
}

impl ReleaseUnitView {
    /// Build the view from a [`DetectionReport`] and the loader-emitted
    /// standalone units. This is the *only* place the Bundle/Hint/Ext
    /// classification touches rendering — every consumer (init, prepare,
    /// dashboard) goes through here.
    ///
    /// `detector_excluded` is the set of detector-match paths the user
    /// has previously toggled OFF (init wizard's persistent state).
    pub fn from_detection(
        report: &DetectionReport,
        standalone: &[StandaloneEntry],
        detector_excluded: &std::collections::HashSet<RepoPathBuf>,
    ) -> Self {
        let mut bundles: Vec<BundleRow> = Vec::new();
        let mut bundle_seen: std::collections::HashSet<RepoPathBuf> =
            std::collections::HashSet::new();
        let mut bundle_paths: Vec<String> = Vec::new();
        let mut hints_by_path: std::collections::HashMap<String, Vec<HintAnnotation>> =
            std::collections::HashMap::new();
        let mut ext_rows: Vec<ExtRow> = Vec::new();

        for m in &report.matches {
            match &m.shape {
                DetectedShape::Bundle(b) => {
                    if !bundle_seen.insert(m.path.clone()) {
                        continue;
                    }
                    bundles.push(BundleRow {
                        label: m.path.escaped().to_string(),
                        kind_label: bundle_kind_label(b),
                        ecosystem: ecosystem_for_bundle(b).to_string(),
                        path: m.path.clone(),
                        selected: !detector_excluded.contains(&m.path),
                    });
                    bundle_paths.push(m.path.escaped().to_string());
                }
                DetectedShape::Hint(h) => {
                    let key = m.path.escaped().to_string();
                    hints_by_path
                        .entry(key)
                        .or_default()
                        .push(HintAnnotation::from_hint(h));
                }
                DetectedShape::ExternallyManaged(ext) => {
                    let (kind, eco) = match ext {
                        ExtKind::MobileIos => ("iOS app — auto [allow_uncovered]", "swift"),
                        ExtKind::MobileAndroid => {
                            ("Android app — auto [allow_uncovered]", "kotlin")
                        }
                    };
                    ext_rows.push(ExtRow {
                        label: m.path.escaped().to_string(),
                        kind_label: kind.to_string(),
                        ecosystem: eco.to_string(),
                        path: m.path.clone(),
                    });
                }
            }
        }

        let mut units: Vec<UnitRow> = Vec::new();
        for (idx, p) in standalone.iter().enumerate() {
            // Skip standalones whose path is already covered by a Bundle.
            let covered_by_bundle = bundle_paths
                .iter()
                .any(|bp| p.prefix == *bp || p.prefix.starts_with(&format!("{bp}/")));
            if covered_by_bundle {
                continue;
            }

            // Attach hint annotations whose path matches this unit's
            // prefix (exact or strict prefix). A nested-monorepo hint
            // at `vendor/foo` decorates the standalone whose prefix is
            // `vendor/foo`; an sdk-cascade hint at `sdks/typescript`
            // decorates the npm package found at the same path.
            let mut annotations: Vec<HintAnnotation> = Vec::new();
            for (hint_path, hint_list) in &hints_by_path {
                let matches = p.prefix == *hint_path
                    || p.prefix.starts_with(&format!("{hint_path}/"))
                    || hint_path == "."
                    || (hint_path.is_empty());
                if matches {
                    for ann in hint_list {
                        if !annotations.contains(ann) {
                            annotations.push(ann.clone());
                        }
                    }
                }
            }

            units.push(UnitRow {
                name: p.name.clone(),
                version: p.version.clone(),
                prefix: p.prefix.clone(),
                ecosystem: p.ecosystem.clone(),
                annotations,
                selected: p.selected,
                backref: idx,
            });
        }

        Self {
            bundles,
            units,
            externally_managed: ext_rows,
        }
    }

    /// Total togglable rows (Bundles + Units; Ext is read-only).
    pub fn togglable_len(&self) -> usize {
        self.bundles.len() + self.units.len()
    }

    /// All rows in display order (Bundles → Units → Ext) as `RowIdx`.
    /// Useful for cursor navigation: cursor is an index into the flat
    /// returned list.
    pub fn flat_indices(&self) -> Vec<RowIdx> {
        let mut out = Vec::with_capacity(
            self.bundles.len() + self.units.len() + self.externally_managed.len(),
        );
        for i in 0..self.bundles.len() {
            out.push(RowIdx::Bundle(i));
        }
        for i in 0..self.units.len() {
            out.push(RowIdx::Unit(i));
        }
        for i in 0..self.externally_managed.len() {
            out.push(RowIdx::Ext(i));
        }
        out
    }

    /// Toggle a Bundle or Unit row. Returns `true` if the row was
    /// togglable and its state changed; `false` for Ext rows or
    /// out-of-bounds indices. Mode-checked: in [`RenderMode::Dashboard`]
    /// nothing toggles (caller's responsibility to gate).
    pub fn toggle(&mut self, idx: RowIdx) -> bool {
        match idx {
            RowIdx::Bundle(i) => {
                if let Some(b) = self.bundles.get_mut(i) {
                    b.selected = !b.selected;
                    return true;
                }
            }
            RowIdx::Unit(i) => {
                if let Some(u) = self.units.get_mut(i) {
                    u.selected = !u.selected;
                    return true;
                }
            }
            RowIdx::Ext(_) => {}
        }
        false
    }

    /// Set every Bundle/Unit row's selection in one go.
    pub fn set_all_togglable(&mut self, value: bool) {
        for b in &mut self.bundles {
            b.selected = value;
        }
        for u in &mut self.units {
            u.selected = value;
        }
    }

    /// Count of currently-selected togglable rows.
    pub fn selected_togglable_count(&self) -> usize {
        self.bundles.iter().filter(|b| b.selected).count()
            + self.units.iter().filter(|u| u.selected).count()
    }

    /// Selected unit-row backrefs (indices into the original
    /// standalone-unit list). Init wizard uses this to flush
    /// `selected = true/false` back to `WizardState::standalone_units`.
    pub fn unit_backrefs(&self) -> Vec<(usize, bool)> {
        self.units.iter().map(|u| (u.backref, u.selected)).collect()
    }

    /// Bundle rows that are currently OFF — their paths need to land
    /// in `WizardState::detector_excluded` on confirm.
    pub fn excluded_bundle_paths(&self) -> Vec<RepoPathBuf> {
        self.bundles
            .iter()
            .filter(|b| !b.selected)
            .map(|b| b.path.clone())
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Which wizard is rendering the view. Drives the title / hints /
/// togglability. Mode-specific extras (bump-choice column for
/// `Prepare`) are layered on top by the calling wizard — this module
/// owns only the universal rows.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RenderMode {
    /// `belaf init` — pre-config selection; bundles + standalones are
    /// togglable, mobile is read-only.
    Init,
    /// `belaf prepare` — post-config selection; same togglability,
    /// row text emphasises version-bump candidates.
    Prepare,
    /// `belaf dashboard` — read-only listing; checkboxes hidden.
    Dashboard,
}

/// Optional context the renderer reads. `cursor` highlights one row
/// in the flat order returned by [`ReleaseUnitView::flat_indices`].
pub struct ViewContext {
    pub mode: RenderMode,
    pub cursor: Option<usize>,
}

impl ReleaseUnitView {
    /// Render the view inside `area`. The caller owns the layout
    /// chrome (block / borders / hint bar); this method only fills
    /// the inner content area.
    pub fn render(&self, frame: &mut Frame, area: Rect, ctx: &ViewContext) {
        let mut items: Vec<ListItem> = Vec::new();
        let label_width = self
            .bundles
            .iter()
            .map(|b| b.label.chars().count())
            .chain(self.units.iter().map(|u| u.name.chars().count()))
            .chain(
                self.externally_managed
                    .iter()
                    .map(|e| e.label.chars().count()),
            )
            .max()
            .unwrap_or(0);

        let flat = self.flat_indices();
        let mut last_section: Option<&'static str> = None;

        for (display_idx, row_idx) in flat.iter().enumerate() {
            let section = match row_idx {
                RowIdx::Bundle(_) => "Bundles",
                RowIdx::Unit(_) => "Standalone",
                RowIdx::Ext(_) => "Externally-managed",
            };
            if last_section != Some(section) {
                if last_section.is_some() {
                    items.push(ListItem::new(Line::from("")));
                }
                items.push(section_header(section));
                last_section = Some(section);
            }

            let is_current = ctx.cursor == Some(display_idx);
            let line = self.render_row(*row_idx, ctx.mode, label_width, is_current);
            items.push(line);
        }

        frame.render_widget(List::new(items), area);
    }

    fn render_row(
        &self,
        idx: RowIdx,
        mode: RenderMode,
        label_width: usize,
        is_current: bool,
    ) -> ListItem<'_> {
        let bg = if is_current {
            Style::default().bg(Color::Rgb(40, 40, 50))
        } else {
            Style::default()
        };
        let cursor_color = |selected: bool, locked: bool| {
            if locked {
                Color::DarkGray
            } else if selected {
                Color::Green
            } else {
                Color::DarkGray
            }
        };
        let label_style = |selected: bool, locked: bool| {
            if is_current {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else if locked {
                Style::default().fg(Color::Yellow)
            } else if selected {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::White)
            }
        };

        match idx {
            RowIdx::Bundle(i) => {
                let b = &self.bundles[i];
                let indicator = checkbox_or_lock(mode, b.selected, false);
                let pad = label_width.saturating_sub(b.label.chars().count());
                let padded = format!("{}{}", b.label, " ".repeat(pad));
                ListItem::new(Line::from(vec![
                    Span::styled("    ", Style::default()),
                    Span::styled(
                        format!("{} ", indicator),
                        Style::default().fg(cursor_color(b.selected, false)),
                    ),
                    Span::styled(
                        glyphs::ecosystem(&b.ecosystem),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(padded, label_style(b.selected, false)),
                    Span::styled("  ", Style::default()),
                    Span::styled(b.kind_label.clone(), Style::default().fg(Color::Gray)),
                ]))
                .style(bg)
            }
            RowIdx::Unit(i) => {
                let u = &self.units[i];
                let indicator = checkbox_or_lock(mode, u.selected, false);
                let pad = label_width.saturating_sub(u.name.chars().count());
                let padded = format!("{}{}", u.name, " ".repeat(pad));
                let secondary = format!("@ {} ({})", u.version, u.prefix);
                let mut spans = vec![
                    Span::styled("    ", Style::default()),
                    Span::styled(
                        format!("{} ", indicator),
                        Style::default().fg(cursor_color(u.selected, false)),
                    ),
                    Span::styled(
                        u.ecosystem.as_deref().map(glyphs::ecosystem).unwrap_or(""),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(padded, label_style(u.selected, false)),
                    Span::styled("  ", Style::default()),
                    Span::styled(secondary, Style::default().fg(Color::Gray)),
                ];
                for ann in &u.annotations {
                    spans.push(Span::styled("  ", Style::default()));
                    spans.push(Span::styled(
                        format!("↳ {}", ann.label()),
                        Style::default().fg(Color::Yellow),
                    ));
                }
                ListItem::new(Line::from(spans)).style(bg)
            }
            RowIdx::Ext(i) => {
                let e = &self.externally_managed[i];
                let indicator = checkbox_or_lock(mode, true, true);
                let pad = label_width.saturating_sub(e.label.chars().count());
                let padded = format!("{}{}", e.label, " ".repeat(pad));
                ListItem::new(Line::from(vec![
                    Span::styled("    ", Style::default()),
                    Span::styled(
                        format!("{} ", indicator),
                        Style::default().fg(cursor_color(true, true)),
                    ),
                    Span::styled(
                        glyphs::ecosystem(&e.ecosystem),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(padded, label_style(true, true)),
                    Span::styled("  ", Style::default()),
                    Span::styled(e.kind_label.clone(), Style::default().fg(Color::Gray)),
                ]))
                .style(bg)
            }
        }
    }
}

fn checkbox_or_lock(mode: RenderMode, selected: bool, locked: bool) -> &'static str {
    if locked {
        return glyphs::locked();
    }
    if matches!(mode, RenderMode::Dashboard) {
        return glyphs::checkbox(selected);
    }
    glyphs::checkbox(selected)
}

fn section_header(name: &str) -> ListItem<'static> {
    ListItem::new(Line::from(vec![
        Span::styled(" ", Style::default()),
        Span::styled(
            format!("{}  ", glyphs::category_glyph(name)),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            name.to_string(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    ]))
}

// ---------------------------------------------------------------------------
// Bundle / hint label helpers
// ---------------------------------------------------------------------------

pub fn ecosystem_for_bundle(b: &BundleKind) -> &'static str {
    match b {
        BundleKind::HexagonalCargo { .. } => "hexagonal-cargo",
        BundleKind::Tauri { .. } => "tauri",
        BundleKind::JvmLibrary { .. } => "kotlin",
    }
}

pub fn bundle_kind_label(b: &BundleKind) -> String {
    match b {
        BundleKind::HexagonalCargo { primary } => {
            let p = match primary {
                HexagonalPrimary::Bin => "bin",
                HexagonalPrimary::Lib => "lib",
                HexagonalPrimary::Workers => "workers",
                HexagonalPrimary::BaseName => "basename",
            };
            format!("hexagonal-cargo/{p}")
        }
        BundleKind::Tauri { single_source } => {
            if *single_source {
                "tauri (single-source)".to_string()
            } else {
                "tauri (legacy multi-file)".to_string()
            }
        }
        BundleKind::JvmLibrary { version_source } => {
            let v = match version_source {
                JvmVersionSource::GradleProperties => "gradle.properties",
                JvmVersionSource::BuildGradleKtsLiteral => "build.gradle.kts",
                JvmVersionSource::PluginManaged => "plugin-managed",
            };
            format!("jvm-library/{v}")
        }
    }
}

// ---------------------------------------------------------------------------
// Header summary helper
// ---------------------------------------------------------------------------

/// Compact one-line summary string the calling wizard renders above
/// the unit list. `N units · M hints applied · K externally-managed`.
pub fn render_summary(view: &ReleaseUnitView) -> String {
    let bundles = view.bundles.len();
    let units = view.units.len();
    let ext = view.externally_managed.len();
    let hints: usize = view.units.iter().map(|u| u.annotations.len()).sum();
    let togglable = bundles + units;
    let mut out = format!("{togglable} togglable");
    if hints > 0 {
        out.push_str(&format!(" · {hints} hint annotation(s)"));
    }
    if ext > 0 {
        out.push_str(&format!(" · {ext} externally-managed"));
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::release_unit::detector::{DetectionReport, DetectorMatch};

    fn standalone(name: &str, prefix: &str, eco: &str, selected: bool) -> StandaloneEntry {
        StandaloneEntry {
            name: name.into(),
            version: "0.1.0".into(),
            prefix: prefix.into(),
            selected,
            ecosystem: Some(eco.into()),
        }
    }

    #[test]
    fn classification_three_arms() {
        let mut r = DetectionReport::default();
        r.matches.push(DetectorMatch {
            shape: DetectedShape::Bundle(BundleKind::Tauri {
                single_source: true,
            }),
            path: RepoPathBuf::new(b"apps/desktop"),
            note: None,
        });
        r.matches.push(DetectorMatch {
            shape: DetectedShape::Hint(HintKind::SdkCascade),
            path: RepoPathBuf::new(b"sdks/typescript"),
            note: None,
        });
        r.matches.push(DetectorMatch {
            shape: DetectedShape::ExternallyManaged(ExtKind::MobileIos),
            path: RepoPathBuf::new(b"apps/ios"),
            note: None,
        });
        let standalones = vec![
            standalone("@org/sdk-ts", "sdks/typescript", "npm", true),
            standalone("my-lib", "crates/my-lib", "cargo", true),
        ];
        let view =
            ReleaseUnitView::from_detection(&r, &standalones, &std::collections::HashSet::new());

        assert_eq!(view.bundles.len(), 1);
        assert_eq!(view.bundles[0].label, "apps/desktop");
        assert_eq!(view.units.len(), 2);
        assert_eq!(view.externally_managed.len(), 1);

        // SdkCascade hint annotates @org/sdk-ts (prefix matches).
        let sdk = view.units.iter().find(|u| u.name == "@org/sdk-ts").unwrap();
        assert!(sdk.annotations.contains(&HintAnnotation::SdkCascade));
        // my-lib has no matching hint.
        let mylib = view.units.iter().find(|u| u.name == "my-lib").unwrap();
        assert!(mylib.annotations.is_empty());
    }

    #[test]
    fn bundle_path_hides_inner_standalones() {
        let mut r = DetectionReport::default();
        r.matches.push(DetectorMatch {
            shape: DetectedShape::Bundle(BundleKind::Tauri {
                single_source: true,
            }),
            path: RepoPathBuf::new(b"apps/desktop"),
            note: None,
        });
        let standalones = vec![
            standalone("npm:desktop", "apps/desktop", "npm", true),
            standalone("cargo:desktop", "apps/desktop/src-tauri", "cargo", true),
            standalone("elsewhere", "crates/elsewhere", "cargo", true),
        ];
        let view =
            ReleaseUnitView::from_detection(&r, &standalones, &std::collections::HashSet::new());

        assert_eq!(view.bundles.len(), 1);
        // Only `elsewhere` survives; the two desktop standalones are
        // hidden because their prefix is covered by the Tauri bundle.
        assert_eq!(view.units.len(), 1);
        assert_eq!(view.units[0].name, "elsewhere");
    }

    #[test]
    fn detector_excluded_starts_bundle_off() {
        let mut r = DetectionReport::default();
        r.matches.push(DetectorMatch {
            shape: DetectedShape::Bundle(BundleKind::Tauri {
                single_source: true,
            }),
            path: RepoPathBuf::new(b"apps/desktop"),
            note: None,
        });
        let mut excluded = std::collections::HashSet::new();
        excluded.insert(RepoPathBuf::new(b"apps/desktop"));
        let view = ReleaseUnitView::from_detection(&r, &[], &excluded);
        assert_eq!(view.bundles.len(), 1);
        assert!(!view.bundles[0].selected);
    }

    #[test]
    fn toggle_round_trip() {
        let mut r = DetectionReport::default();
        r.matches.push(DetectorMatch {
            shape: DetectedShape::Bundle(BundleKind::Tauri {
                single_source: true,
            }),
            path: RepoPathBuf::new(b"apps/desktop"),
            note: None,
        });
        let standalones = vec![standalone("a", "crates/a", "cargo", true)];
        let mut view =
            ReleaseUnitView::from_detection(&r, &standalones, &std::collections::HashSet::new());
        assert_eq!(view.selected_togglable_count(), 2);
        assert!(view.toggle(RowIdx::Unit(0)));
        assert_eq!(view.selected_togglable_count(), 1);
        assert!(view.toggle(RowIdx::Bundle(0)));
        assert_eq!(view.selected_togglable_count(), 0);
        // Ext rows are read-only.
        let mut r2 = DetectionReport::default();
        r2.matches.push(DetectorMatch {
            shape: DetectedShape::ExternallyManaged(ExtKind::MobileIos),
            path: RepoPathBuf::new(b"apps/ios"),
            note: None,
        });
        let mut view2 =
            ReleaseUnitView::from_detection(&r2, &[], &std::collections::HashSet::new());
        assert!(!view2.toggle(RowIdx::Ext(0)));
    }

    #[test]
    fn flat_indices_section_order() {
        let mut r = DetectionReport::default();
        r.matches.push(DetectorMatch {
            shape: DetectedShape::Bundle(BundleKind::Tauri {
                single_source: true,
            }),
            path: RepoPathBuf::new(b"apps/desktop"),
            note: None,
        });
        r.matches.push(DetectorMatch {
            shape: DetectedShape::ExternallyManaged(ExtKind::MobileIos),
            path: RepoPathBuf::new(b"apps/ios"),
            note: None,
        });
        let standalones = vec![standalone("a", "crates/a", "cargo", true)];
        let view =
            ReleaseUnitView::from_detection(&r, &standalones, &std::collections::HashSet::new());
        let flat = view.flat_indices();
        assert_eq!(flat.len(), 3);
        assert!(matches!(flat[0], RowIdx::Bundle(0)));
        assert!(matches!(flat[1], RowIdx::Unit(0)));
        assert!(matches!(flat[2], RowIdx::Ext(0)));
    }
}
