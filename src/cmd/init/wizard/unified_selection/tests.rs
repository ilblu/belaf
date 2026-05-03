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
    assert_eq!(step.view.flat_indices().len(), 4);
    step.cursor = 1;
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

    assert_eq!(step.view.flat_indices().len(), 2);
    assert_eq!(step.view.bundles.len(), 1);
    assert_eq!(step.view.units.len(), 1);
    assert_eq!(step.view.units[0].name, "elsewhere");
}

#[test]
fn hint_annotates_matching_standalone() {
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
