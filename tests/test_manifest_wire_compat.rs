//! Wire-format safety net for the 11-variant fixture matrix.
//!
//! For each variant we synthesise a minimal valid manifest, validate
//! it against `schemas/manifest.v1.schema.json`, and round-trip it
//! through the domain model. Schicht 0 (schema rename) and Schicht 4
//! (config cleanup) can't subtly break the wire format without one
//! of these tests firing.
//!
//! These don't run the full release pipeline — that's covered
//! elsewhere (test_release_prepare etc.). They cover the contract
//! between the producer (CLI) and consumer (github-app) at the
//! manifest level.

use std::fs;

use belaf::core::manifest::{ReleaseEntry, ReleaseManifest};
use serde_json::Value;

const SCHEMA_PATH: &str = "schemas/manifest.v1.schema.json";

/// Build a minimal valid manifest with one release entry per
/// (ecosystem, version_field) shape the CLI emits.
fn build_manifest_for(ecosystem: &str, name: &str, prefix: &str) -> ReleaseManifest {
    let mut m = ReleaseManifest::new("main".to_string(), "ci-bot".to_string());
    m.add_release(ReleaseEntry::new(
        name.to_string(),
        ecosystem.to_string(),
        "0.1.0".to_string(),
        "0.2.0".to_string(),
        "minor".to_string(),
        "## What's new\n- placeholder change\n".to_string(),
        prefix.to_string(),
    ));
    m
}

fn validate_against_schema(json_value: &Value) {
    let schema_raw = fs::read_to_string(SCHEMA_PATH).expect("read schema");
    let schema_json: Value = serde_json::from_str(&schema_raw).expect("parse schema");
    let validator = jsonschema::draft202012::new(&schema_json).expect("compile schema");
    if let Err(err) = validator.validate(json_value) {
        panic!("manifest fails schema validation: {err}");
    }
}

fn round_trip(m: &ReleaseManifest) -> ReleaseManifest {
    let json = m.to_json().expect("serialize");
    let value: Value = serde_json::from_str(&json).expect("re-parse to Value");
    validate_against_schema(&value);
    ReleaseManifest::from_json(&json).expect("deserialize")
}

// ---------------------------------------------------------------------------
// Per-variant wire-compat tests.
//
// Each variant emits the manifest shape that would be produced by
// the corresponding repo layout's release pipeline. Schemas are
// validated; round-trip preserves identity at the domain layer.
// ---------------------------------------------------------------------------

#[test]
fn single_cargo_wire_compat() {
    let m = build_manifest_for("cargo", "tokio", "");
    let m2 = round_trip(&m);
    assert_eq!(m2.releases[0].name, "tokio");
    assert_eq!(m2.releases[0].ecosystem.as_str(), "cargo");
}

#[test]
fn single_npm_wire_compat() {
    let m = build_manifest_for("npm", "lodash", "");
    let m2 = round_trip(&m);
    assert_eq!(m2.releases[0].ecosystem.as_str(), "npm");
}

#[test]
fn npm_workspace_monorepo_wire_compat() {
    let mut m = ReleaseManifest::new("main".to_string(), "ci-bot".to_string());
    for member in ["@turbo/ui", "@turbo/utils", "@turbo/docs"] {
        m.add_release(ReleaseEntry::new(
            member.to_string(),
            "npm".to_string(),
            "0.1.0".to_string(),
            "0.2.0".to_string(),
            "minor".to_string(),
            String::new(),
            String::new(),
        ));
    }
    let m2 = round_trip(&m);
    assert_eq!(m2.releases.len(), 3);
}

#[test]
fn cargo_workspace_monorepo_wire_compat() {
    let mut m = ReleaseManifest::new("main".to_string(), "ci-bot".to_string());
    for c in ["alpha", "beta", "gamma"] {
        m.add_release(ReleaseEntry::new(
            c.to_string(),
            "cargo".to_string(),
            "0.1.0".to_string(),
            "0.2.0".to_string(),
            "minor".to_string(),
            String::new(),
            format!("crates/{c}"),
        ));
    }
    let m2 = round_trip(&m);
    assert_eq!(m2.releases.len(), 3);
}

#[test]
fn hexagonal_cargo_wire_compat() {
    let m = build_manifest_for("cargo", "aura", "apps/services/aura");
    let m2 = round_trip(&m);
    assert_eq!(m2.releases[0].tag_name, "apps/services/aura/v0.2.0");
}

