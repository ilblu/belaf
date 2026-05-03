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

use std::collections::HashMap;

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
///
/// Prepare-specific data (bump-hints, commit counts, cascade-override
/// badges) lives in [`PrepareOverlay`] keyed by `backref`, not on the
/// row itself — the view holds the universal shape, the overlay
/// holds prepare's modal extras.
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
    /// route toggles back to the right slot and to look up
    /// prepare-overlay entries.
    pub backref: usize,
    /// Group id when this unit is part of a `[group.<id>]` block.
    /// Drives [`ViewLayout::Grouped`] rendering: units with `Some(_)`
    /// are folded into their group's tree; units with `None` render
    /// as solo rows.
    pub group_id: Option<String>,
}

/// Visual badge for a wizard-confirmed cascade-from rule. Mirrors
/// `cmd::init::wizard::state::CascadeOverride` without taking a
/// dependency on the wizard module (the view stays UI-pure).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CascadeOverrideBadge {
    pub source: String,
    pub strategy_label: String,
}

/// Bump recommendation label rendered after the unit's secondary
/// text in [`RenderMode::Prepare`]. Mirrors `core::bump::BumpRecommendation`
/// without taking a dependency on it (the view stays pure UI).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BumpHint {
    Major,
    Minor,
    Patch,
    None,
}

impl BumpHint {
    /// `(label, ratatui::style::Color)`.
    pub fn label_and_color(self) -> (&'static str, Color) {
        match self {
            Self::Major => ("MAJOR", Color::Red),
            Self::Minor => ("MINOR", Color::Yellow),
            Self::Patch => ("PATCH", Color::Green),
            Self::None => ("", Color::Gray),
        }
    }
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
///
/// `groups` is populated only by [`Self::from_resolved`] (prepare's
/// post-resolve constructor). [`Self::from_detection`] leaves it
/// empty — init has no group rows in its selection step.
#[derive(Clone, Debug, Default)]
pub struct ReleaseUnitView {
    pub bundles: Vec<BundleRow>,
    pub units: Vec<UnitRow>,
    pub externally_managed: Vec<ExtRow>,
    pub groups: Vec<GroupRowDisplay>,
}

/// Stable index into the view, used to route toggle / cursor events
/// without exposing the internal Vec layout.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RowIdx {
    Bundle(usize),
    Unit(usize),
    Group(usize),
    Ext(usize),
}

/// Selects which row order the view renders.
///
/// - [`ViewLayout::Sectioned`] — bundles → units → ext. The init and
///   dashboard wizards use this; `groups` are ignored.
/// - [`ViewLayout::Grouped`] — bundles → groups (with their members
///   embedded as a tree under the header) → solo units (those with
///   `group_id == None`) → ext. Prepare uses this.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ViewLayout {
    Sectioned,
    Grouped,
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

