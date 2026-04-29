//! Reusable test fixtures for the release-unit + drift + auto-detect
//! pipelines. Each `seed_*` function takes a fresh `TestRepo` and
//! populates its working tree.
//!
//! This module is loaded as a sibling test-helper from individual
//! integration tests via `mod fixtures;`. It deliberately doesn't
//! reference `common::TestRepo` directly — sub-modules of integration
//! test binaries can't share parent-module siblings — so callers
//! pass in a TestRepo via the generic [`Seedable`] trait.

#![allow(dead_code)]

use std::path::Path;
use std::process::Command;

/// Anything an integration test can hand to a fixture seeder.
/// `tests/common.rs::TestRepo` is the obvious impl; tests in this
/// crate provide it locally.
pub trait Seedable {
    fn root(&self) -> &Path;
    fn write_file(&self, relative: &str, content: &str);
    fn commit(&self, message: &str);
}

/// `clikd-shape` — full polyglot monorepo modeling the user's
/// clikd repo. Hits every detector at least once:
///
/// - `apps/services/{aura,ekko,mondo}` — three hexagonal-cargo
///   services. mondo only has `crates/workers` (exercises the
///   `fallback_manifests` resolver path).
/// - `apps/desktop` — Tauri (single-source: package.json +
///   src-tauri/Cargo.toml + src-tauri/tauri.conf.json with the
///   conf referencing `../package.json`).
/// - `apps/mobile-ios` — iOS app (xcodeproj/project.pbxproj).
/// - `sdks/kotlin` — JVM library with `gradle.properties`
///   carrying `version = ...`.
/// - `sdks/typescript`, `sdks/swift` — SDK-cascade members.
pub fn seed_clikd_shape<R: Seedable>(repo: &R) {
    // Cargo services (hexagonal layout)
    repo.write_file(
        "apps/services/aura/crates/bin/Cargo.toml",
        "[package]\nname = \"aura-bin\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    repo.write_file(
        "apps/services/aura/crates/bin/src/main.rs",
        "fn main() {}\n",
    );
    repo.write_file(
        "apps/services/aura/crates/api/Cargo.toml",
        "[package]\nname = \"aura-api\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    repo.write_file(
        "apps/services/aura/crates/api/src/lib.rs",
        "pub fn handler() {}\n",
    );

    repo.write_file(
        "apps/services/ekko/crates/bin/Cargo.toml",
        "[package]\nname = \"ekko-bin\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    );
    repo.write_file(
        "apps/services/ekko/crates/bin/src/main.rs",
        "fn main() {}\n",
    );

    // mondo has only `workers` — exercises fallback_manifests path.
    repo.write_file(
        "apps/services/mondo/crates/workers/Cargo.toml",
        "[package]\nname = \"mondo-workers\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    );
    repo.write_file(
        "apps/services/mondo/crates/workers/src/main.rs",
        "fn main() {}\n",
    );
    repo.write_file(
        "apps/services/mondo/crates/core/Cargo.toml",
        "[package]\nname = \"mondo-core\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    );
    repo.write_file(
        "apps/services/mondo/crates/core/src/lib.rs",
        "pub fn x() {}\n",
    );

    // Tauri desktop (single-source)
    repo.write_file(
        "apps/desktop/package.json",
        "{\n  \"name\": \"clikd-desktop\",\n  \"version\": \"0.3.0\"\n}\n",
    );
    repo.write_file(
        "apps/desktop/src-tauri/Cargo.toml",
        "[package]\nname = \"clikd-desktop\"\nversion = \"0.3.0\"\nedition = \"2021\"\n",
    );
    repo.write_file(
        "apps/desktop/src-tauri/tauri.conf.json",
        "{\n  \"package\": {\n    \"version\": \"../package.json\"\n  }\n}\n",
    );
    repo.write_file("apps/desktop/src-tauri/src/main.rs", "fn main() {}\n");

    // iOS app — should land in [allow_uncovered] via auto-detect.
    repo.write_file(
        "apps/mobile-ios/Clikd.xcodeproj/project.pbxproj",
        "// dummy pbxproj\n",
    );

    // JVM SDK with gradle.properties version source
    repo.write_file(
        "sdks/kotlin/build.gradle.kts",
        "plugins {\n  kotlin(\"jvm\") version \"1.9.0\"\n}\n",
    );
    repo.write_file("sdks/kotlin/gradle.properties", "version=0.5.0\n");
    repo.write_file(
        "sdks/kotlin/src/main/kotlin/Schema.kt",
        "package com.clikd\nclass Schema\n",
    );

    // SDK cascade members (under sdks/*)
    repo.write_file(
        "sdks/typescript/package.json",
        "{\n  \"name\": \"@clikd/typescript-sdk\",\n  \"version\": \"0.5.0\"\n}\n",
    );
    repo.write_file(
        "sdks/swift/Package.swift",
        "// swift-tools-version:5.5\nimport PackageDescription\n",
    );

    // Schema satellite — generates SDKs but isn't a release unit on its own
    repo.write_file("proto/events/v1/schema.graphql", "type Event { id: ID! }\n");

    Command::new("git")
        .args(["add", "-A"])
        .current_dir(repo.root())
        .output()
        .expect("git add");
    repo.commit("seed clikd-shape fixture");
}
