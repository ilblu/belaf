//! DoD #10 — `belaf explain` against the clikd-shape fixture.
//!
//! Seeds the polyglot fixture, writes a canonical config.toml with
//! release_unit entries for each major bundle, runs the actual
//! `belaf explain` binary as a subprocess, and asserts the stdout
//! attributes every configured unit (origin / source / satellites /
//! tag_format) plus the drift snapshot.

mod common;
mod fixtures;

use std::path::Path;

use common::TestRepo;
use fixtures::Seedable;

/// owo_colors 4 with direct `.green()` calls emits ANSI even when
/// `set_override(false)` is honoured, because non-`if_supports_color`
/// styled values always serialise their escape codes. Strip them so
/// substring assertions don't have to encode the ANSI structure.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // ESC — consume `[…<final>` (final is in 0x40-0x7E).
            if chars.next() == Some('[') {
                for c2 in chars.by_ref() {
                    if matches!(c2, '@'..='~') {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

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

const CLIKD_CANONICAL_CONFIG: &str = r#"
upstream_urls = ["https://github.com/test/clikd"]

[[release_unit_glob]]
glob = "apps/services/*"
ecosystem = "cargo"
manifests = ["{path}/crates/bin/Cargo.toml"]
fallback_manifests = ["{path}/crates/workers/Cargo.toml"]
satellites = ["{path}/crates"]
name = "{basename}"

[[release_unit]]
name = "desktop"
ecosystem = "tauri"
satellites = ["apps/desktop"]
[[release_unit.source.manifests]]
path = "apps/desktop/package.json"
version_field = "npm_package_json"
[[release_unit.source.manifests]]
path = "apps/desktop/src-tauri/Cargo.toml"
version_field = "cargo_toml"
[[release_unit.source.manifests]]
path = "apps/desktop/src-tauri/tauri.conf.json"
version_field = "tauri_conf_json"

[[release_unit]]
name = "kotlin-sdk"
ecosystem = "jvm-library"
satellites = ["sdks/kotlin"]
[[release_unit.source.manifests]]
path = "sdks/kotlin/gradle.properties"
version_field = "gradle_properties"

[allow_uncovered]
paths = ["apps/mobile-ios/", "sdks/typescript/", "sdks/swift/"]
"#;

#[test]
fn explain_renders_every_release_unit_for_clikd_shape() {
    let repo = TestRepo::new();
    fixtures::seed_clikd_shape(&repo);
    repo.write_file("belaf/config.toml", CLIKD_CANONICAL_CONFIG);
    repo.commit("add canonical clikd config");

    let out = repo.run_belaf_command_with_env(
        &["--no-color", "explain"],
        &[("BELAF_NO_KEYRING", "1"), ("NO_COLOR", "1")],
    );

    let raw_stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "explain must exit 0, got {:?}\nstdout:\n{raw_stdout}\nstderr:\n{stderr}",
        out.status
    );
    let stdout = strip_ansi(&raw_stdout);

    // 3 services from the glob (aura, ekko, mondo) + desktop + kotlin-sdk = 5
    assert!(
        stdout.contains("Detected:") && stdout.contains("ReleaseUnits"),
        "explain must print the unit count line, got:\n{stdout}"
    );

    // Every configured unit should appear by name.
    for name in ["aura", "ekko", "mondo", "desktop", "kotlin-sdk"] {
        assert!(
            stdout.contains(name),
            "explain output must mention `{name}`, got:\n{stdout}"
        );
    }

    // mondo specifically uses fallback_manifests — its source line
    // must point at crates/workers, not crates/bin.
    let mondo_block = stdout
        .split("• mondo")
        .nth(1)
        .expect("mondo block must exist in output");
    let mondo_block = mondo_block.split("• ").next().unwrap_or(mondo_block);
    assert!(
        mondo_block.contains("crates/workers/Cargo.toml"),
        "mondo's source must be `crates/workers/Cargo.toml` (fallback_manifests path), got:\n{mondo_block}"
    );

    // Desktop is the Tauri triplet — three manifests in one source line.
    let desktop_block = stdout
        .split("• desktop")
        .nth(1)
        .expect("desktop block must exist");
    let desktop_block = desktop_block.split("• ").next().unwrap_or(desktop_block);
    for tauri_manifest in [
        "apps/desktop/package.json",
        "apps/desktop/src-tauri/Cargo.toml",
        "apps/desktop/src-tauri/tauri.conf.json",
    ] {
        assert!(
            desktop_block.contains(tauri_manifest),
            "desktop unit must list `{tauri_manifest}`, got:\n{desktop_block}"
        );
    }

    // Kotlin SDK uses gradle.properties as its version source.
    let kotlin_block = stdout
        .split("• kotlin-sdk")
        .nth(1)
        .expect("kotlin-sdk block must exist");
    let kotlin_block = kotlin_block.split("• ").next().unwrap_or(kotlin_block);
    assert!(
        kotlin_block.contains("sdks/kotlin/gradle.properties"),
        "kotlin-sdk must point at gradle.properties, got:\n{kotlin_block}"
    );

    // [allow_uncovered] echo
    assert!(
        stdout.contains("[allow_uncovered]"),
        "allow_uncovered echo must appear, got:\n{stdout}"
    );
    assert!(
        stdout.contains("apps/mobile-ios"),
        "iOS path must be listed under allow_uncovered, got:\n{stdout}"
    );

    // Drift section: with the canonical config above, the iOS app +
    // typescript/swift SDKs are in [allow_uncovered], so drift must
    // be silent. (If new bundles get added later, the drift section
    // would show — and this assert would catch it as a regression.)
    assert!(
        !stdout.contains("Drift detected"),
        "canonical clikd config must produce zero drift, got:\n{stdout}"
    );
}
