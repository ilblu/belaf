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
        "// swift-tools-version:5.5\n\
         import PackageDescription\n\
         \n\
         let package = Package(\n  \
           name: \"clikd-swift-sdk\",\n  \
           products: [.library(name: \"ClikdSwiftSdk\", targets: [\"ClikdSwiftSdk\"])],\n  \
           targets: [.target(name: \"ClikdSwiftSdk\")]\n\
         )\n",
    );
    repo.write_file(
        "sdks/swift/Sources/ClikdSwiftSdk/ClikdSwiftSdk.swift",
        "public struct ClikdSwiftSdk {}\n",
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

/// L.2 — `lerna-fixed`. A Lerna monorepo using **fixed** versioning
/// (every package shares one version, bumped together via a top-level
/// `lerna.json`). Detector should see two npm packages; ReleaseUnit
/// config models them as a single Group.
pub fn seed_lerna_fixed<R: Seedable>(repo: &R) {
    repo.write_file(
        "lerna.json",
        "{\n  \"version\": \"1.2.3\",\n  \"packages\": [\"packages/*\"]\n}\n",
    );
    repo.write_file(
        "package.json",
        "{\n  \"name\": \"lerna-root\",\n  \"private\": true,\n  \"workspaces\": [\"packages/*\"]\n}\n",
    );
    repo.write_file(
        "packages/core/package.json",
        "{\n  \"name\": \"@lerna-fixed/core\",\n  \"version\": \"1.2.3\"\n}\n",
    );
    repo.write_file("packages/core/index.js", "module.exports = {};\n");
    repo.write_file(
        "packages/utils/package.json",
        "{\n  \"name\": \"@lerna-fixed/utils\",\n  \"version\": \"1.2.3\"\n}\n",
    );
    repo.write_file("packages/utils/index.js", "module.exports = {};\n");

    Command::new("git")
        .args(["add", "-A"])
        .current_dir(repo.root())
        .output()
        .expect("git add");
    repo.commit("seed lerna-fixed fixture");
}

/// L.3 — `tokio-single`. A single-crate Cargo repo (mirroring the
/// shape of a published library: workspace = [".",], one [package]
/// at the root, no internal sub-crates). Drives the single-project
/// tag-format prompt path.
pub fn seed_tokio_single<R: Seedable>(repo: &R) {
    repo.write_file(
        "Cargo.toml",
        "[package]\n\
         name = \"tokio-like\"\n\
         version = \"1.30.0\"\n\
         edition = \"2021\"\n\
         description = \"single-crate library, tokio-shape\"\n\
         license = \"MIT\"\n",
    );
    repo.write_file(
        "src/lib.rs",
        "//! tokio-shape single-crate library.\npub fn spawn() {}\n",
    );

    Command::new("git")
        .args(["add", "-A"])
        .current_dir(repo.root())
        .output()
        .expect("git add");
    repo.commit("seed tokio-single fixture");
}

/// L.4 — `cargo-monorepo-independent`. A Cargo workspace where each
/// member has its **own** independent version (no `[workspace.package]
/// version`-inheritance). Ensures the loader walks per-crate `[package]
/// version` instead of falling back to a workspace-level value.
pub fn seed_cargo_monorepo_independent<R: Seedable>(repo: &R) {
    repo.write_file(
        "Cargo.toml",
        "[workspace]\nresolver = \"2\"\nmembers = [\"crates/alpha\", \"crates/beta\"]\n",
    );
    repo.write_file(
        "crates/alpha/Cargo.toml",
        "[package]\nname = \"alpha\"\nversion = \"0.1.7\"\nedition = \"2021\"\n",
    );
    repo.write_file("crates/alpha/src/lib.rs", "pub fn a() {}\n");
    repo.write_file(
        "crates/beta/Cargo.toml",
        "[package]\nname = \"beta\"\nversion = \"2.4.0\"\nedition = \"2021\"\n",
    );
    repo.write_file("crates/beta/src/lib.rs", "pub fn b() {}\n");

    Command::new("git")
        .args(["add", "-A"])
        .current_dir(repo.root())
        .output()
        .expect("git add");
    repo.commit("seed cargo-monorepo-independent fixture");
}

/// L.5 — `polyglot-cross-eco-group`. A `[[group]]` shape where the
/// same logical artefact ships as both an npm package AND a Maven
/// package — the canonical motivation for the Group primitive.
/// Versions in lockstep; one `[[group]]` member set in config.toml
/// drives both bumps.
pub fn seed_polyglot_cross_eco_group<R: Seedable>(repo: &R) {
    repo.write_file(
        "ts/package.json",
        "{\n  \"name\": \"@org/schema\",\n  \"version\": \"0.4.0\"\n}\n",
    );
    repo.write_file("ts/index.d.ts", "export interface Schema { id: string }\n");
    repo.write_file(
        "jvm/pom.xml",
        "<?xml version=\"1.0\"?>\n\
        <project xmlns=\"http://maven.apache.org/POM/4.0.0\">\n  \
          <modelVersion>4.0.0</modelVersion>\n  \
          <groupId>com.org</groupId>\n  \
          <artifactId>schema</artifactId>\n  \
          <version>0.4.0</version>\n\
        </project>\n",
    );

    Command::new("git")
        .args(["add", "-A"])
        .current_dir(repo.root())
        .output()
        .expect("git add");
    repo.commit("seed polyglot-cross-eco-group fixture");
}

/// L.6 — `kotlin-library-only`. A standalone JVM library nested
/// under `lib/` (the detector requires a non-root parent directory
/// for its `relative_repopath` to be non-empty). Validates the
/// JVM-library detector hits without other heuristics polluting
/// the signal.
pub fn seed_kotlin_library_only<R: Seedable>(repo: &R) {
    repo.write_file(
        "libs/main/build.gradle.kts",
        "plugins {\n  kotlin(\"jvm\") version \"1.9.0\"\n  `maven-publish`\n}\ngroup = \"com.example\"\n",
    );
    repo.write_file("libs/main/gradle.properties", "version=1.4.2\n");
    repo.write_file(
        "settings.gradle.kts",
        "rootProject.name = \"kotlin-lib\"\ninclude(\"libs:main\")\n",
    );
    repo.write_file(
        "libs/main/src/main/kotlin/Lib.kt",
        "package com.example\nclass Lib\n",
    );

    Command::new("git")
        .args(["add", "-A"])
        .current_dir(repo.root())
        .output()
        .expect("git add");
    repo.commit("seed kotlin-library-only fixture");
}

/// L.7 — `ios-only`. A repository containing nothing but an iOS app.
/// Drives the Phase I.4 single-mobile-repo exit path: the wizard
/// should suggest Bitrise/fastlane/Codemagic and exit without
/// bootstrapping. The xcodeproj sits under `app/` because the
/// detector skips matches at the repo root (relative path empty).
pub fn seed_ios_only<R: Seedable>(repo: &R) {
    repo.write_file(
        "app/MyApp.xcodeproj/project.pbxproj",
        "// dummy pbxproj for fixture\n",
    );
    repo.write_file(
        "app/MyApp/Info.plist",
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict/></plist>\n",
    );
    repo.write_file(
        "app/MyApp/AppDelegate.swift",
        "import UIKit\n@main\nclass AppDelegate: UIResponder, UIApplicationDelegate {}\n",
    );

    Command::new("git")
        .args(["add", "-A"])
        .current_dir(repo.root())
        .output()
        .expect("git add");
    repo.commit("seed ios-only fixture");
}
