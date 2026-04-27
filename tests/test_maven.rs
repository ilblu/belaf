//! Maven integration tests — wires `pom.xml` discovery + parsing + rewrite
//! together end-to-end. The unit tests in `core::ecosystem::maven::tests`
//! cover the pure logic (parser, resolver, rewriter); these exercise the
//! loader through `belaf init` + `belaf prepare --ci` against real temp
//! repos.

mod common;

use common::TestRepo;

fn read_manifest_json(repo: &TestRepo) -> serde_json::Value {
    let files = repo.list_files_in_dir("belaf/releases");
    let manifest_file = files
        .iter()
        .find(|f| f.ends_with(".json"))
        .expect("a manifest .json should have been written");
    let content = repo.read_file(&format!("belaf/releases/{manifest_file}"));
    serde_json::from_str(&content).expect("manifest must be valid JSON")
}

#[test]
fn detects_single_module_pom() {
    let repo = TestRepo::new();
    repo.write_file(
        "pom.xml",
        r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>lib</artifactId>
  <version>1.0.0</version>
</project>
"#,
    );
    repo.commit("init");

    let init = repo.run_belaf_command(&["init", "--force"]);
    assert!(
        init.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    repo.write_file("src/main/java/Foo.java", "class Foo {}\n");
    repo.commit("feat: add Foo");

    let _ = repo.run_belaf_command(&["prepare", "--ci"]);

    let manifest = read_manifest_json(&repo);
    let releases = manifest["releases"].as_array().unwrap();
    assert_eq!(releases.len(), 1, "expected one Maven release");
    assert_eq!(releases[0]["name"], "com.example:lib");
    assert_eq!(releases[0]["ecosystem"], "maven");
    assert_eq!(releases[0]["new_version"], "1.1.0");
}

#[test]
fn resolves_revision_property() {
    let repo = TestRepo::new();
    repo.write_file(
        "pom.xml",
        r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>lib</artifactId>
  <version>${revision}</version>
  <properties>
    <revision>2.4.1</revision>
  </properties>
</project>
"#,
    );
    repo.commit("init");

    let init = repo.run_belaf_command(&["init", "--force"]);
    assert!(init.status.success());

    repo.write_file("src/main/java/Bar.java", "class Bar {}\n");
    repo.commit("feat: add Bar");

    let _ = repo.run_belaf_command(&["prepare", "--ci"]);

    let manifest = read_manifest_json(&repo);
    let releases = manifest["releases"].as_array().unwrap();
    assert_eq!(releases.len(), 1);
    assert_eq!(
        releases[0]["previous_version"], "2.4.1",
        "previous_version must reflect the resolved ${{revision}} value"
    );
    assert_eq!(releases[0]["new_version"], "2.5.0");
}

#[test]
fn unsupported_property_in_version_is_hard_error() {
    let repo = TestRepo::new();
    repo.write_file(
        "pom.xml",
        r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>lib</artifactId>
  <version>${customVer}</version>
  <properties>
    <customVer>1.0.0</customVer>
  </properties>
</project>
"#,
    );
    repo.commit("init");

    let init = repo.run_belaf_command(&["init", "--force"]);
    assert!(
        !init.status.success(),
        "init should fail when a POM uses an unsupported property in <version>"
    );
    let stderr = String::from_utf8_lossy(&init.stderr);
    assert!(
        stderr.contains("unsupported property"),
        "error must mention `unsupported property`; got:\n{stderr}"
    );
    assert!(
        stderr.contains("revision"),
        "error must list supported properties incl. `revision`; got:\n{stderr}"
    );
}

#[test]
fn detects_multi_module_aggregator() {
    let repo = TestRepo::new();
    repo.write_file(
        "pom.xml",
        r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>parent</artifactId>
  <version>1.0.0</version>
  <packaging>pom</packaging>
  <modules>
    <module>child1</module>
  </modules>
</project>
"#,
    );
    repo.write_file(
        "child1/pom.xml",
        r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>child1</artifactId>
  <version>1.0.0</version>
</project>
"#,
    );
    repo.commit("init");

    let init = repo.run_belaf_command(&["init", "--force"]);
    assert!(
        init.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    repo.write_file("child1/src/main/java/X.java", "class X {}\n");
    repo.commit("feat: child1 update");

    let _ = repo.run_belaf_command(&["prepare", "--ci"]);

    let manifest = read_manifest_json(&repo);
    let releases = manifest["releases"].as_array().unwrap();
    let names: Vec<&str> = releases
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"com.example:child1"),
        "child1 must be a separate release entry; got {names:?}"
    );
}

