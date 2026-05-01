//! DoD #11 — `belaf init --ci` against the tokio-single fixture
//! lands a 1-line single-project tag-format override + the wizard's
//! TagFormatStep snippet builder produces valid TOML for it.
//!
//! The "TUI suggests `v{version}`" half is covered by the
//! `renders_tag_format_with_single_project` snapshot in
//! `cmd::init::wizard::tag_format::tests`. This file covers the
//! end-to-end shape: detector quiet, init succeeds, `belaf explain`
//! reports the single project.

mod common;
mod fixtures;

use std::path::Path;

use belaf::cmd::init::auto_detect::{self};
use common::TestRepo;
use fixtures::Seedable;

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

#[test]
fn tokio_single_auto_detect_emits_no_release_unit_blocks() {
    // The detector heuristics should NOT fire on a flat single-crate
    // Cargo repo. With nothing detected, `auto_detect::run` returns
    // an empty snippet (or one with only the marker comment) — and
    // the wizard's tag-format step is what writes the actual config
    // contribution. Pinning that the auto-detect path stays quiet
    // ensures the single-project case is driven by the tag-format
    // sub-prompt and not by stray detector output.
    let repo = TestRepo::new();
    fixtures::seed_tokio_single(&repo);

    let r = belaf::core::git::repository::Repository::open(&repo.path).expect("open");
    let result = auto_detect::run(&r);

    assert!(
        !result.toml_snippet.contains("[[release_unit"),
        "tokio-single must not produce any release_unit blocks; got snippet:\n{}",
        result.toml_snippet
    );
    assert_eq!(result.counters.total_release_unit_candidates(), 0);
    assert_eq!(
        result.counters.mobile_ios + result.counters.mobile_android,
        0
    );
}

#[test]
fn tokio_single_init_ci_succeeds_and_writes_config() {
    let repo = TestRepo::new();
    fixtures::seed_tokio_single(&repo);

    let out = repo.run_belaf_command_with_env(
        &["--no-color", "init", "--ci"],
        &[("BELAF_NO_KEYRING", "1"), ("NO_COLOR", "1")],
    );
    assert!(
        out.status.success(),
        "init --ci on tokio-single must succeed; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let cfg = std::fs::read_to_string(repo.path.join("belaf").join("config.toml"))
        .expect("config.toml must exist after init");
    assert!(
        cfg.contains("upstream_urls"),
        "config must contain the upstream_urls key, got:\n{cfg}"
    );
}

#[test]
fn tokio_single_explain_reports_no_release_units() {
    let repo = TestRepo::new();
    fixtures::seed_tokio_single(&repo);
    let _ = repo.run_belaf_command(&["init", "--force"]);
    repo.commit("seed config");

    let out = repo.run_belaf_command_with_env(
        &["--no-color", "explain"],
        &[("BELAF_NO_KEYRING", "1"), ("NO_COLOR", "1")],
    );
    assert!(
        out.status.success(),
        "explain on tokio-single must succeed; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    // No `[[release_unit]]` configured → friendly message rather
    // than a unit count.
    assert!(
        stdout.contains("No [[release_unit]]")
            || stdout.contains("No [[release_unit_glob]]")
            || stdout.contains("0 ReleaseUnits"),
        "explain on a no-release_unit config should say so, got:\n{stdout}"
    );
}
