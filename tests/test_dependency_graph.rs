mod common;

use common::TestRepo;

#[test]
fn test_dependency_chain_a_depends_on_b() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[workspace]
members = ["packages/*"]
resolver = "2"
"#,
    );

    repo.write_file(
        "packages/core/Cargo.toml",
        r#"[package]
name = "dep-core"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("packages/core/src/lib.rs", "pub fn core_fn() {}\n");

    repo.write_file(
        "packages/app/Cargo.toml",
        r#"[package]
name = "dep-app"
version = "1.0.0"
edition = "2021"

[dependencies]
dep-core = { path = "../core" }
"#,
    );
    repo.write_file(
        "packages/app/src/lib.rs",
        "use dep_core::core_fn; pub fn app_fn() { core_fn(); }\n",
    );

    repo.commit("Initial commit with dependency");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(
        output.status.success(),
        "Init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    repo.write_file(
        "packages/core/src/feature.rs",
        "pub fn new_core_feature() {}\n",
    );
    repo.commit("feat(core): add new core feature");

    let output = repo.run_belaf_command(&["prepare", "--no-tui", "auto"]);
    assert!(
        output.status.success(),
        "Prepare failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let core_toml = repo.read_file("packages/core/Cargo.toml");
    assert!(
        core_toml.contains("version = \"1.1.0\""),
        "Core should be bumped to 1.1.0. Got: {core_toml}"
    );
}

#[test]
fn test_dependency_chain_three_levels() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[workspace]
members = ["packages/*"]
resolver = "2"
"#,
    );

    repo.write_file(
        "packages/base/Cargo.toml",
        r#"[package]
name = "chain-base"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("packages/base/src/lib.rs", "pub fn base() {}\n");

    repo.write_file(
        "packages/middle/Cargo.toml",
        r#"[package]
name = "chain-middle"
version = "1.0.0"
edition = "2021"

[dependencies]
chain-base = { path = "../base" }
"#,
    );
    repo.write_file("packages/middle/src/lib.rs", "pub fn middle() {}\n");

    repo.write_file(
        "packages/top/Cargo.toml",
        r#"[package]
name = "chain-top"
version = "1.0.0"
edition = "2021"

[dependencies]
chain-middle = { path = "../middle" }
"#,
    );
    repo.write_file("packages/top/src/lib.rs", "pub fn top() {}\n");

    repo.commit("Initial commit with A->B->C chain");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(
        output.status.success(),
        "Init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    repo.write_file("packages/base/src/feature.rs", "pub fn base_feature() {}\n");
    repo.commit("feat(base): add base feature");

    let output = repo.run_belaf_command(&["prepare", "--no-tui", "auto"]);
    assert!(
        output.status.success(),
        "Prepare failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let base_toml = repo.read_file("packages/base/Cargo.toml");
    assert!(
        base_toml.contains("version = \"1.1.0\""),
        "Base should be bumped. Got: {base_toml}"
    );
}

#[test]
fn test_diamond_dependency() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[workspace]
members = ["packages/*"]
resolver = "2"
"#,
    );

    repo.write_file(
        "packages/shared/Cargo.toml",
        r#"[package]
name = "diamond-shared"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("packages/shared/src/lib.rs", "pub fn shared() {}\n");

    repo.write_file(
        "packages/left/Cargo.toml",
        r#"[package]
name = "diamond-left"
version = "1.0.0"
edition = "2021"

[dependencies]
diamond-shared = { path = "../shared" }
"#,
    );
    repo.write_file("packages/left/src/lib.rs", "pub fn left() {}\n");

    repo.write_file(
        "packages/right/Cargo.toml",
        r#"[package]
name = "diamond-right"
version = "1.0.0"
edition = "2021"

[dependencies]
diamond-shared = { path = "../shared" }
"#,
    );
    repo.write_file("packages/right/src/lib.rs", "pub fn right() {}\n");

    repo.write_file(
        "packages/top/Cargo.toml",
        r#"[package]
name = "diamond-top"
version = "1.0.0"
edition = "2021"

[dependencies]
diamond-left = { path = "../left" }
diamond-right = { path = "../right" }
"#,
    );
    repo.write_file("packages/top/src/lib.rs", "pub fn top() {}\n");

    repo.commit("Initial commit with diamond dependency");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(
        output.status.success(),
        "Init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    repo.write_file(
        "packages/shared/src/feature.rs",
        "pub fn shared_feature() {}\n",
    );
    repo.commit("feat(shared): add shared feature");

    let output = repo.run_belaf_command(&["prepare", "--no-tui", "auto"]);
    assert!(
        output.status.success(),
        "Prepare failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let shared_toml = repo.read_file("packages/shared/Cargo.toml");
    assert!(
        shared_toml.contains("version = \"1.1.0\""),
        "Shared should be bumped. Got: {shared_toml}"
    );
}

#[test]
fn test_independent_packages_no_cascade() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[workspace]
members = ["packages/*"]
resolver = "2"
"#,
    );

    repo.write_file(
        "packages/alpha/Cargo.toml",
        r#"[package]
name = "indep-alpha"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("packages/alpha/src/lib.rs", "pub fn alpha() {}\n");

    repo.write_file(
        "packages/beta/Cargo.toml",
        r#"[package]
name = "indep-beta"
version = "2.0.0"
edition = "2021"
"#,
    );
    repo.write_file("packages/beta/src/lib.rs", "pub fn beta() {}\n");

    repo.commit("Initial commit with independent packages");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(output.status.success());

    repo.write_file(
        "packages/alpha/src/feature.rs",
        "pub fn alpha_feature() {}\n",
    );
    repo.commit("feat(alpha): add alpha feature");

    let output = repo.run_belaf_command(&["prepare", "--no-tui", "auto"]);
    assert!(output.status.success());

    let alpha_toml = repo.read_file("packages/alpha/Cargo.toml");
    let beta_toml = repo.read_file("packages/beta/Cargo.toml");

    assert!(
        alpha_toml.contains("version = \"1.1.0\""),
        "Alpha should be bumped. Got: {alpha_toml}"
    );
    assert!(
        beta_toml.contains("version = \"2.0.0\""),
        "Beta should NOT be bumped (independent). Got: {beta_toml}"
    );
}

#[test]
fn test_workspace_cargo_detection() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[workspace]
members = ["crates/*"]
resolver = "2"
"#,
    );

    repo.write_file(
        "crates/lib-a/Cargo.toml",
        r#"[package]
name = "workspace-lib-a"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write_file("crates/lib-a/src/lib.rs", "pub fn lib_a() {}\n");

    repo.write_file(
        "crates/lib-b/Cargo.toml",
        r#"[package]
name = "workspace-lib-b"
version = "0.2.0"
edition = "2021"

[dependencies]
workspace-lib-a = { path = "../lib-a" }
"#,
    );
    repo.write_file("crates/lib-b/src/lib.rs", "pub fn lib_b() {}\n");

    repo.commit("Initial workspace commit");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(
        output.status.success(),
        "Init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let bootstrap = repo.read_file("belaf/bootstrap.toml");
    assert!(
        bootstrap.contains("workspace-lib-a") || bootstrap.contains("lib-a"),
        "Should detect workspace-lib-a. Bootstrap: {bootstrap}"
    );
    assert!(
        bootstrap.contains("workspace-lib-b") || bootstrap.contains("lib-b"),
        "Should detect workspace-lib-b. Bootstrap: {bootstrap}"
    );
}

#[test]
fn test_npm_workspace_dependencies() {
    let repo = TestRepo::new();

    repo.write_file(
        "package.json",
        r#"{
  "name": "npm-workspace-root",
  "private": true,
  "workspaces": ["packages/*"]
}
"#,
    );

    repo.write_file(
        "packages/core/package.json",
        r#"{
  "name": "@myorg/core",
  "version": "1.0.0"
}
"#,
    );
    repo.write_file("packages/core/index.js", "module.exports = {};\n");

    repo.write_file(
        "packages/app/package.json",
        r#"{
  "name": "@myorg/app",
  "version": "1.0.0",
  "dependencies": {
    "@myorg/core": "workspace:*"
  }
}
"#,
    );
    repo.write_file(
        "packages/app/index.js",
        "const core = require('@myorg/core');\n",
    );

    repo.commit("Initial npm workspace");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(
        output.status.success(),
        "Init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let bootstrap = repo.read_file("belaf/bootstrap.toml");
    assert!(
        bootstrap.contains("@myorg/core") || bootstrap.contains("core"),
        "Should detect @myorg/core"
    );
}

