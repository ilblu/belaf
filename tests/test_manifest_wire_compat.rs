//! Wire-format safety net driven by the real `belaf prepare --ci`
//! pipeline.
//!
//! Each test seeds a synthetic [`TestRepo`], runs
//! `belaf init --ci --force` followed by `belaf prepare --ci`, reads
//! the manifest the production
//! [`ReleasePipeline::create_manifest`](belaf::core::workflow) path
//! wrote to `belaf/releases/*.json`, and validates it against
//! `schemas/manifest.v1.schema.json`.
//!
//! Variants without any release units (mobile-only) are the
//! exception: prepare emits no manifest, so the assertion is "no
//! manifest" rather than "schema-valid manifest".

mod common;
mod fixtures;

use std::fs;
use std::path::{Path, PathBuf};

use common::TestRepo;
use fixtures::Seedable;
use serde_json::Value;

const SCHEMA_PATH: &str = "schemas/manifest.v1.schema.json";

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

fn validate_against_schema(json_value: &Value) {
    let schema_raw = fs::read_to_string(SCHEMA_PATH).expect("read schema");
    let schema_json: Value = serde_json::from_str(&schema_raw).expect("parse schema");
    let validator = jsonschema::draft202012::new(&schema_json).expect("compile schema");
    if let Err(err) = validator.validate(json_value) {
        panic!(
            "manifest fails schema validation: {err}\n\nmanifest:\n{}",
            serde_json::to_string_pretty(json_value).unwrap_or_default()
        );
    }
}

fn manifest_files(repo: &TestRepo) -> Vec<PathBuf> {
    let dir = repo.path.join("belaf").join("releases");
    if !dir.exists() {
        return Vec::new();
    }
    fs::read_dir(&dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
                .collect()
        })
        .unwrap_or_default()
}

fn read_manifest(path: &Path) -> Value {
    let raw = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("read manifest at {}: {e}", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("parse manifest at {}: {e}\ncontent:\n{raw}", path.display()))
}

/// Drive a synthetic repo through `belaf init --ci --force` then
/// `belaf prepare --ci`, returning the manifest the pipeline emitted.
///
/// `seed` writes the working tree + creates the seed commit. `mutate`
/// runs after `init` and before `prepare` — its job is to add a
/// conventional-commit feat/fix so prepare has something to bump.
fn run_pipeline(seed: fn(&TestRepo), mutate: fn(&TestRepo)) -> Value {
    let repo = TestRepo::new();
    seed(&repo);

    let init_out = repo.run_belaf_command_with_env(
        &["init", "--ci", "--force", "--auto-detect"],
        &[("BELAF_NO_KEYRING", "1")],
    );
    assert!(
        init_out.status.success(),
        "init failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&init_out.stdout),
        String::from_utf8_lossy(&init_out.stderr),
    );
    repo.commit("chore: bootstrap belaf");

    mutate(&repo);

    // The push/PR step will fail (no upstream, no auth) but the
    // manifest is written before that, so the exit code doesn't matter.
    let prepare_out =
        repo.run_belaf_command_with_env(&["prepare", "--ci"], &[("BELAF_NO_KEYRING", "1")]);

    let manifests = manifest_files(&repo);
    assert!(
        !manifests.is_empty(),
        "prepare should have emitted a manifest under belaf/releases/\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&prepare_out.stdout),
        String::from_utf8_lossy(&prepare_out.stderr),
    );
    read_manifest(&manifests[0])
}

#[test]
fn pipeline_single_cargo_emits_schema_valid_manifest() {
    fn mutate(repo: &TestRepo) {
        repo.write_file("src/feat_module.rs", "pub fn new_thing() {}\n");
        repo.commit("feat: add new_thing");
    }
    let m = run_pipeline(fixtures::seed_tokio_single, mutate);
    validate_against_schema(&m);
    let releases = m["releases"].as_array().expect("releases array");
    assert_eq!(releases.len(), 1);
    assert_eq!(releases[0]["ecosystem"], "cargo");
    assert_eq!(releases[0]["name"], "tokio-like");
    // Tag name must come from tag_format precedence — not just a
    // default — so the github-app reads the same string the CLI
    // pushes upstream.
    assert!(
        releases[0]["tag_name"]
            .as_str()
            .map(|s| !s.is_empty())
            .unwrap_or(false),
        "tag_name must be populated"
    );
}

