//! 11-variant test matrix for the release-unit pipeline.
//!
//! Pins the classification + render contract against every shape we
//! expect to encounter in the wild. Each variant produces:
//!
//! - one `view_<variant>_struct.snap` — `Debug` of `ReleaseUnitView`
//!   (mode-unagnostic; flips only when classification changes).
//! - three `view_<variant>_<mode>_render.snap` — visual TUI render
//!   in `Init` / `Prepare` / `Dashboard` mode.
//!
//! The split lets a UX tweak break only the render half while a
//! classification regression breaks both — easy to read in CI which
//! kind of change a PR makes.
//!
//! Wire-format compatibility lives in `test_manifest_wire_compat.rs`.

mod common;
mod fixtures;

use std::collections::HashSet;
use std::path::Path;

use belaf::core::git::repository::Repository;
use belaf::core::release_unit::detector;
use belaf::core::ui::release_unit_view::{
    ReleaseUnitView, RenderMode, StandaloneEntry, ViewContext,
};
use common::TestRepo;
use fixtures::Seedable;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;

impl Seedable for TestRepo {
    fn root(&self) -> &Path {
        &self.path
    }
    fn write_file(&self, relative: &str, content: &str) {
        TestRepo::write_file(self, relative, content);
    }
    fn commit(&self, message: &str) {
        TestRepo::commit(self, message);
    }
}

/// Build a `ReleaseUnitView` from a seeded `TestRepo` using the same
/// path the init wizard takes: `detector::detect_all` + the
/// session's standalone-loader output.
fn build_view(repo: &TestRepo) -> ReleaseUnitView {
    let r = Repository::open(&repo.path).expect("Repository::open");
    let report = detector::detect_all(&r);

    // For these tests we don't run the full session-loader. Instead
    // we pass an empty standalone list — the snapshots show only what
    // the detector produced. This is enough to pin the
    // Bundle/Hint/ExternallyManaged classification and the hint
    // annotation logic; the loader-produced standalone units are
    // covered by ecosystem-specific tests elsewhere.
    let standalones: Vec<StandaloneEntry> = Vec::new();
    ReleaseUnitView::from_detection(&report, &standalones, &HashSet::new())
}

/// Render the view in a fixed-size virtual terminal so snapshots are
/// stable across CI machines.
fn render_view_to_string(view: &ReleaseUnitView, mode: RenderMode) -> String {
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| {
            let area = Rect::new(0, 0, 100, 30);
            let ctx = ViewContext { mode, cursor: None };
            view.render(frame, area, &ctx);
        })
        .expect("draw");
    let buffer = terminal.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            if let Some(cell) = buffer.cell((x, y)) {
                out.push_str(cell.symbol());
            }
        }
        // Trim trailing spaces per line for stable diffs.
        while out.ends_with(' ') {
            out.pop();
        }
        out.push('\n');
    }
    out
}

/// Single test entry per variant. We name each test
/// `<variant>_classification_and_render` so a failing test is greppable.
macro_rules! variant_test {
    ($name:ident, $seed:path) => {
        #[test]
        fn $name() {
            let variant_name = stringify!($name).trim_end_matches("_classification_and_render");
            let repo = TestRepo::new();
            $seed(&repo);
            let view = build_view(&repo);

            // Stable struct snapshot.
            insta::with_settings!({snapshot_suffix => format!("{variant_name}_struct")}, {
                insta::assert_debug_snapshot!(&view);
            });

            // Three render snapshots (Init / Prepare / Dashboard).
            for mode in [RenderMode::Init, RenderMode::Prepare, RenderMode::Dashboard] {
                let mode_label = match mode {
                    RenderMode::Init => "init",
                    RenderMode::Prepare => "prepare",
                    RenderMode::Dashboard => "dashboard",
                };
                let rendered = render_view_to_string(&view, mode);
                insta::with_settings!({
                    snapshot_suffix => format!("{variant_name}_{mode_label}_render")
                }, {
                    insta::assert_snapshot!(&rendered);
                });
            }
        }
    };
}

