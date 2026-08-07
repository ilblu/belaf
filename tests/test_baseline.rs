//! Per-unit history baselines: the `baseline` key on `[release_unit.<name>]`
//! and the `belaf baseline` command that reports and writes it.
//!
//! Two things are under test here. First, that `belaf prepare` names **every**
//! untagged deploy unit in one error instead of bailing on the first — without
//! that, adopting `baseline` is a whack-a-mole loop of one unit per run.
//! Second, that `baseline` actually scopes to one unit: `"first-release"`
//! unblocks exactly the unit it is set on, a sha bounds that unit's commit
//! window, and every other unit keeps the guard.

mod common;

use common::TestRepo;

/// A cargo workspace with three members. Only `alpha` gets a release tag, so
/// the repo has version-shaped tags while `beta` and `gamma` have none — the
/// exact shape that trips the untagged-deploy-unit guard.
fn seed_three_units(repo: &TestRepo, config: &str) {
    repo.write_file(
        "Cargo.toml",
        "[workspace]\nmembers = [\"packages/*\"]\nresolver = \"2\"\n",
    );
    for name in ["alpha", "beta", "gamma"] {
        repo.write_file(
            &format!("packages/{name}/Cargo.toml"),
            &format!("[package]\nname = \"{name}\"\nversion = \"1.0.0\"\nedition = \"2021\"\n"),
        );
        repo.write_file(&format!("packages/{name}/src/lib.rs"), "pub fn f() {}\n");
    }
    repo.write_file("belaf/config.toml", config);
    repo.commit("Initial commit");
    let lock = std::process::Command::new("cargo")
        .args(["generate-lockfile"])
        .current_dir(&repo.path)
        .output()
        .expect("cargo generate-lockfile");
    assert!(
        lock.status.success(),
        "lockfile: {}",
        String::from_utf8_lossy(&lock.stderr)
    );
    repo.commit("chore: lockfile");
    repo.tag("alpha-v1.0.0");
}

fn touch_and_commit(repo: &TestRepo, unit: &str, file: &str, message: &str) {
    repo.write_file(
        &format!("packages/{unit}/src/{file}"),
        &format!("pub fn {} () {{}}\n", file.trim_end_matches(".rs")),
    );
    repo.commit(message);
}

// ---------------------------------------------------------------------------
// Part 1 — every violation in one error
// ---------------------------------------------------------------------------