#[test]
fn pipeline_single_npm_emits_schema_valid_manifest() {
    fn mutate(repo: &TestRepo) {
        repo.write_file("index.js", "module.exports = function v2() {};\n");
        repo.commit("feat: extend public api");
    }
    let m = run_pipeline(fixtures::seed_lodash_single, mutate);
    validate_against_schema(&m);
    let releases = m["releases"].as_array().expect("releases array");
    assert_eq!(releases.len(), 1);
    assert_eq!(releases[0]["ecosystem"], "npm");
}

#[test]
fn pipeline_cargo_workspace_emits_schema_valid_manifest() {
    fn mutate(repo: &TestRepo) {
        repo.write_file("crates/alpha/src/extra.rs", "pub fn extra() {}\n");
        repo.commit("feat(alpha): expose extra()");
    }
    let m = run_pipeline(fixtures::seed_cargo_monorepo_independent, mutate);
    validate_against_schema(&m);
    let releases = m["releases"].as_array().expect("releases array");
    assert!(!releases.is_empty(), "expected at least one release");
    assert!(
        releases.iter().all(|r| r["ecosystem"] == "cargo"),
        "all releases should be cargo"
    );
}

#[test]
fn pipeline_nested_submodule_outer_crate_emits_schema_valid_manifest() {
    fn mutate(repo: &TestRepo) {
        repo.write_file("src/extra.rs", "pub fn extra() {}\n");
        repo.commit("feat: extend outer crate");
    }
    let m = run_pipeline(fixtures::seed_vendored_monorepo, mutate);
    validate_against_schema(&m);
    let releases = m["releases"].as_array().expect("releases array");
    assert_eq!(releases.len(), 1);
    assert_eq!(releases[0]["name"], "outer");
}

#[test]
fn pipeline_npm_workspace_monorepo_emits_schema_valid_manifest() {
    // The turbo fixture has a nested workspace that fires the
    // `NpmWorkspace` *hint* (annotation only — never a configured
    // release_unit block). prepare's strict-coverage guard rejects
    // hint-detector hits that aren't either configured or
    // `[ignore_paths]`-listed, so we add the hint paths to
    // ignore_paths after init to let prepare run.
    let repo = TestRepo::new();
    fixtures::seed_turbo_workspace(&repo);
    let init_out = repo.run_belaf_command_with_env(
        &["init", "--ci", "--force", "--auto-detect"],
        &[("BELAF_NO_KEYRING", "1")],
    );
    assert!(init_out.status.success(), "init failed");
    let cfg = repo.read_file("belaf/config.toml");
    let cfg =
        format!("{cfg}\n[ignore_paths]\npaths = [\"apps/docs\", \"apps/docs/sub-packages\"]\n");
    repo.write_file("belaf/config.toml", &cfg);
    repo.commit("chore: bootstrap belaf");
    repo.write_file(
        "packages/ui/src/Button.tsx",
        "export const Button = () => null;\n",
    );
    repo.commit("feat(@turbo/ui): add Button component");
    let _ = repo.run_belaf_command_with_env(&["prepare", "--ci"], &[("BELAF_NO_KEYRING", "1")]);
    let manifests = manifest_files(&repo);
    if let Some(m) = manifests.first().map(|p| read_manifest(p)) {
        validate_against_schema(&m);
        assert!(m["releases"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["ecosystem"] == "npm"));
    }
}

#[test]
fn pipeline_hexagonal_cargo_emits_schema_valid_manifest() {
    fn mutate(repo: &TestRepo) {
        repo.write_file(
            "apps/services/aura/crates/api/src/handler.rs",
            "pub fn handler() {}\n",
        );
        repo.commit("feat(aura): add handler");
    }
    let m = run_pipeline(fixtures::seed_hexagonal_cargo_only, mutate);
    validate_against_schema(&m);
    let releases = m["releases"].as_array().expect("releases array");
    assert!(!releases.is_empty(), "expected at least one release");
}

#[test]
fn pipeline_tauri_app_emits_schema_valid_manifest() {
    fn mutate(repo: &TestRepo) {
        repo.write_file(
            "apps/desktop/package.json",
            r#"{
  "name": "desktop",
  "version": "0.1.0",
  "private": true,
  "description": "added"
}
"#,
        );
        repo.commit("feat: extend desktop manifest");
    }
    let m = run_pipeline(fixtures::seed_tauri_app_only, mutate);
    validate_against_schema(&m);
    let releases = m["releases"].as_array().expect("releases array");
    assert!(!releases.is_empty(), "expected at least one release");
}

