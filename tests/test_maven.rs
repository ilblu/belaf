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
fn resolves_sha1_property() {
    let repo = TestRepo::new();
    repo.write_file(
        "pom.xml",
        r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>lib</artifactId>
  <version>${sha1}</version>
  <properties>
    <sha1>3.1.0</sha1>
  </properties>
</project>
"#,
    );
    repo.commit("init");

    let init = repo.run_belaf_command(&["init", "--force"]);
    assert!(init.status.success());

    repo.write_file("src/main/java/S.java", "class S {}\n");
    repo.commit("feat: add S");

    let _ = repo.run_belaf_command(&["prepare", "--ci"]);

    let manifest = read_manifest_json(&repo);
    let releases = manifest["releases"].as_array().unwrap();
    assert_eq!(releases.len(), 1);
    assert_eq!(
        releases[0]["previous_version"], "3.1.0",
        "previous_version must reflect the resolved ${{sha1}} value"
    );
    assert_eq!(releases[0]["new_version"], "3.2.0");
}

#[test]
fn resolves_changelist_property() {
    let repo = TestRepo::new();
    repo.write_file(
        "pom.xml",
        r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>lib</artifactId>
  <version>${changelist}</version>
  <properties>
    <changelist>4.5.6</changelist>
  </properties>
</project>
"#,
    );
    repo.commit("init");

    let init = repo.run_belaf_command(&["init", "--force"]);
    assert!(init.status.success());

    repo.write_file("src/main/java/C.java", "class C {}\n");
    repo.commit("fix: typo");

    let _ = repo.run_belaf_command(&["prepare", "--ci"]);

    let manifest = read_manifest_json(&repo);
    let releases = manifest["releases"].as_array().unwrap();
    assert_eq!(releases.len(), 1);
    assert_eq!(
        releases[0]["previous_version"], "4.5.6",
        "previous_version must reflect the resolved ${{changelist}} value"
    );
    assert_eq!(releases[0]["new_version"], "4.5.7");
}

