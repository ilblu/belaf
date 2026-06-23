//! F11a/F11b — per-unit bump policy overrides + prerelease generation.

mod common;

use common::TestRepo;

/// Seed a single root crate at `version`, write `belaf/config.toml`, commit a
/// lockfile (so prepare sees a clean tree), and tag the given release.
fn seed_single(repo: &TestRepo, name: &str, version: &str, config: &str, tag: &str) {
    repo.write_file(
        "Cargo.toml",
        &format!("[package]\nname = \"{name}\"\nversion = \"{version}\"\nedition = \"2021\"\n"),
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
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
    repo.tag(tag);
}

#[test]
fn max_bump_caps_minor_to_patch() {
    // F11a — max_bump = "patch" clamps a feat (which would be minor) to patch.
    let repo = TestRepo::new();
    seed_single(
        &repo,
        "mc",
        "1.0.0",
        "[release_unit.mc.bump]\nmax_bump = \"patch\"\n",
        "mc-v1.0.0",
    );
    repo.write_file("src/feature.rs", "pub fn feature() {}\n");
    repo.commit("feat: add a feature");

    let _ = repo.run_belaf_command_with_env(&["prepare", "--ci"], &[("BELAF_NO_KEYRING", "1")]);

    let cargo = repo.read_file("Cargo.toml");
    assert!(
        cargo.contains("version = \"1.0.1\""),
        "feat must be capped to patch (1.0.1), not 1.1.0; got:\n{cargo}"
    );
}

#[test]
fn prerelease_label_emits_beta_suffix() {
    // F11b — prerelease = "beta" produces `-beta.1` off the stable base.
    let repo = TestRepo::new();
    seed_single(
        &repo,
        "mc",
        "0.5.0",
        "[release_unit.mc.bump]\nprerelease = \"beta\"\nfeatures_always_bump_minor = true\n",
        "mc-v0.5.0",
    );
    repo.write_file("src/feature.rs", "pub fn feature() {}\n");
    repo.commit("feat: add a feature");

    let _ = repo.run_belaf_command_with_env(&["prepare", "--ci"], &[("BELAF_NO_KEYRING", "1")]);

    let cargo = repo.read_file("Cargo.toml");
    assert!(
        cargo.contains("0.6.0-beta.1"),
        "feat under prerelease=beta must yield 0.6.0-beta.1; got:\n{cargo}"
    );
}

#[test]
fn per_unit_breaking_policy_diverges_from_global() {
    // F11a — global breaking→major, but a per-unit override keeps one unit on a
    // 0.x (breaking→minor) track. svc → 1.0.0, desktop → 0.2.0.
    let repo = TestRepo::new();
    repo.write_file(
        "Cargo.toml",
        "[workspace]\nmembers = [\"packages/*\"]\nresolver = \"2\"\n",
    );
    repo.write_file(
        "packages/svc/Cargo.toml",
        "[package]\nname = \"svc\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    repo.write_file("packages/svc/src/lib.rs", "pub fn s() {}\n");
    repo.write_file(
        "packages/desktop/Cargo.toml",
        "[package]\nname = \"desktop\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    repo.write_file("packages/desktop/src/lib.rs", "pub fn d() {}\n");
    repo.write_file(
        "belaf/config.toml",
        "[bump]\nbreaking_always_bump_major = true\n\n[release_unit.desktop.bump]\nbreaking_always_bump_major = false\n",
    );
    repo.commit("Initial commit");
    let lock = std::process::Command::new("cargo")
        .args(["generate-lockfile"])
        .current_dir(&repo.path)
        .output()
        .expect("cargo generate-lockfile");
    assert!(lock.status.success());
    repo.commit("chore: lockfile");
    repo.tag("svc-v0.1.0");
    repo.tag("desktop-v0.1.0");

    repo.write_file("packages/svc/src/x.rs", "pub fn x() {}\n");
    repo.commit("feat(svc)!: breaking change");
    repo.write_file("packages/desktop/src/y.rs", "pub fn y() {}\n");
    repo.commit("feat(desktop)!: breaking change");

    let _ = repo.run_belaf_command_with_env(&["prepare", "--ci"], &[("BELAF_NO_KEYRING", "1")]);

    let svc = repo.read_file("packages/svc/Cargo.toml");
    let desktop = repo.read_file("packages/desktop/Cargo.toml");
    assert!(
        svc.contains("version = \"1.0.0\""),
        "svc breaking → major (global policy); got:\n{svc}"
    );
    assert!(
        desktop.contains("version = \"0.2.0\""),
        "desktop breaking → minor (per-unit override keeps 0.x); got:\n{desktop}"
    );
}

#[test]
fn prerelease_base_is_last_stable_across_multiple_betas() {
    // F11b (Risk 11, MUST) — the base for a prerelease comes from the last
    // STABLE tag, not the last prerelease tag, so changes accumulated across
    // betas are counted once. Stable 0.5.0, then two feats released as betas →
    // 0.6.0-beta.2 (NOT 0.7.0-beta.1, which a last-prerelease boundary yields).
    let repo = TestRepo::new();
    repo.write_file(
        "Cargo.toml",
        "[package]\nname = \"mc\"\nversion = \"0.5.0\"\nedition = \"2021\"\n",
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
    repo.write_file(
        "belaf/config.toml",
        "[release_unit.mc.bump]\nprerelease = \"beta\"\nfeatures_always_bump_minor = true\n",
    );
    repo.commit("Initial commit");
    let lock = std::process::Command::new("cargo")
        .args(["generate-lockfile"])
        .current_dir(&repo.path)
        .output()
        .expect("lockfile");
    assert!(lock.status.success());
    repo.commit("chore: lockfile");
    repo.tag("mc-v0.5.0"); // last STABLE release

    // feat #1, then simulate it having shipped as 0.6.0-beta.1.
    repo.write_file("src/f1.rs", "pub fn f1() {}\n");
    repo.commit("feat: first feature");
    repo.write_file(
        "Cargo.toml",
        "[package]\nname = \"mc\"\nversion = \"0.6.0-beta.1\"\nedition = \"2021\"\n",
    );
    let _ = std::process::Command::new("cargo")
        .args(["generate-lockfile"])
        .current_dir(&repo.path)
        .output();
    repo.commit("chore: release 0.6.0-beta.1");
    repo.tag("mc-v0.6.0-beta.1"); // a PRERELEASE tag (must be ignored for base)

    // feat #2 lands; the next prepare should yield 0.6.0-beta.2.
    repo.write_file("src/f2.rs", "pub fn f2() {}\n");
    repo.commit("feat: second feature");

    let _ = repo.run_belaf_command_with_env(&["prepare", "--ci"], &[("BELAF_NO_KEYRING", "1")]);

    let cargo = repo.read_file("Cargo.toml");
    assert!(
        cargo.contains("0.6.0-beta.2"),
        "base must come from the stable tag 0.5.0 (→ 0.6.0-beta.2), not the last \
         beta (which would give 0.7.0-beta.1); got:\n{cargo}"
    );
    assert!(
        !cargo.contains("0.7.0"),
        "base must NOT rise off the last prerelease tag; got:\n{cargo}"
    );
}
