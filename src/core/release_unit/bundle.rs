//! Bundle detection + emission, one module per bundle kind.
//!
//! A "Bundle" in the [`super::shape::DetectedShape`] taxonomy is a
//! multi-manifest ReleaseUnit that emits a `[[release_unit]]` config
//! block (or, for hexagonal-cargo siblings, a single
//! `[[release_unit_glob]]`) and hides its inner manifests from the
//! Standalone list in the wizard.
//!
//! Each bundle module owns both the detection and the snippet emission
//! for its kind — so adding a new Bundle (e.g. `uniffi-rust-binding`)
//! is a single new file plus one `mod` line here, with no edits to
//! `auto_detect.rs` or `detector/scanners.rs`.
//!
//! Free functions, not a trait: the surface is currently 2 functions
//! per kind with no shared state — a trait would be ceremony for no
//! gain. If a future bundle needs persistent config (e.g. Tauri-
//! specific tag-format defaults) we can add a trait then.

use std::path::Path;

pub mod hexagonal;
pub mod jvm_library;
pub mod tauri;

use super::shape::DetectorMatch;
use crate::cmd::init::auto_detect::DetectionCounters;

/// Run every bundle scanner against `workdir` and concatenate the
/// matches. Detection ordering matches the legacy `detector::detect_all`
/// pre-1.0 — hexagonal first, then tauri, then jvm — so the dedup
/// logic in `unified_selection.rs::rebuild_rows` keeps the same
/// outcome.
pub fn detect_all(workdir: &Path) -> Vec<DetectorMatch> {
    let mut out = Vec::new();
    out.extend(hexagonal::detect(workdir));
    out.extend(tauri::detect(workdir));
    out.extend(jvm_library::detect(workdir));
    out
}

/// Emit `[[release_unit]]` / `[[release_unit_glob]]` blocks for every
/// Bundle match. Auto-detect calls this once and never has to know
/// which Bundle kinds exist — adding a new Bundle is a new module +
/// one `mod` line + one call here.
pub fn emit_all(
    matches: &[DetectorMatch],
    snippet: &mut String,
    counters: &mut DetectionCounters,
) {
    let bundles: Vec<&DetectorMatch> = matches
        .iter()
        .filter(|m| m.shape.is_bundle())
        .collect();
    hexagonal::emit_all(&bundles, snippet, counters);
    tauri::emit_all(&bundles, snippet, counters);
    jvm_library::emit_all(&bundles, snippet, counters);
}