/// Per-variant toggle test — sweeps every togglable row and asserts
/// that each toggle hits exactly one row in the expected slot
/// (Bundle vs Unit) without spilling over to a sibling section.
/// Pins the brief's "toggle hits the right row in each repo shape"
/// requirement.
fn assert_toggle_round_trip(seed: fn(&TestRepo)) {
    let repo = TestRepo::new();
    seed(&repo);
    let mut view = build_view(&repo);
    let initial_selected = view.selected_togglable_count();

    for idx in view.flat_indices().clone() {
        match idx {
            belaf::core::ui::release_unit_view::RowIdx::Bundle(i) => {
                let prior = view.bundles[i].selected;
                view.toggle(idx);
                assert_eq!(
                    view.bundles[i].selected, !prior,
                    "Bundle({i}) toggle should flip its `selected` flag"
                );
                view.toggle(idx);
                assert_eq!(view.bundles[i].selected, prior, "double-toggle restores");
            }
            belaf::core::ui::release_unit_view::RowIdx::Unit(i) => {
                let prior = view.units[i].selected;
                view.toggle(idx);
                assert_eq!(
                    view.units[i].selected, !prior,
                    "Unit({i}) toggle should flip its `selected` flag"
                );
                view.toggle(idx);
                assert_eq!(view.units[i].selected, prior, "double-toggle restores");
            }
            belaf::core::ui::release_unit_view::RowIdx::Ext(_) => {
                let prior = view.selected_togglable_count();
                view.toggle(idx);
                assert_eq!(
                    view.selected_togglable_count(),
                    prior,
                    "Ext rows are read-only — toggle must be a no-op"
                );
            }
            belaf::core::ui::release_unit_view::RowIdx::Group(_) => {
                unreachable!("from_detection produces no group rows; only from_resolved does")
            }
        }
    }
    assert_eq!(
        view.selected_togglable_count(),
        initial_selected,
        "after the sweep + double-toggles every togglable row is back to its initial state"
    );
}

macro_rules! toggle_test {
    ($name:ident, $seed:path) => {
        #[test]
        fn $name() {
            assert_toggle_round_trip($seed);
        }
    };
}

// ===========================================================================
// 11 variants. Indexed per BELAF_PLAN §5.
// ===========================================================================

// 1. Single-crate Cargo (tokio-like)
variant_test!(
    single_cargo_classification_and_render,
    fixtures::seed_tokio_single
);

// 2. Single npm package (lodash-like)
variant_test!(
    single_npm_classification_and_render,
    fixtures::seed_lodash_single
);

// 3. npm workspace monorepo (turbo-like)
variant_test!(
    npm_workspace_monorepo_classification_and_render,
    fixtures::seed_turbo_workspace
);

// 4. Cargo workspace monorepo (embassy-like)
variant_test!(
    cargo_workspace_monorepo_classification_and_render,
    fixtures::seed_cargo_monorepo_independent
);

// 5. Hexagonal Cargo service
variant_test!(
    hexagonal_cargo_classification_and_render,
    fixtures::seed_hexagonal_cargo_only
);

// 6. Tauri app
variant_test!(
    tauri_app_classification_and_render,
    fixtures::seed_tauri_app_only
);

// 7. JVM SDK
variant_test!(
    jvm_sdk_classification_and_render,
    fixtures::seed_kotlin_library_only
);

// 8. Generated SDK (TS)
variant_test!(
    generated_ts_sdk_classification_and_render,
    fixtures::seed_ts_sdk_cascade
);

// 9. Mobile-only
variant_test!(
    mobile_only_classification_and_render,
    fixtures::seed_ios_only
);

// 10. Polyglot
variant_test!(
    polyglot_classification_and_render,
    fixtures::seed_clikd_shape
);

// 11. Nested submodule with own monorepo
variant_test!(
    nested_submodule_classification_and_render,
    fixtures::seed_vendored_monorepo
);

// ===========================================================================
// Per-variant toggle tests — pin that `view.toggle(idx)` hits the
// right row slot in each repo shape (Bundles vs Units vs read-only
// Ext). Complements the snapshot-style `*_struct.snap` coverage with
// a behavioural assertion.
// ===========================================================================

toggle_test!(single_cargo_toggle_round_trip, fixtures::seed_tokio_single);
toggle_test!(single_npm_toggle_round_trip, fixtures::seed_lodash_single);
toggle_test!(
    npm_workspace_toggle_round_trip,
    fixtures::seed_turbo_workspace
);
toggle_test!(
    cargo_workspace_toggle_round_trip,
    fixtures::seed_cargo_monorepo_independent
);
toggle_test!(
    hexagonal_cargo_toggle_round_trip,
    fixtures::seed_hexagonal_cargo_only
);
toggle_test!(tauri_app_toggle_round_trip, fixtures::seed_tauri_app_only);
toggle_test!(
    jvm_sdk_toggle_round_trip,
    fixtures::seed_kotlin_library_only
);
toggle_test!(
    generated_ts_sdk_toggle_round_trip,
    fixtures::seed_ts_sdk_cascade
);
toggle_test!(mobile_only_toggle_round_trip, fixtures::seed_ios_only);
toggle_test!(polyglot_toggle_round_trip, fixtures::seed_clikd_shape);
toggle_test!(
    nested_submodule_toggle_round_trip,
    fixtures::seed_vendored_monorepo
);
