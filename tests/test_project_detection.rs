mod common;
use common::TestRepo;

#[test]
fn test_detect_single_rust_project() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "my-crate"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
    repo.commit("initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);

    assert!(
        output.status.success(),
        "Failed to init: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    let bootstrap = repo.read_file("belaf/bootstrap.toml");
    assert!(bootstrap.contains("my-crate"), "Rust project not detected");
    assert!(bootstrap.contains("0.1.0"), "Version not detected");
}

#[test]
fn test_detect_single_go_project() {
    let repo = TestRepo::new();

    repo.write_file(
        "go.mod",
        r"module myapp

go 1.21
",
    );
    repo.write_file("main.go", "package main\n\nfunc main() {}\n");
    repo.commit("initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);

    assert!(
        output.status.success(),
        "Failed to init: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    let bootstrap = repo.read_file("belaf/bootstrap.toml");
    assert!(bootstrap.contains("myapp"), "Go project not detected");
}

#[test]
fn test_detect_single_elixir_project() {
    let repo = TestRepo::new();

    repo.write_file(
        "mix.exs",
        r#"defmodule MyApp.MixProject do
  use Mix.Project

  def project do
    [
      app: :my_app,
      version: "1.0.0",
      elixir: "~> 1.14"
    ]
  end
end
"#,
    );
    repo.commit("initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);

    assert!(
        output.status.success(),
        "Failed to init: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    let bootstrap = repo.read_file("belaf/bootstrap.toml");
    assert!(bootstrap.contains("my_app"), "Elixir project not detected");
    assert!(bootstrap.contains("1.0.0"), "Version not detected");
}

#[test]
fn test_detect_single_npm_project() {
    let repo = TestRepo::new();

    repo.write_file(
        "package.json",
        r#"{
  "name": "my-package",
  "version": "2.0.0",
  "description": "Test package"
}
"#,
    );
    repo.commit("initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);

    assert!(
        output.status.success(),
        "Failed to init: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    let bootstrap = repo.read_file("belaf/bootstrap.toml");
    assert!(bootstrap.contains("my-package"), "NPM project not detected");
    assert!(bootstrap.contains("2.0.0"), "Version not detected");
}

#[test]
fn test_detect_single_python_project() {
    let repo = TestRepo::new();

    repo.write_file(
        "setup.cfg",
        r"[metadata]
name = my-python-pkg
version = 3.0.0
description = Test Python package
",
    );
    repo.write_file(
        "setup.py",
        r#"from setuptools import setup
version = "3.0.0"  # belaf project-version
setup()
"#,
    );
    repo.commit("initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);

    assert!(
        output.status.success(),
        "Failed to init: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    let bootstrap = repo.read_file("belaf/bootstrap.toml");
    assert!(
        bootstrap.contains("my-python-pkg"),
        "Python project not detected"
    );
    assert!(bootstrap.contains("3.0.0"), "Version not detected");
}

#[test]
fn test_detect_monorepo_multiple_rust_crates() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[workspace]
members = ["crates/*"]
"#,
    );

    repo.write_file(
        "crates/web/Cargo.toml",
        r#"[package]
name = "web"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write_file("crates/web/src/lib.rs", "pub fn web() {}\n");

    repo.write_file(
        "crates/api/Cargo.toml",
        r#"[package]
name = "api"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write_file("crates/api/src/lib.rs", "pub fn api() {}\n");

    repo.write_file(
        "crates/core/Cargo.toml",
        r#"[package]
name = "core"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write_file("crates/core/src/lib.rs", "pub fn core_fn() {}\n");

    repo.commit("initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);

    assert!(
        output.status.success(),
        "Failed to init: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    let bootstrap = repo.read_file("belaf/bootstrap.toml");
    assert!(bootstrap.contains("web"), "web crate not detected");
    assert!(bootstrap.contains("api"), "api crate not detected");
    assert!(bootstrap.contains("core"), "core crate not detected");
}

#[test]
fn test_detect_monorepo_mixed_languages() {
    let repo = TestRepo::new();

    repo.write_file(
        "backend/Cargo.toml",
        r#"[package]
name = "backend"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write_file("backend/src/lib.rs", "pub fn backend() {}\n");

    repo.write_file(
        "frontend/package.json",
        r#"{
  "name": "frontend",
  "version": "1.0.0"
}
"#,
    );

    repo.write_file(
        "scripts/setup.cfg",
        r"[metadata]
name = deployment-tools
version = 0.1.0
",
    );
    repo.write_file(
        "scripts/setup.py",
        r#"from setuptools import setup
version = "0.1.0"  # belaf project-version
setup()
"#,
    );

    repo.commit("initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);

    assert!(
        output.status.success(),
        "Failed to init: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    let bootstrap = repo.read_file("belaf/bootstrap.toml");
    assert!(bootstrap.contains("backend"), "Rust backend not detected");
    assert!(bootstrap.contains("frontend"), "NPM frontend not detected");
    assert!(
        bootstrap.contains("deployment-tools"),
        "Python scripts not detected"
    );
}

#[test]
fn test_detect_workspace_with_dependencies() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[workspace]
members = ["crates/*"]
"#,
    );

    repo.write_file(
        "crates/common/Cargo.toml",
        r#"[package]
name = "common"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write_file("crates/common/src/lib.rs", "pub fn common() {}\n");

    repo.write_file(
        "crates/app/Cargo.toml",
        r#"[package]
name = "app"
version = "0.1.0"
edition = "2021"

[dependencies]
common = { path = "../common" }
"#,
    );
    repo.write_file("crates/app/src/lib.rs", "pub fn app() {}\n");

    repo.commit("initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);

    assert!(
        output.status.success(),
        "Failed to init: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    let bootstrap = repo.read_file("belaf/bootstrap.toml");
    assert!(bootstrap.contains("common"), "common crate not detected");
    assert!(bootstrap.contains("app"), "app crate not detected");
}

#[test]
fn test_detect_single_swift_package() {
    let repo = TestRepo::new();

    repo.write_file(
        "Package.swift",
        r#"// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "MySwiftLibrary",
    products: [
        .library(name: "MySwiftLibrary", targets: ["MySwiftLibrary"]),
    ],
    targets: [
        .target(name: "MySwiftLibrary"),
    ]
)
"#,
    );
    repo.write_file(
        "Sources/MySwiftLibrary/MySwiftLibrary.swift",
        "public struct MySwiftLibrary {}\n",
    );
    repo.commit("initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);

    assert!(
        output.status.success(),
        "Failed to init: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    let bootstrap = repo.read_file("belaf/bootstrap.toml");
    assert!(
        bootstrap.contains("MySwiftLibrary"),
        "Swift package not detected"
    );
}