#[test]
fn test_mixed_ecosystem_monorepo() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[workspace]
members = ["rust"]
resolver = "2"
"#,
    );

    repo.write_file(
        "rust/Cargo.toml",
        r#"[package]
name = "mixed-rust"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("rust/src/lib.rs", "pub fn rust_fn() {}\n");

    repo.write_file(
        "node/package.json",
        r#"{
  "name": "mixed-node",
  "version": "1.0.0"
}
"#,
    );
    repo.write_file("node/index.js", "module.exports = {};\n");

    repo.write_file(
        "python/setup.cfg",
        r"[metadata]
name = mixed-python
version = 1.0.0
",
    );
    repo.write_file(
        "python/setup.py",
        r"from setuptools import setup
setup()
",
    );
    repo.write_file("python/src/__init__.py", "");

    repo.commit("Initial mixed ecosystem monorepo");

    let output = repo.run_belaf_command(&["init", "--force"]);
    assert!(
        output.status.success(),
        "Init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let bootstrap = repo.read_file("belaf/bootstrap.toml");
    assert!(
        bootstrap.contains("mixed-rust"),
        "Should detect Rust package"
    );
    assert!(
        bootstrap.contains("mixed-node"),
        "Should detect Node package"
    );
    assert!(
        bootstrap.contains("mixed-python"),
        "Should detect Python package"
    );
}