/// One resolved release unit as prepare sees it: the unit data plus
/// the bump-pipeline outputs (suggested bump, commit count) that
/// decorate the row in [`RenderMode::Prepare`].
///
/// Re-declared here to keep `release_unit_view` agnostic of the
/// prepare wizard's private types — the prepare adapter does the
/// trivial copy from `ReleaseUnitItem` (or any equivalent) into
/// `ResolvedEntry` before calling [`ReleaseUnitView::from_resolved`].
#[derive(Clone, Debug)]
pub struct ResolvedEntry {
    pub name: String,
    pub version: String,
    pub ecosystem: Option<String>,
    pub selected: bool,
    pub group_id: Option<String>,
    pub commit_count: usize,
    pub bump_hint: BumpHint,
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
                        ExtKind::JvmPluginManaged => (
                            "JVM (plugin-managed) — uses your plugin's release flow (axion-release / nebula-release / app-versioning)",
                            "kotlin",
                        ),
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
                group_id: None,
            });
        }

        Self {
            bundles,
            units,
            externally_managed: ext_rows,
            groups: Vec::new(),
        }
    }

    /// Build the view from prepare's resolved-unit list. Produces
    /// `(view, overlay)`: the view holds the universal row shape with
    /// `[group.<id>]` membership baked into [`UnitRow::group_id`] and
    /// one [`GroupRowDisplay`] per group; the overlay carries
    /// prepare-modal extras (bump hints, commit counts) keyed by
    /// `UnitRow::backref`. Render with
    /// `view.render_with_overlay(.., ViewLayout::Grouped)`.
    pub fn from_resolved(entries: &[ResolvedEntry]) -> (Self, PrepareOverlay) {
        let mut units: Vec<UnitRow> = Vec::with_capacity(entries.len());
        let mut overlay = PrepareOverlay::default();

        for (idx, e) in entries.iter().enumerate() {
            units.push(UnitRow {
                name: e.name.clone(),
                version: e.version.clone(),
                prefix: String::new(),
                ecosystem: e.ecosystem.clone(),
                annotations: Vec::new(),
                selected: e.selected,
                backref: idx,
                group_id: e.group_id.clone(),
            });
            overlay.bumps.insert(idx, e.bump_hint);
            overlay.commits.insert(idx, e.commit_count);
        }

        let mut groups: Vec<GroupRowDisplay> = Vec::new();
        let mut group_pos: HashMap<String, usize> = HashMap::new();
        for e in entries {
            let Some(gid) = e.group_id.as_ref() else {
                continue;
            };
            if let Some(&pos) = group_pos.get(gid) {
                let g = &mut groups[pos];
                g.members.push(GroupMemberDisplay {
                    name: e.name.clone(),
                    ecosystem_label: e.ecosystem.clone().unwrap_or_else(|| "unknown".to_string()),
                });
                if !e.selected {
                    g.all_selected = false;
                }
                if e.selected {
                    g.any_selected = true;
                }
            } else {
                let pos = groups.len();
                group_pos.insert(gid.clone(), pos);
                groups.push(GroupRowDisplay {
                    id: gid.clone(),
                    members: vec![GroupMemberDisplay {
                        name: e.name.clone(),
                        ecosystem_label: e
                            .ecosystem
                            .clone()
                            .unwrap_or_else(|| "unknown".to_string()),
                    }],
                    all_selected: e.selected,
                    any_selected: e.selected,
                    suggested_bump: Some(e.bump_hint),
                });
            }
        }

        let view = Self {
            bundles: Vec::new(),
            units,
            externally_managed: Vec::new(),
            groups,
        };
        (view, overlay)
    }

    /// Total togglable rows (Bundles + Units; Ext is read-only).
    pub fn togglable_len(&self) -> usize {
        self.bundles.len() + self.units.len()
    }

    /// All rows in [`ViewLayout::Sectioned`] order as `RowIdx`. Useful
    /// for cursor navigation: cursor is an index into the flat returned
    /// list. Equivalent to `flat_indices_for_layout(ViewLayout::Sectioned)`.
    pub fn flat_indices(&self) -> Vec<RowIdx> {
        self.flat_indices_for_layout(ViewLayout::Sectioned)
    }

    /// All rows in display order for the given layout. In Grouped, the
    /// returned indices match the cursor stops the renderer produces:
    /// each group is a single stop (members are visually embedded but
    /// not separately addressable), and units with `group_id` set are
    /// hidden from the solo list.
    pub fn flat_indices_for_layout(&self, layout: ViewLayout) -> Vec<RowIdx> {
        let mut out = Vec::with_capacity(
            self.bundles.len()
                + self.units.len()
                + self.externally_managed.len()
                + self.groups.len(),
        );
        for i in 0..self.bundles.len() {
            out.push(RowIdx::Bundle(i));
        }
        match layout {
            ViewLayout::Sectioned => {
                for i in 0..self.units.len() {
                    out.push(RowIdx::Unit(i));
                }
            }
            ViewLayout::Grouped => {
                let mut emitted_groups: Vec<&str> = Vec::with_capacity(self.groups.len());
                for (i, u) in self.units.iter().enumerate() {
                    match &u.group_id {
                        Some(gid) => {
                            if !emitted_groups.contains(&gid.as_str()) {
                                if let Some(group_idx) =
                                    self.groups.iter().position(|g| g.id == *gid)
                                {
                                    out.push(RowIdx::Group(group_idx));
                                    emitted_groups.push(gid.as_str());
                                }
                            }
                        }
                        None => out.push(RowIdx::Unit(i)),
                    }
                }
            }
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
            RowIdx::Group(i) => {
                if let Some(g) = self.groups.get(i).cloned() {
                    let target = !g.all_selected;
                    let member_names: Vec<String> =
                        g.members.iter().map(|m| m.name.clone()).collect();
                    for u in self.units.iter_mut() {
                        if member_names.contains(&u.name) {
                            u.selected = target;
                        }
                    }
                    if let Some(gm) = self.groups.get_mut(i) {
                        gm.all_selected = target;
                        gm.any_selected = target;
                    }
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

/// Render a single [`UnitRow`] as a `(Line, background-style)` pair.
/// All callers go through [`ReleaseUnitView::render`] /
/// [`ReleaseUnitView::render_with_overlay`]; this helper stays
/// internal so per-row formatting cannot drift between consumers.
fn render_unit_row_line(
    row: &UnitRow,
    is_current: bool,
    mode: RenderMode,
    label_width: usize,
    overlay: Option<&PrepareOverlay>,
) -> (Line<'static>, Style) {
    let bg = if is_current {
        Style::default().bg(Color::Rgb(40, 40, 50))
    } else {
        Style::default()
    };
    let cursor_color = if row.selected {
        Color::Green
    } else {
        Color::DarkGray
    };
    let label_style = if is_current {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else if row.selected {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::White)
    };

    let indicator = checkbox_or_lock(mode, row.selected, false);
    let pad = label_width.saturating_sub(row.name.chars().count());
    let padded = format!("{}{}", row.name, " ".repeat(pad));
    let commit_count = overlay.and_then(|o| o.commits.get(&row.backref).copied());
    let secondary = match (mode, commit_count) {
        (RenderMode::Prepare, Some(n)) => format!("({} commits)", n),
        _ => format!("@ {} ({})", row.version, row.prefix),
    };
    let mut spans = vec![
        Span::styled("    ".to_string(), Style::default()),
        Span::styled(format!("{} ", indicator), Style::default().fg(cursor_color)),
        Span::styled(
            row.ecosystem
                .as_deref()
                .map(glyphs::ecosystem)
                .unwrap_or("")
                .to_string(),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(padded, label_style),
        Span::styled("  ".to_string(), Style::default()),
        Span::styled(secondary, Style::default().fg(Color::Gray)),
    ];
    for ann in &row.annotations {
        spans.push(Span::styled("  ".to_string(), Style::default()));
        spans.push(Span::styled(
            format!("↳ {}", ann.label()),
            Style::default().fg(Color::Yellow),
        ));
    }
    if let Some(badge) = overlay.and_then(|o| o.cascade_overrides.get(&row.backref)) {
        spans.push(Span::styled("  ".to_string(), Style::default()));
        spans.push(Span::styled(
            format!("⇄ {} · {}", badge.source, badge.strategy_label),
            Style::default().fg(Color::Magenta),
        ));
    }
    if matches!(mode, RenderMode::Prepare) {
        if let Some(hint) = overlay.and_then(|o| o.bumps.get(&row.backref).copied()) {
            let (text, color) = hint.label_and_color();
            if !text.is_empty() {
                spans.push(Span::styled("  ".to_string(), Style::default()));
                spans.push(Span::styled(
                    format!("→ {}", text),
                    Style::default().fg(color),
                ));
            }
        }
    }
    (Line::from(spans), bg)
}

// ---------------------------------------------------------------------------
// PrepareOverlay — wizard-modal extras keyed by `UnitRow::backref`.
// ---------------------------------------------------------------------------

/// Sidecar data the view layers on top of [`UnitRow`]s at render
/// time. Init populates `cascade_overrides` (user-confirmed
/// `cascade_from` rules); prepare populates `bumps` + `commits`
/// (conventional-commit recommendations). Both wizards reach the
/// same renderer via [`ReleaseUnitView::render_with_overlay`] —
/// keeping this off `UnitRow` itself means dashboard and init's
/// non-cascade flows aren't paying for fields they don't use.
#[derive(Debug, Default)]
pub struct PrepareOverlay {
    /// Conventional-commit bump suggestion per unit, keyed by
    /// `UnitRow::backref`. Populated by prepare.
    pub bumps: HashMap<usize, BumpHint>,
    /// Number of commits since the last release tag, keyed by
    /// `UnitRow::backref`. Populated by prepare.
    pub commits: HashMap<usize, usize>,
    /// User-confirmed `cascade_from` overrides, keyed by
    /// `UnitRow::backref`. Populated by init's `[c]` sub-step.
    pub cascade_overrides: HashMap<usize, CascadeOverrideBadge>,
}

// ---------------------------------------------------------------------------
// GroupRowDisplay — shared helper for prepare's group-row tree.
// ---------------------------------------------------------------------------

/// One member entry in a group row's tree.
#[derive(Clone, Debug)]
pub struct GroupMemberDisplay {
    pub name: String,
    pub ecosystem_label: String,
}

/// Aggregate state of a `[group.<id>]` block as a list row: the
/// header summarises the group's selection-state and shared bump,
/// the tree below it lists each member by ecosystem.
#[derive(Clone, Debug)]
pub struct GroupRowDisplay {
    pub id: String,
    pub members: Vec<GroupMemberDisplay>,
    pub all_selected: bool,
    pub any_selected: bool,
    pub suggested_bump: Option<BumpHint>,
}

impl GroupRowDisplay {
    /// Render this group-row into the multi-line shape prepare uses
    /// in its selection table. Header line carries selection state +
    /// shared bump; one indented connector-line per member follows.
    fn render_lines(&self, is_current: bool) -> Vec<Line<'static>> {
        let checkbox = if self.all_selected {
            "✅"
        } else if self.any_selected {
            "🟨"
        } else {
            "⬜"
        };

        let header_label_color = if is_current {
            Color::Cyan
        } else if self.all_selected {
            Color::Green
        } else if self.any_selected {
            Color::Yellow
        } else {
            Color::White
        };

        let (suggestion_text, suggestion_color) = match self.suggested_bump {
            Some(BumpHint::Major) => ("MAJOR", Color::Red),
            Some(BumpHint::Minor) => ("MINOR", Color::Yellow),
            Some(BumpHint::Patch) => ("PATCH", Color::Green),
            _ => ("", Color::Gray),
        };

        let header = Line::from(vec![
            Span::styled(format!(" {} ", checkbox), Style::default()),
            Span::styled(
                self.id.clone(),
                Style::default()
                    .fg(header_label_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" (group, {} members)", self.members.len()),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::ITALIC),
            ),
            Span::styled(
                if !suggestion_text.is_empty() {
                    format!("  → {}", suggestion_text)
                } else {
                    String::new()
                },
                Style::default().fg(suggestion_color),
            ),
        ]);

        let mut lines = vec![header];
        for (i, m) in self.members.iter().enumerate() {
            let connector = if i == self.members.len() - 1 {
                "    └─ "
            } else {
                "    ├─ "
            };
            lines.push(Line::from(vec![
                Span::styled(connector, Style::default().fg(Color::DarkGray)),
                Span::styled(m.name.clone(), Style::default().fg(Color::Gray)),
                Span::styled(
                    format!("  ({})", m.ecosystem_label),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }
        lines
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
    /// Render the view inside `area` using [`ViewLayout::Sectioned`].
    /// The caller owns the layout chrome (block / borders / hint bar);
    /// this method only fills the inner content area.
    pub fn render(&self, frame: &mut Frame, area: Rect, ctx: &ViewContext) {
        self.render_inner(frame, area, ctx, None, ViewLayout::Sectioned);
    }

    /// Render with prepare-modal overlay data (bump hints, commit
    /// counts, cascade-override badges) layered on top of the
    /// universal row shape. Pass [`ViewLayout::Sectioned`] for
    /// init/dashboard, [`ViewLayout::Grouped`] for prepare.
    pub fn render_with_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        ctx: &ViewContext,
        overlay: &PrepareOverlay,
        layout: ViewLayout,
    ) {
        self.render_inner(frame, area, ctx, Some(overlay), layout);
    }

    fn render_inner(
        &self,
        frame: &mut Frame,
        area: Rect,
        ctx: &ViewContext,
        overlay: Option<&PrepareOverlay>,
        layout: ViewLayout,
    ) {
        let items = match layout {
            ViewLayout::Sectioned => self.build_sectioned_items(ctx, overlay),
            ViewLayout::Grouped => self.build_grouped_items(ctx, overlay),
        };
        frame.render_widget(List::new(items), area);
    }

    fn label_width(&self) -> usize {
        self.bundles
            .iter()
            .map(|b| b.label.chars().count())
            .chain(self.units.iter().map(|u| u.name.chars().count()))
            .chain(
                self.externally_managed
                    .iter()
                    .map(|e| e.label.chars().count()),
            )
            .max()
            .unwrap_or(0)
    }

    fn build_sectioned_items<'a>(
        &'a self,
        ctx: &ViewContext,
        overlay: Option<&PrepareOverlay>,
    ) -> Vec<ListItem<'a>> {
        let mut items: Vec<ListItem> = Vec::new();
        let label_width = self.label_width();
        let flat = self.flat_indices_for_layout(ViewLayout::Sectioned);
        let mut last_section: Option<&'static str> = None;

        for (display_idx, row_idx) in flat.iter().enumerate() {
            let section = match row_idx {
                RowIdx::Bundle(_) => "Bundles",
                RowIdx::Unit(_) => "Standalone",
                RowIdx::Ext(_) => "Externally-managed",
                RowIdx::Group(_) => "Groups",
            };
            if last_section != Some(section) {
                if last_section.is_some() {
                    items.push(ListItem::new(Line::from("")));
                }
                items.push(section_header(section));
                last_section = Some(section);
            }

            let is_current = ctx.cursor == Some(display_idx);
            let line = self.render_row(*row_idx, ctx.mode, label_width, is_current, overlay);
            items.push(line);
        }

        items
    }

    fn build_grouped_items<'a>(
        &'a self,
        ctx: &ViewContext,
        overlay: Option<&PrepareOverlay>,
    ) -> Vec<ListItem<'a>> {
        let mut items: Vec<ListItem> = Vec::new();
        let label_width = self.label_width();
        let flat = self.flat_indices_for_layout(ViewLayout::Grouped);

        for (display_idx, row_idx) in flat.iter().enumerate() {
            let is_current = ctx.cursor == Some(display_idx);
            let item = self.render_row(*row_idx, ctx.mode, label_width, is_current, overlay);
            items.push(item);
        }

        items
    }

    fn render_row(
        &self,
        idx: RowIdx,
        mode: RenderMode,
        label_width: usize,
        is_current: bool,
        overlay: Option<&PrepareOverlay>,
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
                let (line, line_bg) =
                    render_unit_row_line(u, is_current, mode, label_width, overlay);
                ListItem::new(line).style(line_bg)
            }
            RowIdx::Group(i) => {
                let group = &self.groups[i];
                let lines = group.render_lines(is_current);
                let style = if is_current {
                    Style::default().bg(Color::Rgb(40, 40, 50))
                } else {
                    Style::default()
                };
                ListItem::new(lines).style(style)
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

    fn resolved(name: &str, group_id: Option<&str>, selected: bool) -> ResolvedEntry {
        ResolvedEntry {
            name: name.into(),
            version: "0.1.0".into(),
            ecosystem: Some("npm".into()),
            selected,
            group_id: group_id.map(str::to_string),
            commit_count: 0,
            bump_hint: BumpHint::None,
        }
    }

    #[test]
    fn from_resolved_collapses_group_members_in_encounter_order() {
        let entries = vec![
            resolved("@org/utils", None, true),
            resolved("@org/schema", Some("schema-bundle"), true),
            resolved("com.org:schema", Some("schema-bundle"), true),
            resolved("cli", None, true),
        ];
        let (view, overlay) = ReleaseUnitView::from_resolved(&entries);

        assert_eq!(view.units.len(), 4);
        assert_eq!(view.groups.len(), 1);
        assert_eq!(view.groups[0].id, "schema-bundle");
        assert_eq!(view.groups[0].members.len(), 2);
        assert!(view.groups[0].all_selected);

        let flat = view.flat_indices_for_layout(ViewLayout::Grouped);
        assert_eq!(flat.len(), 3, "two solos plus one group row");
        assert!(matches!(flat[0], RowIdx::Unit(0)));
        assert!(matches!(flat[1], RowIdx::Group(0)));
        assert!(matches!(flat[2], RowIdx::Unit(3)));

        assert_eq!(overlay.commits.len(), 4);
        assert_eq!(overlay.bumps.len(), 4);
    }

    #[test]
    fn from_resolved_group_toggle_flips_all_members() {
        let entries = vec![
            resolved("a", Some("g"), true),
            resolved("b", Some("g"), true),
        ];
        let (mut view, _overlay) = ReleaseUnitView::from_resolved(&entries);
        assert!(view.groups[0].all_selected);
        assert!(view.toggle(RowIdx::Group(0)));
        assert!(!view.units[0].selected);
        assert!(!view.units[1].selected);
        assert!(!view.groups[0].all_selected);
        assert!(view.toggle(RowIdx::Group(0)));
        assert!(view.units[0].selected);
        assert!(view.units[1].selected);
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