#[test]
fn every_untagged_deploy_unit_is_reported_in_one_error() {
    let repo = TestRepo::new();
    seed_three_units(&repo, "# no baselines yet\n");
    touch_and_commit(&repo, "beta", "b.rs", "feat(beta): something");
    touch_and_commit(&repo, "gamma", "g.rs", "feat(gamma): something");

    let output =
        repo.run_belaf_command_with_env(&["prepare", "--ci"], &[("BELAF_NO_KEYRING", "1")]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "the guard must still refuse the run; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("`beta` (tried template"),
        "beta must be named; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("`gamma` (tried template"),
        "gamma must be named in the SAME error — reporting one unit per run is \
         the bug this replaces; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("2 release units"),
        "the error must say how many units it found; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("baseline = \"first-release\""),
        "the error must point at the per-unit fix; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("belaf baseline --fix"),
        "the error must point at the command that writes it; stderr:\n{stderr}"
    );
}

// ---------------------------------------------------------------------------
// Part 2 — `baseline` scopes to exactly one unit
// ---------------------------------------------------------------------------

#[test]
fn first_release_unblocks_exactly_one_unit() {
    let repo = TestRepo::new();
    seed_three_units(&repo, "[release_unit.beta]\nbaseline = \"first-release\"\n");
    touch_and_commit(&repo, "beta", "b.rs", "feat(beta): something");
    touch_and_commit(&repo, "gamma", "g.rs", "feat(gamma): something");

    let output =
        repo.run_belaf_command_with_env(&["prepare", "--ci"], &[("BELAF_NO_KEYRING", "1")]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !stderr.contains("`beta` (tried template"),
        "beta declared `baseline = \"first-release\"` and must no longer be \
         reported; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("`gamma` (tried template"),
        "gamma declared nothing and must still be guarded — the key is per-unit, \
         not repo-wide; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("1 release unit,"),
        "exactly one unit must remain unanswered; stderr:\n{stderr}"
    );
}

#[test]
fn first_release_on_every_untagged_unit_lets_the_run_proceed() {
    let repo = TestRepo::new();
    seed_three_units(
        &repo,
        "[release_unit.beta]\nbaseline = \"first-release\"\n\n\
         [release_unit.gamma]\nbaseline = \"first-release\"\n",
    );
    touch_and_commit(&repo, "beta", "b.rs", "feat(beta): something");

    let output =
        repo.run_belaf_command_with_env(&["prepare", "--ci"], &[("BELAF_NO_KEYRING", "1")]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !stderr.contains("could not locate a previous-release tag"),
        "with every untagged unit answered the guard must not fire at all; \
         stderr:\n{stderr}"
    );
    let beta = repo.read_file("packages/beta/Cargo.toml");
    assert!(
        beta.contains("version = \"1.1.0\""),
        "beta's feat must produce a real bump off its manifest version; got:\n{beta}"
    );
}

#[test]
fn baseline_sha_bounds_the_commit_window() {
    let repo = TestRepo::new();
    seed_three_units(
        &repo,
        "[release_unit.gamma]\nbaseline = \"first-release\"\n",
    );

    // Before the baseline: a feature that must NOT be counted.
    touch_and_commit(&repo, "beta", "early.rs", "feat(beta): early feature");
    let cutoff = repo.git(&["rev-parse", "--short", "HEAD"]);
    assert!(!cutoff.is_empty(), "must have resolved a short sha");

    // Point beta's baseline at that commit. A short sha on purpose —
    // `revparse_single` resolves it, and that is the form a human types.
    repo.write_file(
        "belaf/config.toml",
        &format!(
            "[release_unit.gamma]\nbaseline = \"first-release\"\n\n\
             [release_unit.beta]\nbaseline = \"{cutoff}\"\n"
        ),
    );
    repo.commit("chore: pin beta's baseline");

    // After the baseline: the only change that must land in the window.
    touch_and_commit(&repo, "beta", "later.rs", "fix(beta): later fix");

    let output =
        repo.run_belaf_command_with_env(&["prepare", "--ci"], &[("BELAF_NO_KEYRING", "1")]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("could not locate a previous-release tag"),
        "a sha baseline must satisfy the guard; stderr:\n{stderr}"
    );

    // The changelog template sentence-cases the subject, so compare lowercased.
    let changelog = repo.read_file("packages/beta/CHANGELOG.md");
    let lowered = changelog.to_lowercase();
    assert!(
        lowered.contains("later fix"),
        "the commit after the baseline must be in the window; got:\n{changelog}"
    );
    assert!(
        !lowered.contains("early feature"),
        "the commit at/before the baseline must be excluded — that is the whole \
         point of a sha baseline; got:\n{changelog}"
    );

    // And the bump follows the bounded window: only a `fix` is visible, so
    // patch, not the minor the pre-baseline `feat` would have forced.
    let beta = repo.read_file("packages/beta/Cargo.toml");
    assert!(
        beta.contains("version = \"1.0.1\""),
        "only the post-baseline fix may drive the bump; got:\n{beta}"
    );
}

#[test]
fn an_unresolvable_baseline_sha_is_a_hard_error_naming_the_unit() {
    let repo = TestRepo::new();
    seed_three_units(
        &repo,
        "[release_unit.beta]\nbaseline = \"0000000000000000000000000000000000000000\"\n\n\
         [release_unit.gamma]\nbaseline = \"first-release\"\n",
    );
    touch_and_commit(&repo, "beta", "b.rs", "feat(beta): something");

    let output =
        repo.run_belaf_command_with_env(&["prepare", "--ci"], &[("BELAF_NO_KEYRING", "1")]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success(), "stderr:\n{stderr}");
    assert!(
        stderr.contains("release_unit `beta`"),
        "the error must name the offending unit; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("baseline ="),
        "the error must name the offending key; stderr:\n{stderr}"
    );
}

#[test]
fn an_empty_baseline_value_is_rejected_at_config_load() {
    let repo = TestRepo::new();
    seed_three_units(&repo, "[release_unit.beta]\nbaseline = \"\"\n");

    let output = repo.run_belaf_command_with_env(&["baseline", "--ci"], &[]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "stderr:\n{stderr}");
    assert!(
        stderr.contains("`baseline` is empty"),
        "an empty value is a typo, not \"no baseline\"; stderr:\n{stderr}"
    );
}

// ---------------------------------------------------------------------------
// Part 3 — `belaf baseline`
// ---------------------------------------------------------------------------

fn parse_ci_json(output: &std::process::Output) -> serde_json::Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "--ci stdout must be pure JSON ({e}); stdout:\n{stdout}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn baseline_ci_lists_the_untagged_units() {
    let repo = TestRepo::new();
    seed_three_units(&repo, "# nothing declared\n");

    let output = repo.run_belaf_command_with_env(&["baseline", "--ci"], &[]);
    assert_eq!(
        output.status.code(),
        Some(4),
        "unanswered units are a precondition failure; stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json = parse_ci_json(&output);
    assert_eq!(json["status"], "needs_baseline");
    let names: Vec<&str> = json["units"]
        .as_array()
        .expect("units must be an array")
        .iter()
        .map(|u| u["name"].as_str().expect("name must be a string"))
        .collect();
    assert!(names.contains(&"beta"), "{json}");
    assert!(names.contains(&"gamma"), "{json}");
    assert!(
        !names.contains(&"alpha"),
        "alpha has a matching tag and must not be listed; {json}"
    );
    assert!(
        json["units"][0]["tag_template"]
            .as_str()
            .is_some_and(|t| t.contains("{version}")),
        "each entry must carry the template that missed; {json}"
    );
    assert!(json["written"].as_array().expect("written").is_empty());
}

#[test]
fn baseline_ci_reports_ok_when_nothing_is_unanswered() {
    let repo = TestRepo::new();
    seed_three_units(
        &repo,
        "[release_unit.beta]\nbaseline = \"first-release\"\n\n\
         [release_unit.gamma]\nbaseline = \"first-release\"\n",
    );

    let output = repo.run_belaf_command_with_env(&["baseline", "--ci"], &[]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_ci_json(&output);
    assert_eq!(json["status"], "ok");
    assert!(
        json["units"].as_array().expect("units").is_empty(),
        "{json}"
    );
}

#[test]
fn baseline_fix_writes_the_keys_preserves_comments_and_is_idempotent() {
    let repo = TestRepo::new();
    seed_three_units(
        &repo,
        "# hand-written header that must survive\n[bump]\nfeatures_always_bump_minor = true\n",
    );

    let first = repo.run_belaf_command_with_env(&["baseline", "--fix"], &[]);
    assert_eq!(
        first.status.code(),
        Some(0),
        "a --fix that answered everything must exit 0; stderr:\n{}",
        String::from_utf8_lossy(&first.stderr)
    );

    let after_first = repo.read_file("belaf/config.toml");
    assert!(
        after_first.contains("# hand-written header that must survive"),
        "surrounding comments must be preserved:\n{after_first}"
    );
    assert!(
        after_first.contains("features_always_bump_minor = true"),
        "existing keys must be preserved:\n{after_first}"
    );
    assert!(after_first.contains("[release_unit.beta]"), "{after_first}");
    assert!(
        after_first.contains("[release_unit.gamma]"),
        "{after_first}"
    );
    assert_eq!(
        after_first.matches("baseline = \"first-release\"").count(),
        2,
        "one key per reported unit, and none for the tagged `alpha`:\n{after_first}"
    );
    assert!(
        after_first.contains("`belaf baseline --fix`: no release tag matched"),
        "each generated key must explain itself:\n{after_first}"
    );

    // Second run: the units now resolve, so there is nothing to report and
    // nothing to write. The file must not be touched at all.
    let second = repo.run_belaf_command_with_env(&["baseline", "--fix"], &[]);
    assert_eq!(
        second.status.code(),
        Some(0),
        "stderr:\n{}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(
        repo.read_file("belaf/config.toml"),
        after_first,
        "a second --fix must leave the config byte-identical"
    );

    // And the written config actually unblocks the run it was written for.
    repo.commit("chore: baselines");
    touch_and_commit(&repo, "beta", "b.rs", "feat(beta): something");
    let prepare =
        repo.run_belaf_command_with_env(&["prepare", "--ci"], &[("BELAF_NO_KEYRING", "1")]);
    let stderr = String::from_utf8_lossy(&prepare.stderr);
    assert!(
        !stderr.contains("could not locate a previous-release tag"),
        "the keys --fix wrote must satisfy the guard they were written for; \
         stderr:\n{stderr}"
    );
}

#[test]
fn baseline_fix_leaves_an_existing_baseline_alone() {
    let repo = TestRepo::new();
    seed_three_units(&repo, "[release_unit.beta]\nbaseline = \"first-release\"\n");

    let output = repo.run_belaf_command_with_env(&["baseline", "--fix"], &[]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let after = repo.read_file("belaf/config.toml");
    assert_eq!(
        after.matches("[release_unit.beta]").count(),
        1,
        "beta already had a baseline and must not be re-emitted:\n{after}"
    );
    assert!(
        after.contains("[release_unit.gamma]"),
        "gamma was the only unanswered unit and must have been written:\n{after}"
    );
    assert!(
        !after
            .split("[release_unit.gamma]")
            .next()
            .expect("split always yields a first element")
            .contains("`belaf baseline --fix`: no release tag matched"),
        "the pre-existing beta key must not have picked up a generated comment:\n{after}"
    );
}

/// A glob-expanded unit has no `[release_unit.<its-name>]` block of its own,
/// and a bare one would be read as a *partial override* of an auto-detected
/// unit that does not exist — `resolve_partial_against_discovered` rejects it
/// and the config stops loading entirely. So `--fix` must refuse to write one
/// and say why, instead of "fixing" the config into an unloadable state.
#[test]
fn baseline_fix_refuses_to_write_a_block_for_a_glob_expanded_unit() {
    let repo = TestRepo::new();
    repo.write_file(
        "packages/alpha/package.json",
        "{\"name\":\"alpha\",\"version\":\"1.0.0\"}\n",
    );
    for svc in ["svc1", "svc2"] {
        repo.write_file(
            &format!("services/{svc}/package.json"),
            &format!("{{\"name\":\"{svc}\",\"version\":\"1.0.0\"}}\n"),
        );
    }
    let config = "[release_unit.services]\necosystem = \"npm\"\nglob = \"services/*\"\n\
                  name = \"{basename}\"\nmanifests = [\"{path}/package.json\"]\n";
    repo.write_file("belaf/config.toml", config);
    repo.commit("Initial commit");
    repo.tag("alpha@v1.0.0");

    let output = repo.run_belaf_command_with_env(&["baseline", "--ci", "--fix"], &[]);
    assert_eq!(
        output.status.code(),
        Some(4),
        "nothing was fixed, so this must not read as success; stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json = parse_ci_json(&output);
    assert!(
        json["written"].as_array().expect("written").is_empty(),
        "{json}"
    );
    let skipped: Vec<&str> = json["skipped"]
        .as_array()
        .expect("skipped must be an array")
        .iter()
        .map(|s| s["config_key"].as_str().expect("config_key"))
        .collect();
    assert_eq!(skipped, vec!["svc1", "svc2"], "{json}");

    assert_eq!(
        repo.read_file("belaf/config.toml"),
        config,
        "the config must be untouched"
    );

    // The proof that refusing was right: a hand-written bare block for a
    // glob expansion is exactly what breaks the config.
    repo.write_file(
        "belaf/config.toml",
        &format!("{config}\n[release_unit.svc1]\nbaseline = \"first-release\"\n"),
    );
    let broken = repo.run_belaf_command_with_env(&["baseline", "--ci"], &[]);
    let stderr = String::from_utf8_lossy(&broken.stderr);
    assert!(
        stderr.contains("partial override"),
        "a bare block for a glob expansion must fail to resolve — this is what \
         --fix is avoiding; stderr:\n{stderr}"
    );
}
