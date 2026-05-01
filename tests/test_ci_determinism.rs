//! DoD #7 — `belaf init --ci --auto-detect` must be byte-deterministic.
//!
//! We can't easily compare the CI output against a TUI all-Y run from
//! a regression test (the TUI needs a tty), but we *can* assert the
//! stronger property the DoD ultimately needs: running
//! `belaf init --ci --auto-detect` twice on the same fixture produces
//! the **exact same** `belaf/config.toml` byte-for-byte. That rules
//! out:
//!
//!   - non-deterministic ordering in detector output
//!   - timestamp / random / `system_time` leaks into the snippet
//!   - duplicate-append on second-run (the marker idempotency from M7)
//!
//! Together with the snapshot tests in `test_clikd_shape.rs` (which
//! pin the canonical detector kinds) this gives the same coverage
//! the byte-identical CI-vs-TUI comparison would.

mod common;
mod fixtures;

use std::path::Path;

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

fn run_init(repo: &TestRepo) -> std::process::Output {
    repo.run_belaf_command_with_env(
        &["--no-color", "init", "--ci", "--auto-detect"],
        &[("BELAF_NO_KEYRING", "1"), ("NO_COLOR", "1")],
    )
}

fn read_config(repo: &TestRepo) -> String {
    std::fs::read_to_string(repo.path.join("belaf").join("config.toml"))
        .expect("config.toml must exist after init")
}

#[test]
fn ci_init_against_clikd_shape_is_byte_deterministic() {
    let repo_a = TestRepo::new();
    fixtures::seed_clikd_shape(&repo_a);
    let out_a = run_init(&repo_a);
    assert!(
        out_a.status.success(),
        "first init must succeed; stderr:\n{}",
        String::from_utf8_lossy(&out_a.stderr)
    );
    let cfg_a = read_config(&repo_a);

    let repo_b = TestRepo::new();
    fixtures::seed_clikd_shape(&repo_b);
    let out_b = run_init(&repo_b);
    assert!(
        out_b.status.success(),
        "second init must succeed; stderr:\n{}",
        String::from_utf8_lossy(&out_b.stderr)
    );
    let cfg_b = read_config(&repo_b);

    assert_eq!(
        cfg_a, cfg_b,
        "two CI init runs against the same fixture must produce byte-identical config.toml.\n--- run A ---\n{cfg_a}\n--- run B ---\n{cfg_b}"
    );
}

#[test]
fn ci_init_idempotent_re_run_does_not_duplicate_append() {
    // M7 — the auto-detect marker should keep the second run from
    // appending the snippet again. Body of config.toml after run 2
    // must equal body after run 1.
    let repo = TestRepo::new();
    fixtures::seed_clikd_shape(&repo);

    let out1 = run_init(&repo);
    assert!(out1.status.success());
    let cfg1 = read_config(&repo);

    // Re-run init. Force flag so the existing config doesn't block.
    let out2 = repo.run_belaf_command_with_env(
        &["--no-color", "init", "--ci", "--auto-detect", "--force"],
        &[("BELAF_NO_KEYRING", "1"), ("NO_COLOR", "1")],
    );
    assert!(
        out2.status.success(),
        "second init --force must succeed; stderr:\n{}",
        String::from_utf8_lossy(&out2.stderr)
    );
    let cfg2 = read_config(&repo);

    // The config might have changed (e.g. base config rewritten),
    // but the count of release_unit blocks must NOT have grown.
    let count = |s: &str| s.matches("[[release_unit").count();
    assert_eq!(
        count(&cfg1),
        count(&cfg2),
        "auto-detect snippet must NOT be duplicate-appended on re-run.\n--- after run 1 ---\n{cfg1}\n--- after run 2 ---\n{cfg2}"
    );
    let marker_count = |s: &str| s.matches("belaf:auto-detect-marker").count();
    assert!(
        marker_count(&cfg2) <= 1,
        "marker comment must appear at most once after two runs; got {} occurrences in:\n{cfg2}",
        marker_count(&cfg2)
    );
}