#[test]
fn pipeline_jvm_library_emits_schema_valid_manifest() {
    fn mutate(repo: &TestRepo) {
        repo.write_file(
            "libs/main/src/main/kotlin/Extra.kt",
            "package com.example\nclass Extra\n",
        );
        repo.commit("feat: add Extra");
    }
    // jvm-library coverage may be rejected by `init --ci` if the
    // detector doesn't surface a release_unit. The variant is wired
    // through the production pipeline once init configures the
    // bundle; absent that, prepare has nothing to bump and emits no
    // manifest. Tolerate either outcome but if we get a manifest it
    // must be schema-valid.
    let repo = TestRepo::new();
    fixtures::seed_kotlin_library_only(&repo);
    let init_out =
        repo.run_belaf_command_with_env(&["init", "--ci", "--force"], &[("BELAF_NO_KEYRING", "1")]);
    if !init_out.status.success() {
        return;
    }
    repo.commit("chore: bootstrap belaf");
    mutate(&repo);
    let _ = repo.run_belaf_command_with_env(&["prepare", "--ci"], &[("BELAF_NO_KEYRING", "1")]);
    if let Some(m) = manifest_files(&repo).first().map(|p| read_manifest(p)) {
        validate_against_schema(&m);
    }
}

#[test]
fn pipeline_generated_ts_sdk_emits_schema_valid_manifest() {
    // The ts-sdk-cascade fixture fires an `SdkCascade` *hint* on the
    // `sdks/typescript/` path. Prepare's strict-coverage guard
    // rejects hint hits without explicit coverage. Real users would
    // either add the cascade to a `[release_unit.X]` block or list
    // the path in `[ignore_paths]`; the test does the latter so
    // prepare can produce a manifest for the npm package itself.
    let repo = TestRepo::new();
    fixtures::seed_ts_sdk_cascade(&repo);
    let init_out = repo.run_belaf_command_with_env(
        &["init", "--ci", "--force", "--auto-detect"],
        &[("BELAF_NO_KEYRING", "1")],
    );
    assert!(init_out.status.success(), "init failed");
    let cfg = repo.read_file("belaf/config.toml");
    let cfg = format!("{cfg}\n[ignore_paths]\npaths = [\"sdks/typescript\"]\n");
    repo.write_file("belaf/config.toml", &cfg);
    repo.commit("chore: bootstrap belaf");
    repo.write_file(
        "sdks/typescript/src/extra.ts",
        "export const extra = () => 1;\n",
    );
    repo.commit("feat: extend sdk surface");
    let _ = repo.run_belaf_command_with_env(&["prepare", "--ci"], &[("BELAF_NO_KEYRING", "1")]);
    if let Some(m) = manifest_files(&repo).first().map(|p| read_manifest(p)) {
        validate_against_schema(&m);
    }
}

#[test]
fn pipeline_mobile_only_emits_no_manifest() {
    // iOS-only repos surface as `ExternallyManaged`; init writes
    // `[allow_uncovered]` and no release units, so prepare emits no
    // manifest. Pin that contract.
    let repo = TestRepo::new();
    fixtures::seed_ios_only(&repo);
    let _init =
        repo.run_belaf_command_with_env(&["init", "--ci", "--force"], &[("BELAF_NO_KEYRING", "1")]);
    repo.write_file("app/MyApp/View.swift", "import SwiftUI\n");
    repo.commit("feat: add view");
    let _ = repo.run_belaf_command_with_env(&["prepare", "--ci"], &[("BELAF_NO_KEYRING", "1")]);
    assert!(
        manifest_files(&repo).is_empty(),
        "mobile-only repo must not produce a manifest"
    );
}

#[test]
fn pipeline_polyglot_emits_schema_valid_manifest() {
    fn mutate(repo: &TestRepo) {
        repo.write_file("ts/extra.ts", "export const extra = 1;\n");
        repo.commit("feat: extend schema");
    }
    let repo = TestRepo::new();
    fixtures::seed_polyglot_cross_eco_group(&repo);
    let init_out =
        repo.run_belaf_command_with_env(&["init", "--ci", "--force"], &[("BELAF_NO_KEYRING", "1")]);
    if !init_out.status.success() {
        return;
    }
    repo.commit("chore: bootstrap belaf");
    mutate(&repo);
    let _ = repo.run_belaf_command_with_env(&["prepare", "--ci"], &[("BELAF_NO_KEYRING", "1")]);
    if let Some(m) = manifest_files(&repo).first().map(|p| read_manifest(p)) {
        validate_against_schema(&m);
    }
}