#[test]
fn parent_cycle_is_hard_error() {
    let repo = TestRepo::new();
    repo.write_file(
        "a/pom.xml",
        r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>org</groupId>
  <artifactId>a</artifactId>
  <version>1.0.0</version>
  <parent>
    <groupId>org</groupId>
    <artifactId>b</artifactId>
    <version>1.0.0</version>
  </parent>
</project>
"#,
    );
    repo.write_file(
        "b/pom.xml",
        r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>org</groupId>
  <artifactId>b</artifactId>
  <version>1.0.0</version>
  <parent>
    <groupId>org</groupId>
    <artifactId>a</artifactId>
    <version>1.0.0</version>
  </parent>
</project>
"#,
    );
    repo.commit("init");

    let out = repo.run_belaf_command(&["init", "--force"]);
    assert!(
        !out.status.success(),
        "init must fail on Maven <parent> cycle"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("cycle"),
        "error must mention `cycle`; got:\n{stderr}"
    );
    assert!(
        stderr.contains("org:a") && stderr.contains("org:b"),
        "error must name both cycle members; got:\n{stderr}"
    );
}

#[test]
fn maven_tag_format_uses_slash_not_colon() {
    // Regression for the v1 default (`name@v<version>` with `name` =
    // "groupId:artifactId") which produced un-pushable git tags. The v2
    // default for Maven is `{groupId}/{artifactId}@v{version}`.
    let repo = TestRepo::new();
    repo.write_file(
        "pom.xml",
        r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>lib</artifactId>
  <version>1.0.0</version>
</project>
"#,
    );
    repo.commit("init");
    let init = repo.run_belaf_command(&["init", "--force"]);
    assert!(init.status.success());
    repo.write_file("src/main/java/F.java", "class F {}\n");
    repo.commit("feat: F");

    let _ = repo.run_belaf_command(&["prepare", "--ci"]);

    let manifest_files = repo.list_files_in_dir("belaf/releases");
    let manifest_file = manifest_files.iter().find(|f| f.ends_with(".json")).unwrap();
    let content = repo.read_file(&format!("belaf/releases/{manifest_file}"));
    let manifest: serde_json::Value = serde_json::from_str(&content).unwrap();
    let releases = manifest["releases"].as_array().unwrap();
    assert_eq!(
        releases[0]["tag_name"], "com.example/lib@v1.1.0",
        "Maven tag must use `/` instead of `:` so it survives git ref-format"
    );
    assert!(
        !releases[0]["tag_name"]
            .as_str()
            .unwrap()
            .contains(':'),
        "Maven tag must not contain `:` (git ref-format rejects it)"
    );
}

#[test]
fn project_tag_format_override_with_invalid_var_is_hard_error() {
    let repo = TestRepo::new();
    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "my-crate"
version = "1.0.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn x() {}\n");
    repo.commit("init");
    let init = repo.run_belaf_command(&["init", "--force"]);
    assert!(init.status.success());

    // npm has no {groupId} variable. Using it on a Cargo project must
    // hard-error with the offending var named.
    let cfg = repo.read_file("belaf/config.toml");
    let bad = format!(
        "{cfg}\n[projects.\"my-crate\"]\ntag_format = \"{{groupId}}-{{name}}-{{version}}\"\n"
    );
    repo.write_file("belaf/config.toml", &bad);
    repo.write_file("src/feat.rs", "pub fn feat() {}\n");
    repo.commit("feat: drift");

    let out = repo.run_belaf_command(&["prepare", "--ci"]);
    assert!(
        !out.status.success(),
        "prepare must reject a tag_format using a variable not allowed for the ecosystem"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("groupId") && stderr.contains("cargo"),
        "stderr must name the offending variable + ecosystem; got:\n{stderr}"
    );
}

#[test]
fn rewriter_preserves_comments() {
    let repo = TestRepo::new();
    let pom = r#"<?xml version="1.0"?>
<!-- top comment -->
<project>
  <modelVersion>4.0.0</modelVersion>
  <!-- coords -->
  <groupId>com.example</groupId>
  <artifactId>lib</artifactId>
  <version>1.0.0</version>
</project>
"#;
    repo.write_file("pom.xml", pom);
    repo.commit("init");

    let init = repo.run_belaf_command(&["init", "--force"]);
    assert!(init.status.success());

    repo.write_file("src/main/java/F.java", "class F {}\n");
    repo.commit("fix: F");

    let _ = repo.run_belaf_command(&["prepare", "--ci"]);

    let after = repo.read_file("pom.xml");
    assert!(after.contains("<!-- top comment -->"));
    assert!(after.contains("<!-- coords -->"));
    assert!(
        after.contains("<version>1.0.1</version>"),
        "expected top-level version bumped to 1.0.1; got:\n{after}"
    );
}
