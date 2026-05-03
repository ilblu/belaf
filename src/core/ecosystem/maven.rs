//! Maven (Java/Kotlin/Scala) projects.
//!
//! Loads `pom.xml` files into the project graph, then rewrites them on
//! release. Supports the subset of Maven that real CI-friendly projects
//! actually need:
//!
//! - Single-module POMs.
//! - `<parent>` references — rewritten in the child when the parent's
//!   version bumps. Cycles between POMs via `<parent>` are detected via
//!   Tarjan-SCC at finalize time and reported with all members.
//! - `<dependencyManagement>` — version bumps in inter-project deps
//!   propagate.
//! - Multi-module aggregators (`<modules>`): each child is loaded as a
//!   separate project, the aggregator itself is a project too.
//! - CI-friendly property resolution: `${revision}`, `${sha1}`,
//!   `${changelist}`, `${project.version}`. Walks `<parent>`-inherited
//!   `<properties>`. **No** `-D` system properties, **no** environment
//!   variables, **no** profiles or `settings.xml` (out of scope).
//!
//! An unsupported property name in a `<version>` field is a hard error
//! that names the supported set so the user can pick one of those
//! instead.
//!
//! Read **and** write go through `quick_xml::Reader`/`Writer` so we keep
//! whitespace and comments byte-stable across rewrite — line-based was
//! considered (csproj uses it) but POM `<version>` elements can have
//! internal whitespace and comments between tags, so the streaming-XML
//! path is safer.
//!
//! Submodule layout:
//! - [`pom_parser`] — streaming `pom.xml` parser + parent-cycle
//!   detection.
//! - [`property_resolver`] — `<parent>` chain inheritance + Maven
//!   CI-friendly property substitution.
//! - [`pom_rewriter`] — [`MavenRewriter`] + the `quick_xml` rewriter
//!   that preserves comments and whitespace.

use std::collections::HashMap;

use anyhow::anyhow;
use tracing::info;

use crate::{
    atry,
    core::{
        ecosystem::format_handler::{
            scan_index_for_filename, DiscoveredUnit, FormatHandler, RawInternalDep,
        },
        errors::Result,
        git::repository::{RepoPathBuf, Repository},
        release_unit::VersionFieldSpec,
        resolved_release_unit::{DepRequirement, ReleaseUnitId},
        rewriters::Rewriter,
        version::Version,
    },
};

mod pom_parser;
mod pom_rewriter;
mod property_resolver;

pub use pom_rewriter::MavenRewriter;

use pom_parser::{detect_parent_cycles, ParsedPom};
use property_resolver::resolve_pom;

#[derive(Debug, Default)]
pub struct MavenLoader;