#[test]
fn tauri_app_wire_compat() {
    // Tauri ships under a single tag — bundle_manifests carries the
    // 3-file lockstep state.
    let m = build_manifest_for("tauri", "desktop", "apps/desktop");
    let m_with_bundle = ReleaseManifest::new("main".to_string(), "ci-bot".to_string());
    let mut m_with_bundle = m_with_bundle;
    let r = m
        .releases
        .into_iter()
        .next()
        .unwrap()
        .with_bundle_manifests(vec![
            "apps/desktop/package.json".to_string(),
            "apps/desktop/src-tauri/Cargo.toml".to_string(),
            "apps/desktop/src-tauri/tauri.conf.json".to_string(),
        ]);
    m_with_bundle.add_release(r);
    let m2 = round_trip(&m_with_bundle);
    assert_eq!(m2.releases[0].bundle_manifests.len(), 3);
}

#[test]
fn jvm_sdk_wire_compat() {
    let m = build_manifest_for("jvm-library", "kotlin-sdk", "sdks/kotlin");
    let m2 = round_trip(&m);
    assert_eq!(m2.releases[0].ecosystem.as_str(), "jvm-library");
}

#[test]
fn generated_ts_sdk_wire_compat() {
    use belaf::core::wire::domain::CascadeFromWire;

    let m_initial = ReleaseManifest::new("main".to_string(), "ci-bot".to_string());
    let mut m = m_initial;
    let r = ReleaseEntry::new(
        "@org/sdk-typescript".to_string(),
        "npm".to_string(),
        "1.0.0".to_string(),
        "1.1.0".to_string(),
        "minor".to_string(),
        String::new(),
        "sdks/typescript".to_string(),
    )
    .with_cascade_from(CascadeFromWire {
        source: "schema".to_string(),
        bump: "floor_minor".to_string(),
    });
    m.add_release(r);
    let m2 = round_trip(&m);
    assert_eq!(
        m2.releases[0]
            .cascade_from
            .as_ref()
            .map(|c| c.source.as_str()),
        Some("schema")
    );
}

#[test]
fn mobile_only_wire_compat() {
    // Mobile apps don't produce manifest entries; the manifest is
    // just empty (groups + releases both empty) and must validate.
    let m = ReleaseManifest::new("main".to_string(), "ci-bot".to_string());
    let m2 = round_trip(&m);
    assert!(m2.releases.is_empty());
    assert!(m2.groups.is_empty());
}

#[test]
fn polyglot_wire_compat() {
    use belaf::core::wire::domain::Group;

    let mut m = ReleaseManifest::new("main".to_string(), "ci-bot".to_string());
    m.add_release(
        ReleaseEntry::new(
            "aura".to_string(),
            "cargo".to_string(),
            "0.1.0".to_string(),
            "0.2.0".to_string(),
            "minor".to_string(),
            String::new(),
            "apps/services/aura".to_string(),
        )
        .with_bundle_manifests(vec!["apps/services/aura/crates/bin/Cargo.toml".to_string()]),
    );
    m.add_release(ReleaseEntry::new(
        "kotlin-sdk".to_string(),
        "jvm-library".to_string(),
        "1.0.0".to_string(),
        "1.0.1".to_string(),
        "patch".to_string(),
        String::new(),
        "sdks/kotlin".to_string(),
    ));
    m.add_release(ReleaseEntry::new(
        "@org/sdk-ts".to_string(),
        "npm".to_string(),
        "1.0.0".to_string(),
        "1.0.1".to_string(),
        "patch".to_string(),
        String::new(),
        "sdks/typescript".to_string(),
    ));
    m.add_group(Group {
        id: "schema".to_string(),
        members: vec!["kotlin-sdk".to_string(), "@org/sdk-ts".to_string()],
        x: serde_json::Map::new(),
    });
    let m2 = round_trip(&m);
    assert_eq!(m2.releases.len(), 3);
    assert_eq!(m2.groups.len(), 1);
    assert_eq!(m2.groups[0].id, "schema");
}

#[test]
fn nested_submodule_wire_compat() {
    // Nested-monorepo Hint doesn't directly produce a release entry.
    // The outer repo's own manifest still validates.
    let m = build_manifest_for("cargo", "outer", "");
    let m2 = round_trip(&m);
    assert_eq!(m2.releases[0].name, "outer");
}
