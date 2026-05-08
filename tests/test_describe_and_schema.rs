//! Smoke tests for the agent-discovery surface: `belaf describe`,
//! `belaf schema`, and the stable exit-code contract. These are the
//! commands an AI agent will hit first on a fresh repo, so the
//! contract has to be tight.

use belaf::core::exit_code::ExitCode;

#[test]
fn exit_codes_are_distinct() {
    let codes: Vec<i32> = ExitCode::all().iter().map(|c| (*c).into()).collect();
    let mut sorted = codes.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(codes.len(), sorted.len(), "exit codes must be unique");
    assert_eq!(i32::from(ExitCode::Ok), 0, "Ok must always be 0");
}

#[test]
fn exit_code_labels_documented() {
    for code in ExitCode::all() {
        assert!(
            !code.label().is_empty(),
            "every exit code must have a non-empty label"
        );
        assert!(
            !code.description().is_empty(),
            "every exit code must have a description"
        );
    }
}

#[test]
fn schema_manifest_is_embedded_and_valid_json() {
    let cmd = std::process::Command::new(env!("CARGO_BIN_EXE_belaf"))
        .args(["schema", "manifest"])
        .env("BELAF_NO_KEYRING", "1")
        .env("BELAF_API_URL", "http://127.0.0.1:0")
        .output()
        .expect("run belaf schema manifest");
    assert!(cmd.status.success(), "exit code: {:?}", cmd.status);
    let stdout = String::from_utf8(cmd.stdout).expect("utf8");
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("schema must be JSON");
    assert!(
        parsed.get("title").is_some(),
        "manifest schema must have a title"
    );
    assert!(
        parsed.get("$schema").is_some(),
        "manifest schema must have a $schema field"
    );
}

#[test]
fn schema_unknown_name_errors() {
    let cmd = std::process::Command::new(env!("CARGO_BIN_EXE_belaf"))
        .args(["schema", "no-such-schema"])
        .env("BELAF_NO_KEYRING", "1")
        .env("BELAF_API_URL", "http://127.0.0.1:0")
        .output()
        .expect("run belaf schema bogus");
    assert!(!cmd.status.success(), "must fail on unknown schema");
}

#[test]
fn describe_json_has_required_keys() {
    let cmd = std::process::Command::new(env!("CARGO_BIN_EXE_belaf"))
        .args(["describe", "--json"])
        .env("BELAF_NO_KEYRING", "1")
        .env("BELAF_API_URL", "http://127.0.0.1:0")
        .output()
        .expect("run belaf describe --json");
    assert!(cmd.status.success(), "exit code: {:?}", cmd.status);
    let stdout = String::from_utf8(cmd.stdout).expect("utf8");
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("describe output must be JSON");
    for key in [
        "name",
        "version",
        "commands",
        "env_vars",
        "exit_codes",
        "schemas",
        "example_workflows",
    ] {
        assert!(
            parsed.get(key).is_some(),
            "describe output must contain `{key}`"
        );
    }
    let commands = parsed["commands"].as_array().expect("commands is array");
    assert!(!commands.is_empty(), "must have at least one command");
    // Ensure the discovery + agent-friendly commands themselves appear.
    let names: Vec<&str> = commands.iter().filter_map(|c| c["name"].as_str()).collect();
    for required in ["describe", "schema", "prepare", "explain"] {
        assert!(names.contains(&required), "missing command: {required}");
    }
}

#[test]
fn describe_default_is_json() {
    let cmd = std::process::Command::new(env!("CARGO_BIN_EXE_belaf"))
        .arg("describe")
        .env("BELAF_NO_KEYRING", "1")
        .env("BELAF_API_URL", "http://127.0.0.1:0")
        .output()
        .expect("run belaf describe");
    assert!(cmd.status.success());
    let stdout = String::from_utf8(cmd.stdout).expect("utf8");
    let _: serde_json::Value = serde_json::from_str(&stdout).expect("default output must be JSON");
}

#[test]
fn describe_text_renders_human_readable() {
    let cmd = std::process::Command::new(env!("CARGO_BIN_EXE_belaf"))
        .args(["describe", "--text"])
        .env("BELAF_NO_KEYRING", "1")
        .env("BELAF_API_URL", "http://127.0.0.1:0")
        .output()
        .expect("run belaf describe --text");
    assert!(cmd.status.success());
    let stdout = String::from_utf8(cmd.stdout).expect("utf8");
    assert!(stdout.starts_with("belaf "), "text output starts with name");
    assert!(stdout.contains("COMMANDS"));
    assert!(stdout.contains("EXIT CODES"));
}