impl FormatHandler for MavenLoader {
    fn name(&self) -> &'static str {
        "maven"
    }

    fn display_name(&self) -> &'static str {
        "Maven"
    }

    fn manifest_filename(&self) -> &'static str {
        "pom.xml"
    }

    fn default_version_field(&self) -> VersionFieldSpec {
        // Maven `<version>...</version>` between the project root tags.
        // The MavenRewriter handles full XML-aware rewriting; this
        // regex is a fallback the version_field dispatcher uses.
        VersionFieldSpec::GenericRegex {
            pattern: r"<version>([^<]+)</version>".to_string(),
            replace: "<version>{version}</version>".to_string(),
        }
    }

    fn tag_format_default(&self) -> &'static str {
        "{groupId}/{artifactId}@v{version}"
    }

    fn tag_template_vars(&self) -> &'static [&'static str] {
        &["name", "version", "ecosystem", "groupId", "artifactId"]
    }

    fn make_rewriter(
        &self,
        unit_id: ReleaseUnitId,
        manifest_path: RepoPathBuf,
    ) -> Box<dyn Rewriter> {
        Box::new(MavenRewriter::new(unit_id, manifest_path))
    }

    /// Walk every pom.xml, resolve parent-chains + property substitution
    /// (Tarjan-SCC cycle detection on the parent graph), and emit one
    /// DiscoveredUnit per. Internal-deps come from `<dependencies>` +
    /// `<parent>`; targets are matched by `groupId:artifactId`.
    fn discover_units(
        &self,
        repo: &Repository,
        configured_skip_paths: &[RepoPathBuf],
    ) -> Result<Vec<DiscoveredUnit>> {
        let pom_paths = scan_index_for_filename(repo, "pom.xml", configured_skip_paths)?;
        if pom_paths.is_empty() {
            return Ok(Vec::new());
        }

        info!("loading {} pom.xml file(s)", pom_paths.len());

        let mut parsed: Vec<ParsedPom> = Vec::with_capacity(pom_paths.len());
        for repo_path in &pom_paths {
            let fs_path = repo.resolve_workdir(repo_path);
            let pom = atry!(
                ParsedPom::from_file(repo_path, &fs_path);
                ["failed to parse Maven POM `{}`", repo_path.escaped()]
            );
            parsed.push(pom);
        }

        let mut coord_to_idx: HashMap<(String, String), usize> = HashMap::new();
        for (idx, pom) in parsed.iter().enumerate() {
            if let Some(gid) = &pom.group_id {
                coord_to_idx.insert((gid.clone(), pom.artifact_id.clone()), idx);
            }
        }

        detect_parent_cycles(&parsed, &coord_to_idx)?;

        let mut resolved = Vec::with_capacity(parsed.len());
        for idx in 0..parsed.len() {
            let r = atry!(
                resolve_pom(idx, &parsed, &coord_to_idx);
                ["failed to resolve Maven coordinates for `{}`", parsed[idx].repo_path.escaped()]
            );
            resolved.push(r);
        }

        let mut resolved_coord_to_name: HashMap<(String, String), String> = HashMap::new();
        for r in &resolved {
            resolved_coord_to_name.insert(
                (r.group_id.clone(), r.artifact_id.clone()),
                format!("{}:{}", r.group_id, r.artifact_id),
            );
        }

        let mut units: Vec<DiscoveredUnit> = Vec::with_capacity(resolved.len());
        for (idx, r) in resolved.iter().enumerate() {
            let user_name = format!("{}:{}", r.group_id, r.artifact_id);
            let version = atry!(
                semver::Version::parse(&r.version)
                    .map_err(|e| anyhow!("not semver: {e}"));
                ["Maven version `{}` for `{}` is not parseable as semver",
                 r.version, user_name]
                (note "belaf supports semver-shaped Maven versions only (e.g. 1.2.3, 1.0.0-SNAPSHOT). \
                 Pure-numeric chains like `1.0` need a third component (`1.0.0`).")
            );
            let (prefix, _) = parsed[idx].repo_path.split_basename();
            let manifest = parsed[idx].repo_path.clone();

            let mut internal_deps = Vec::new();
            let pom = &parsed[idx];
            if let Some(p) = &pom.parent {
                if let Some(target_name) =
                    resolved_coord_to_name.get(&(p.group_id.clone(), p.artifact_id.clone()))
                {
                    if target_name != &user_name {
                        internal_deps.push(RawInternalDep {
                            target_package_name: target_name.clone(),
                            literal: p.version.clone(),
                            requirement: DepRequirement::Manual(p.version.clone()),
                        });
                    }
                }
            }
            for dep in &pom.dependencies {
                let Some(dep_version) = &dep.version else {
                    continue;
                };
                let Some(target_name) =
                    resolved_coord_to_name.get(&(dep.group_id.clone(), dep.artifact_id.clone()))
                else {
                    continue;
                };
                if target_name == &user_name {
                    continue;
                }
                internal_deps.push(RawInternalDep {
                    target_package_name: target_name.clone(),
                    literal: dep_version.clone(),
                    requirement: DepRequirement::Manual(dep_version.clone()),
                });
            }

            units.push(DiscoveredUnit {
                qnames: vec![user_name, "maven".to_owned()],
                version: Version::Semver(version),
                prefix: prefix.to_owned(),
                anchor_manifest: manifest.clone(),
                rewriter_factories: vec![Box::new(move |id| {
                    Box::new(MavenRewriter::new(id, manifest))
                })],
                internal_deps,
            });
        }

        Ok(units)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use property_resolver::ResolvedPom;

    fn parse(content: &str) -> ParsedPom {
        let rp = RepoPathBuf::new(b"pom.xml");
        ParsedPom::from_str(rp.as_ref(), Path::new("pom.xml"), content).expect("parse")
    }

    #[test]
    fn parses_single_module_pom() {
        let p = parse(
            r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>lib</artifactId>
  <version>1.0.0</version>
</project>"#,
        );
        assert_eq!(p.group_id.as_deref(), Some("com.example"));
        assert_eq!(p.artifact_id, "lib");
        assert_eq!(p.version.as_deref(), Some("1.0.0"));
        assert!(p.parent.is_none());
        assert!(p.modules.is_empty());
    }

    #[test]
    fn parses_parent_block() {
        let p = parse(
            r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <parent>
    <groupId>com.example</groupId>
    <artifactId>parent</artifactId>
    <version>2.0.0</version>
    <relativePath>../pom.xml</relativePath>
  </parent>
  <artifactId>child</artifactId>
</project>"#,
        );
        let parent = p.parent.expect("parent ref");
        assert_eq!(parent.group_id, "com.example");
        assert_eq!(parent.artifact_id, "parent");
        assert_eq!(parent.version, "2.0.0");
    }

    #[test]
    fn parses_modules_aggregator() {
        let p = parse(
            r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>aggregator</artifactId>
  <version>1.0.0</version>
  <packaging>pom</packaging>
  <modules>
    <module>core</module>
    <module>util</module>
  </modules>
</project>"#,
        );
        assert!(p.is_pom_packaging);
        assert_eq!(p.modules, vec!["core", "util"]);
    }

    #[test]
    fn parses_properties() {
        let p = parse(
            r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>lib</artifactId>
  <version>${revision}</version>
  <properties>
    <revision>2.4.1</revision>
    <changelist>-SNAPSHOT</changelist>
  </properties>
</project>"#,
        );
        assert_eq!(
            p.properties.get("revision").map(String::as_str),
            Some("2.4.1")
        );
        assert_eq!(
            p.properties.get("changelist").map(String::as_str),
            Some("-SNAPSHOT")
        );
    }

    #[test]
    fn parses_dependencies_with_versions() {
        let p = parse(
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
    <dependency>
      <groupId>org.junit.jupiter</groupId>
      <artifactId>junit-jupiter-api</artifactId>
    </dependency>
  </dependencies>
</project>"#,
        );
        assert_eq!(p.dependencies.len(), 2);
        assert_eq!(p.dependencies[0].artifact_id, "lib");
        assert_eq!(p.dependencies[0].version.as_deref(), Some("1.0.0"));
        assert!(p.dependencies[1].version.is_none());
    }

    fn resolve(content: &str) -> ResolvedPom {
        let p = parse(content);
        let mut coords = HashMap::new();
        if let Some(g) = &p.group_id {
            coords.insert((g.clone(), p.artifact_id.clone()), 0);
        }
        let stack = vec![p];
        resolve_pom(0, &stack, &coords).expect("resolve")
    }

    #[test]
    fn resolves_revision_property() {
        let r = resolve(
            r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>lib</artifactId>
  <version>${revision}</version>
  <properties><revision>2.4.1</revision></properties>
</project>"#,
        );
        assert_eq!(r.version, "2.4.1");
    }

    #[test]
    fn resolves_revision_with_changelist_concatenation() {
        let r = resolve(
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
</project>"#,
        );
        assert_eq!(r.version, "1.2.3-SNAPSHOT");
    }

    #[test]
    fn resolves_sha1_property() {
        let r = resolve(
            r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>lib</artifactId>
  <version>${sha1}</version>
  <properties><sha1>3.1.0</sha1></properties>
</project>"#,
        );
        assert_eq!(r.version, "3.1.0");
    }

    #[test]
    fn resolves_changelist_property() {
        let r = resolve(
            r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>lib</artifactId>
  <version>${changelist}</version>
  <properties><changelist>4.5.6</changelist></properties>
</project>"#,
        );
        assert_eq!(r.version, "4.5.6");
    }

    #[test]
    fn project_version_self_ref_in_top_level_version_does_not_converge() {
        let p = parse(
            r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>lib</artifactId>
  <version>${project.version}</version>
</project>"#,
        );
        let mut coords = HashMap::new();
        coords.insert((p.group_id.clone().unwrap(), p.artifact_id.clone()), 0);
        let stack = vec![p];
        let err = resolve_pom(0, &stack, &coords).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("did not converge"),
            "expected non-convergence message; got: {msg}"
        );
    }

    #[test]
    fn rejects_unsupported_property() {
        let p = parse(
            r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>lib</artifactId>
  <version>${customVer}</version>
  <properties><customVer>1.0.0</customVer></properties>
</project>"#,
        );
        let mut coords = HashMap::new();
        coords.insert((p.group_id.clone().unwrap(), p.artifact_id.clone()), 0);
        let stack = vec![p];
        let err = resolve_pom(0, &stack, &coords).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("unsupported property"),
            "expected unsupported-property message, got: {msg}"
        );
        assert!(
            msg.contains("revision"),
            "error must list supported properties incl. `revision`; got: {msg}"
        );
    }

    #[test]
    fn detects_parent_cycle_via_tarjan() {
        let a = parse(
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
</project>"#,
        );
        let b = parse(
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
</project>"#,
        );
        let mut coords = HashMap::new();
        coords.insert(("org".to_string(), "a".to_string()), 0);
        coords.insert(("org".to_string(), "b".to_string()), 1);
        let stack = vec![a, b];
        let err = detect_parent_cycles(&stack, &coords).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("cycle"), "expected cycle error, got: {msg}");
        assert!(msg.contains("org:a"));
        assert!(msg.contains("org:b"));
    }

    #[test]
    fn rewrites_top_level_version_only() {
        let original = r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>lib</artifactId>
  <version>1.0.0</version>
  <dependencies>
    <dependency>
      <groupId>com.other</groupId>
      <artifactId>thing</artifactId>
      <version>9.9.9</version>
    </dependency>
  </dependencies>
</project>"#;
        let no_lookup = |_: &str, _: &str| -> Option<String> { None };
        let rewritten = pom_rewriter::rewrite_pom(original, "2.0.0", &no_lookup).expect("rewrite");
        assert!(
            rewritten.contains("<version>2.0.0</version>"),
            "expected new top-level version 2.0.0; got:\n{rewritten}"
        );
        assert!(
            rewritten.contains("<version>9.9.9</version>"),
            "external dep version must not be rewritten; got:\n{rewritten}"
        );
        assert_eq!(
            rewritten.matches("<version>").count(),
            2,
            "expected exactly two <version> elements (project + dep); got:\n{rewritten}"
        );
    }

    #[test]
    fn rewrite_preserves_comments_and_whitespace() {
        let original = r#"<?xml version="1.0"?>
<!-- top-level comment -->
<project>
  <modelVersion>4.0.0</modelVersion>
  <!-- coordinates -->
  <groupId>com.example</groupId>
  <artifactId>lib</artifactId>
  <version>1.0.0</version>
</project>
"#;
        let no_lookup = |_: &str, _: &str| -> Option<String> { None };
        let rewritten = pom_rewriter::rewrite_pom(original, "1.0.1", &no_lookup).expect("rewrite");
        assert!(rewritten.contains("<!-- top-level comment -->"));
        assert!(rewritten.contains("<!-- coordinates -->"));
        assert!(rewritten.contains("<version>1.0.1</version>"));
    }

    #[test]
    fn rewrites_parent_version_when_lookup_resolves() {
        let child = r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <parent>
    <groupId>com.example</groupId>
    <artifactId>parent</artifactId>
    <version>1.0.0</version>
  </parent>
  <artifactId>child</artifactId>
</project>"#;

        let lookup = |g: &str, a: &str| -> Option<String> {
            if g == "com.example" && a == "parent" {
                Some("1.1.0".to_string())
            } else {
                None
            }
        };
        let rewritten = pom_rewriter::rewrite_pom(child, "1.1.0", &lookup).expect("rewrite");
        assert!(
            rewritten.contains("<version>1.1.0</version>"),
            "<parent> version should now be 1.1.0; got:\n{rewritten}"
        );
        assert!(
            !rewritten.contains("<version>1.0.0</version>"),
            "<parent> version 1.0.0 should be gone; got:\n{rewritten}"
        );
    }

    #[test]
    fn rewrites_dependency_version_when_lookup_resolves() {
        let pom = r#"<?xml version="1.0"?>
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
    <dependency>
      <groupId>org.junit.jupiter</groupId>
      <artifactId>junit-jupiter-api</artifactId>
      <version>5.10.0</version>
    </dependency>
  </dependencies>
</project>"#;

        let lookup = |g: &str, a: &str| -> Option<String> {
            if g == "com.example" && a == "lib" {
                Some("2.0.0".to_string())
            } else {
                None
            }
        };
        let rewritten = pom_rewriter::rewrite_pom(pom, "1.1.0", &lookup).expect("rewrite");

        assert!(
            rewritten.contains("<artifactId>app</artifactId>\n  <version>1.1.0</version>"),
            "top-level project version should be 1.1.0; got:\n{rewritten}"
        );
        let lib_block_idx = rewritten.find("<artifactId>lib</artifactId>").unwrap();
        let after_lib = &rewritten[lib_block_idx..];
        assert!(
            after_lib.contains("<version>2.0.0</version>"),
            "sibling dep `lib` should be 2.0.0; got after lib block:\n{after_lib}"
        );
        assert!(
            rewritten.contains("<version>5.10.0</version>"),
            "external JUnit dep version must be preserved; got:\n{rewritten}"
        );
    }

    #[test]
    fn rewrites_dependency_management_version() {
        let pom = r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>parent</artifactId>
  <version>1.0.0</version>
  <packaging>pom</packaging>
  <dependencyManagement>
    <dependencies>
      <dependency>
        <groupId>com.example</groupId>
        <artifactId>lib</artifactId>
        <version>1.0.0</version>
      </dependency>
    </dependencies>
  </dependencyManagement>
</project>"#;

        let lookup = |g: &str, a: &str| -> Option<String> {
            if g == "com.example" && a == "lib" {
                Some("2.0.0".to_string())
            } else {
                None
            }
        };
        let rewritten = pom_rewriter::rewrite_pom(pom, "1.1.0", &lookup).expect("rewrite");
        let lib_block_idx = rewritten.find("<artifactId>lib</artifactId>").unwrap();
        let after_lib = &rewritten[lib_block_idx..];
        assert!(
            after_lib.contains("<version>2.0.0</version>"),
            "dependencyManagement entry should be 2.0.0; got:\n{after_lib}"
        );
    }
}
