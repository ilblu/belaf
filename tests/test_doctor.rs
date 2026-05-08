//! Smoke tests for `belaf doctor`. Doctor's contract is the JSON
//! payload — agents will key off `status` per check and the overall
//! `ok` boolean. These tests exercise the contract from the binary
//! boundary, not the internal API, so refactors don't silently break
//! agents.

use std::process::Command;

fn doctor_cmd(json: bool) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_belaf"));
    cmd.arg("doctor");
    if json {
        cmd.arg("--json");
    }
    cmd.env("BELAF_NO_KEYRING", "1")
        // Unreachable port — keeps the API health probe hermetic
        // (no network dependency) and fast (connect-refused returns
        // immediately rather than waiting for the 3s timeout).
        .env("BELAF_API_URL", "http://127.0.0.1:9");
    // Run from a tempdir so the config check is deterministic
    // (the belaf project repo itself doesn't have belaf/config.toml).
    let dir = tempfile::tempdir().unwrap();
    // git init inside so the repository check passes.
    std::process::Command::new("git")
        .arg("init")
        .current_dir(dir.path())
        .output()
        .unwrap();
    cmd.current_dir(dir.path());
    let out = cmd.output().expect("run belaf doctor");
    drop(dir);
    out
}

#[test]
fn doctor_json_has_required_keys() {
    let out = doctor_cmd(true);
    assert!(out.status.success() || !out.status.success(), "must exit");
    let stdout = String::from_utf8(out.stdout).expect("utf8");
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("doctor --json must emit JSON");
    for key in [
        "ok",
        "auth",
        "config",
        "repository",
        "ecosystems",
        "api",
        "environment",
    ] {
        assert!(
            parsed.get(key).is_some(),
            "doctor output must contain `{key}`"
        );
    }
    let env = &parsed["environment"];
    for key in [
        "api_url",
        "api_url_overridden",
        "web_url",
        "web_url_overridden",
        "keyring_disabled",
        "ci_detected",
        "ci_signals",
    ] {
        assert!(env.get(key).is_some(), "environment must contain `{key}`");
    }
}

#[test]
fn doctor_check_status_values_are_valid() {
    let out = doctor_cmd(true);
    let stdout = String::from_utf8(out.stdout).expect("utf8");
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    for check_key in ["auth", "config", "repository", "ecosystems", "api"] {
        let s = parsed[check_key]["status"]
            .as_str()
            .unwrap_or_else(|| panic!("{check_key}.status must be a string"));
        assert!(
            matches!(s, "ok" | "warn" | "error" | "skipped"),
            "{check_key}.status was {s:?} — must be one of ok/warn/error/skipped"
        );
    }
}

#[test]
fn doctor_text_output_is_not_json() {
    let out = doctor_cmd(false);
    let stdout = String::from_utf8(out.stdout).expect("utf8");
    assert!(
        !stdout.trim_start().starts_with('{'),
        "default text output must not be JSON"
    );
    assert!(stdout.contains("doctor"));
}

#[test]
fn doctor_reports_keyring_disabled_in_environment() {
    let out = doctor_cmd(true);
    let stdout = String::from_utf8(out.stdout).expect("utf8");
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        parsed["environment"]["keyring_disabled"], true,
        "BELAF_NO_KEYRING was set in env, must be reflected"
    );
}

#[test]
fn doctor_exit_code_reflects_overall_status() {
    let out = doctor_cmd(true);
    let stdout = String::from_utf8(out.stdout).expect("utf8");
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let ok = parsed["ok"].as_bool().expect("ok must be bool");
    if ok {
        assert!(out.status.success(), "ok=true → exit 0");
    } else {
        // Precondition exit code (4) per the stable exit-code contract.
        assert_eq!(
            out.status.code(),
            Some(4),
            "ok=false → exit 4 (precondition)"
        );
    }
}

#[test]
fn doctor_lists_ci_signals_when_present() {
    // Run a fresh process with CI=1 set explicitly.
    let dir = tempfile::tempdir().unwrap();
    std::process::Command::new("git")
        .arg("init")
        .current_dir(dir.path())
        .output()
        .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_belaf"))
        .args(["doctor", "--json"])
        .env("BELAF_NO_KEYRING", "1")
        // Unreachable port — keeps the API health probe hermetic
        // (no network dependency) and fast (connect-refused returns
        // immediately rather than waiting for the 3s timeout).
        .env("BELAF_API_URL", "http://127.0.0.1:9")
        .env("CI", "1")
        .current_dir(dir.path())
        .output()
        .expect("run belaf doctor with CI set");
    let stdout = String::from_utf8(out.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        parsed["environment"]["ci_detected"], true,
        "CI=1 must be detected"
    );
    let signals = parsed["environment"]["ci_signals"].as_array().unwrap();
    assert!(
        signals.iter().any(|v| v == "CI"),
        "CI signal must appear in ci_signals (got {:?})",
        signals
    );
}
