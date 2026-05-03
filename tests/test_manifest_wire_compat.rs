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
//! Coverage today is the four ecosystems with a registered
//! `Ecosystem` loader that fully wires through to `prepare`:
//! single-crate cargo, single-package npm, cargo workspaces, and
//! nested-submodule outer crates. The remaining seven variants
//! (Tauri / hexagonal-cargo / JVM-library bundles, mobile-only,
//! cascade-from SDKs, npm workspace monorepos, polyglot cross-eco
//! groups) currently route through `[release_unit.X]` blocks whose
//! resolver→graph integration is the work of the ReleaseUnit-centric
//! graph refactor (see plan §Schicht 2.5). Once that lands, this file
//! will grow to cover all eleven variants the same way.

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

    let init_out =
        repo.run_belaf_command_with_env(&["init", "--ci", "--force"], &[("BELAF_NO_KEYRING", "1")]);
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
    let _ = repo.run_belaf_command_with_env(&["prepare", "--ci"], &[("BELAF_NO_KEYRING", "1")]);

    let manifests = manifest_files(&repo);
    assert!(
        !manifests.is_empty(),
        "prepare should have emitted a manifest under belaf/releases/"
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