#[test]
fn resolves_revision_concatenated_with_sha1_and_changelist() {
    // Maven CI-friendly versions canonical pattern:
    // `<version>${revision}${sha1}${changelist}</version>`. Exercises all
    // three properties in the same field so a regression in any one of
    // them breaks the test.
    let repo = TestRepo::new();
    repo.write_file(
        "pom.xml",
        r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>lib</artifactId>
  <version>${revision}${sha1}${changelist}</version>
  <properties>
    <revision>1.2.3</revision>
    <sha1></sha1>
    <changelist>-SNAPSHOT</changelist>
  </properties>
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

    repo.write_file("src/main/java/M.java", "class M {}\n");
    repo.commit("feat: add M");

    let _ = repo.run_belaf_command(&["prepare", "--ci"]);

    let manifest = read_manifest_json(&repo);
    let releases = manifest["releases"].as_array().unwrap();
    assert_eq!(releases.len(), 1);
    assert_eq!(
        releases[0]["previous_version"], "1.2.3-SNAPSHOT",
        "all three properties must concatenate in resolution order"
    );
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
    let manifest_file = manifest_files
        .iter()
        .find(|f| f.ends_with(".json"))
        .unwrap();
    let content = repo.read_file(&format!("belaf/releases/{manifest_file}"));
    let manifest: serde_json::Value = serde_json::from_str(&content).unwrap();
    let releases = manifest["releases"].as_array().unwrap();
    assert_eq!(
        releases[0]["tag_name"], "com.example/lib@v1.1.0",
        "Maven tag must use `/` instead of `:` so it survives git ref-format"
    );
    assert!(
        !releases[0]["tag_name"].as_str().unwrap().contains(':'),
        "Maven tag must not contain `:` (git ref-format rejects it)"
    );
}

/// `[release_unit.<name>].tag_format` flows from the resolver into
/// `ResolvedReleaseUnit` so the workflow's tag builder can look it up
/// by user-facing name.
#[test]
fn release_unit_tag_format_lookup_by_name() {
    use belaf::core::config::NamedReleaseUnitConfig;
    use belaf::core::release_unit::resolver::resolve;
    use belaf::core::release_unit::syntax::{ManifestFileConfig, ManifestList, ReleaseUnitConfig};

    let repo_path =
        std::env::temp_dir().join(format!("belaf-tag-format-lookup-{}", std::process::id()));
    std::fs::create_dir_all(&repo_path).unwrap();
    std::process::Command::new("git")
        .arg("init")
        .current_dir(&repo_path)
        .output()
        .unwrap();
    let cargo_path = repo_path.join("crates/my-crate/Cargo.toml");
    std::fs::create_dir_all(cargo_path.parent().unwrap()).unwrap();
    std::fs::write(
        &cargo_path,
        "[package]\nname = \"my-crate\"\nversion = \"1.0.0\"\nedition = \"2021\"\n",
    )
    .unwrap();

    let unit = NamedReleaseUnitConfig {
        name: "my-crate".into(),
        config: ReleaseUnitConfig {
            ecosystem: "cargo".into(),
            name: None,
            glob: None,
            manifests: Some(ManifestList::Explicit(vec![ManifestFileConfig {
                path: "crates/my-crate/Cargo.toml".into(),
                ecosystem: None,
                version_field: "cargo_toml".into(),
                regex_pattern: None,
                regex_replace: None,
            }])),
            external: None,
            fallback_manifests: vec![],
            version_field: None,
            satellites: Vec::new(),
            tag_format: Some("custom-{name}-v{version}".into()),
            visibility: None,
            cascade_from: None,
        },
    };

    let repo = belaf::core::git::repository::Repository::open_with(
        &repo_path,
        "origin",
        belaf::core::config::syntax::AnalysisConfig {
            commit_cache_size: 1024,
            tree_cache_size: 1024,
        },
    )
    .unwrap();
    let resolved = resolve(&repo, &[unit]).unwrap();
    assert_eq!(resolved.len(), 1);
    assert_eq!(
        resolved[0].unit.tag_format.as_deref(),
        Some("custom-{name}-v{version}"),
        "ResolvedReleaseUnit must carry the tag_format from [release_unit.<name>]"
    );
    let _ = std::fs::remove_dir_all(&repo_path);
}

/// Plan §12 / Gap #1. Multi-module repo: aggregator (parent) + child
/// where the child has a `<parent>` ref. When the parent gets bumped,
/// the child's `<parent><version>` must follow, otherwise the child's
/// build is broken on next mvn invocation.
#[test]
fn multi_module_parent_version_propagates_to_children() {
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
  <parent>
    <groupId>com.example</groupId>
    <artifactId>parent</artifactId>
    <version>1.0.0</version>
  </parent>
  <artifactId>child1</artifactId>
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

    // Touch the parent so it gets bumped (feature commit on the parent dir).
    repo.write_file("README.md", "# parent\n");
    repo.commit("feat: parent change");

    let _ = repo.run_belaf_command(&["prepare", "--ci"]);

    let parent_after = repo.read_file("pom.xml");
    let child_after = repo.read_file("child1/pom.xml");

    assert!(
        parent_after.contains("<artifactId>parent</artifactId>")
            && parent_after.contains("<version>1.1.0</version>"),
        "parent POM should be at 1.1.0; got:\n{parent_after}"
    );
    assert!(
        child_after.contains("<artifactId>parent</artifactId>")
            && child_after.contains("<version>1.1.0</version>"),
        "child's <parent><version> should follow the parent bump to 1.1.0; got:\n{child_after}"
    );
    assert!(
        !child_after.contains("<version>1.0.0</version>"),
        "stale parent version 1.0.0 must not remain in child; got:\n{child_after}"
    );
}

/// Inter-module dep with explicit version: when the target sibling
/// is bumped, the dep's `<version>` in the depending POM follows.
#[test]
fn inter_module_dependency_version_propagates() {
    let repo = TestRepo::new();
    repo.write_file(
        "modules/lib/pom.xml",
        r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>lib</artifactId>
  <version>1.0.0</version>
</project>
"#,
    );
    repo.write_file(
        "modules/app/pom.xml",
        r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>app</artifactId>
  <version>1.0.0</version>
  <dependencies>
    <dependency>
      <groupId>com.example</groupId>
      <artifactId>lib</artifactId>
      <version>1.0.0</version>
    </dependency>
  </dependencies>
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

    // Touch the lib so it gets a feat bump. App should follow because of the dep.
    repo.write_file("modules/lib/feat.java", "class F {}\n");
    repo.commit("feat: bump lib");

    let _ = repo.run_belaf_command(&["prepare", "--ci"]);

    let app_after = repo.read_file("modules/app/pom.xml");
    // The dep's <version> should now point at 1.1.0.
    let lib_dep_idx = app_after
        .find("<artifactId>lib</artifactId>")
        .expect("app POM should still reference lib");
    let after_lib_dep = &app_after[lib_dep_idx..];
    assert!(
        after_lib_dep.contains("<version>1.1.0</version>"),
        "app's dep on `lib` should follow lib's bump to 1.1.0; got after lib block:\n{after_lib_dep}"
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
