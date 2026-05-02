//! Hint and ExternallyManaged scanners. Bundle scanners moved to
//! [`super::super::bundle::*`] in Schicht 2 — adding a new bundle is
//! a single new file under `bundle/`, not an edit here.
//!
//! Each scanner takes the repo's working directory and returns the
//! matches it found. Pure-functional — filesystem only, no git access.

use std::path::{Path, PathBuf};

use crate::core::git::repository::RepoPathBuf;

use super::super::walk::{
    cargo_toml_has_workspace_section, file_contains_pattern, relative_repopath, walk_capped,
};
use super::{DetectedShape, DetectorMatch, ExtKind, HintKind, SingleProjectEcosystem};

// Mobile app — ExternallyManaged

pub(super) fn mobile_app(workdir: &Path) -> Vec<DetectorMatch> {
    let mut out = Vec::new();
    for ios_dir in find_xcodeproj_parents(workdir) {
        let repopath = match relative_repopath(workdir, &ios_dir) {
            Some(r) => r,
            None => continue,
        };
        out.push(DetectorMatch {
            shape: DetectedShape::ExternallyManaged(ExtKind::MobileIos),
            path: repopath,
            note: Some("iOS app (recommend Bitrise/fastlane)".to_string()),
        });
    }
    for android_dir in find_android_app_dirs(workdir) {
        let repopath = match relative_repopath(workdir, &android_dir) {
            Some(r) => r,
            None => continue,
        };
        out.push(DetectorMatch {
            shape: DetectedShape::ExternallyManaged(ExtKind::MobileAndroid),
            path: repopath,
            note: Some("Android app (recommend Bitrise/Codemagic)".to_string()),
        });
    }
    out
}

fn find_xcodeproj_parents(workdir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_capped(workdir, 6, |p| {
        if p.is_dir() && p.extension().and_then(|s| s.to_str()) == Some("xcodeproj") {
            let pbx = p.join("project.pbxproj");
            if pbx.exists() {
                if let Some(parent) = p.parent() {
                    out.push(parent.to_path_buf());
                }
            }
        }
    });
    out
}

fn find_android_app_dirs(workdir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_capped(workdir, 5, |p| {
        let bgk = p.join("build.gradle.kts");
        let bg = p.join("build.gradle");
        for f in [&bgk, &bg] {
            if f.exists()
                && file_contains_pattern(f, r"versionName\s*=")
                && file_contains_pattern(f, r"versionCode\s*=")
            {
                out.push(p.to_path_buf());
                break;
            }
        }
    });
    out
}

// Nested npm workspace — Hint

pub(super) fn nested_npm_workspace(workdir: &Path) -> Vec<DetectorMatch> {
    let mut out = Vec::new();
    walk_capped(workdir, 5, |p| {
        if p == workdir {
            return;
        }
        let pkg = p.join("package.json");
        if pkg.exists() && file_contains_pattern(&pkg, r#""workspaces"\s*:"#) {
            if let Some(repopath) = relative_repopath(workdir, p) {
                out.push(DetectorMatch {
                    shape: DetectedShape::Hint(HintKind::NpmWorkspace),
                    path: repopath,
                    note: None,
                });
            }
        }
    });
    out
}

// SDK cascade members — Hint

pub(super) fn sdk_cascade_members(workdir: &Path) -> Vec<DetectorMatch> {
    let mut out = Vec::new();
    let sdks = workdir.join("sdks");
    if !sdks.is_dir() {
        return out;
    }
    if let Ok(entries) = std::fs::read_dir(&sdks) {
        for e in entries.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            let indicators = [
                "graphql-codegen.yml",
                "graphql-codegen.yaml",
                "orval.config.ts",
                "orval.config.js",
                "apollo.gradle.kts",
                "swift-codegen.yml",
                "openapi-generator.yaml",
                "openapi-generator.yml",
            ];
            let has_indicator = indicators.iter().any(|f| p.join(f).exists());
            let has_package = p.join("package.json").exists()
                || p.join("Cargo.toml").exists()
                || p.join("Package.swift").exists()
                || p.join("gradle.properties").exists();

            if has_indicator || has_package {
                if let Some(repopath) = relative_repopath(workdir, &p) {
                    out.push(DetectorMatch {
                        shape: DetectedShape::Hint(HintKind::SdkCascade),
                        path: repopath,
                        note: None,
                    });
                }
            }
        }
    }
    out
}

// Single-project repo — Hint

pub(super) fn single_project_repo(workdir: &Path) -> Vec<DetectorMatch> {
    let cargo_at_root = workdir.join("Cargo.toml").is_file();
    let pkg_at_root = workdir.join("package.json").is_file();
    let pyproj_at_root = workdir.join("pyproject.toml").is_file();
    let setup_at_root = workdir.join("setup.py").is_file();
    let go_mod_at_root = workdir.join("go.mod").is_file();
    let pom_at_root = workdir.join("pom.xml").is_file();
    let pkg_swift_at_root = workdir.join("Package.swift").is_file();
    let mix_at_root = workdir.join("mix.exs").is_file();

    let manifest_count = [
        cargo_at_root,
        pkg_at_root,
        pyproj_at_root || setup_at_root,
        go_mod_at_root,
        pom_at_root,
        pkg_swift_at_root,
        mix_at_root,
    ]
    .iter()
    .filter(|present| **present)
    .count();

    if manifest_count != 1 {
        return Vec::new();
    }

    if cargo_at_root && cargo_toml_has_workspace_section(&workdir.join("Cargo.toml")) {
        return Vec::new();
    }
    if pkg_at_root && file_contains_pattern(&workdir.join("package.json"), r#""workspaces"\s*:"#) {
        return Vec::new();
    }

    if has_nested_manifest(workdir) {
        return Vec::new();
    }

    let ecosystem = if cargo_at_root {
        SingleProjectEcosystem::Cargo
    } else if pkg_at_root {
        SingleProjectEcosystem::Npm
    } else if pyproj_at_root || setup_at_root {
        SingleProjectEcosystem::Pypa
    } else if go_mod_at_root {
        SingleProjectEcosystem::Go
    } else if pom_at_root {
        SingleProjectEcosystem::Maven
    } else if pkg_swift_at_root {
        SingleProjectEcosystem::Swift
    } else if mix_at_root {
        SingleProjectEcosystem::Elixir
    } else {
        return Vec::new();
    };

    vec![DetectorMatch {
        shape: DetectedShape::Hint(HintKind::SingleProject { ecosystem }),
        path: RepoPathBuf::new(b"."),
        note: None,
    }]
}

fn has_nested_manifest(workdir: &Path) -> bool {
    let mut found = false;
    walk_capped(workdir, 4, |p| {
        if found || p == workdir {
            return;
        }
        if p.join("Cargo.toml").is_file()
            || p.join("package.json").is_file()
            || p.join("pyproject.toml").is_file()
            || p.join("setup.py").is_file()
            || p.join("go.mod").is_file()
            || p.join("pom.xml").is_file()
            || p.join("Package.swift").is_file()
            || p.join("mix.exs").is_file()
            || p.join("gradle.properties").is_file()
            || p.join("build.gradle.kts").is_file()
        {
            found = true;
        }
    });
    found
}

// Nested monorepo — Hint

pub(super) fn nested_monorepo(workdir: &Path) -> Vec<DetectorMatch> {
    let gitmodules = workdir.join(".gitmodules");
    if !gitmodules.is_file() {
        return Vec::new();
    }
    let content = match std::fs::read_to_string(&gitmodules) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut out = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("path") {
            let rest = rest.trim_start();
            let value = rest.strip_prefix('=').map(|v| v.trim()).unwrap_or("");
            if value.is_empty() {
                continue;
            }
            let sub = workdir.join(value);
            if !sub.is_dir() {
                continue;
            }
            let has_belaf_config = sub.join("belaf").join("config.toml").is_file();
            let manifest_count = [
                "Cargo.toml",
                "package.json",
                "pyproject.toml",
                "go.mod",
                "pom.xml",
                "Package.swift",
                "mix.exs",
            ]
            .iter()
            .filter(|f| sub.join(*f).is_file())
            .count();

            if has_belaf_config || manifest_count >= 2 {
                if let Some(repopath) = relative_repopath(workdir, &sub) {
                    out.push(DetectorMatch {
                        shape: DetectedShape::Hint(HintKind::NestedMonorepo),
                        path: repopath,
                        note: Some(if has_belaf_config {
                            "submodule has its own belaf/config.toml".to_string()
                        } else {
                            "submodule holds multiple manifests".to_string()
                        }),
                    });
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(p: &Path, content: &str) {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, content).unwrap();
    }

    #[test]
    fn mobile_app_ios_detected() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("apps/clients/ios/MyApp.xcodeproj/project.pbxproj"),
            "// not really a pbxproj — just needs to exist for detection\n",
        );
        let matches = mobile_app(root);
        let ios: Vec<_> = matches
            .iter()
            .filter(|m| matches!(m.shape, DetectedShape::ExternallyManaged(ExtKind::MobileIos)))
            .collect();
        assert_eq!(ios.len(), 1);
        assert_eq!(ios[0].path.escaped(), "apps/clients/ios");
    }

    #[test]
    fn mobile_app_android_detected_via_dual_version() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("apps/clients/android/app/build.gradle.kts"),
            "android {\n  defaultConfig {\n    versionName = \"1.0\"\n    versionCode = 1\n  }\n}\n",
        );
        let matches = mobile_app(root);
        let android: Vec<_> = matches
            .iter()
            .filter(|m| {
                matches!(
                    m.shape,
                    DetectedShape::ExternallyManaged(ExtKind::MobileAndroid)
                )
            })
            .collect();
        assert_eq!(android.len(), 1);
    }

    #[test]
    fn nested_npm_workspace_detected() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(&root.join("package.json"), r#"{"name":"root"}"#);
        write(
            &root.join("apps/dashboards/docs/package.json"),
            r#"{"name":"docs","workspaces":["packages/*"]}"#,
        );
        let matches = nested_npm_workspace(root);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].path.escaped(), "apps/dashboards/docs");
    }

    #[test]
    fn sdk_cascade_members_via_codegen_indicator() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("sdks/typescript/graphql-codegen.yml"),
            "schema: ../../proto",
        );
        write(
            &root.join("sdks/typescript/package.json"),
            r#"{"version":"0.1.0"}"#,
        );
        let matches = sdk_cascade_members(root);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].path.escaped(), "sdks/typescript");
    }

    #[test]
    fn single_project_detects_lone_cargo_crate() {
        let t = TempDir::new().unwrap();
        write(
            &t.path().join("Cargo.toml"),
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n",
        );
        let matches = single_project_repo(t.path());
        assert_eq!(matches.len(), 1);
        match &matches[0].shape {
            DetectedShape::Hint(HintKind::SingleProject { ecosystem }) => {
                assert_eq!(*ecosystem, SingleProjectEcosystem::Cargo);
            }
            other => panic!("expected SingleProject; got {other:?}"),
        }
    }

    #[test]
    fn single_project_skips_cargo_workspace() {
        let t = TempDir::new().unwrap();
        write(
            &t.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"a\"]\n",
        );
        write(
            &t.path().join("a/Cargo.toml"),
            "[package]\nname = \"a\"\nversion = \"0.1.0\"\n",
        );
        let matches = single_project_repo(t.path());
        assert!(
            matches.is_empty(),
            "workspace root must not trigger single-project; got {matches:?}"
        );
    }

    #[test]
    fn single_project_skips_npm_workspace() {
        let t = TempDir::new().unwrap();
        write(
            &t.path().join("package.json"),
            r#"{"name":"r","version":"1.0.0","workspaces":["packages/*"]}"#,
        );
        write(
            &t.path().join("packages/a/package.json"),
            r#"{"name":"a","version":"1.0.0"}"#,
        );
        let matches = single_project_repo(t.path());
        assert!(matches.is_empty());
    }

    #[test]
    fn single_project_skips_when_nested_manifest_present() {
        let t = TempDir::new().unwrap();
        write(
            &t.path().join("package.json"),
            r#"{"name":"top","version":"1.0.0"}"#,
        );
        write(
            &t.path().join("nested/Cargo.toml"),
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n",
        );
        let matches = single_project_repo(t.path());
        assert!(matches.is_empty());
    }

    #[test]
    fn nested_monorepo_detects_submodule_with_belaf_config() {
        let t = TempDir::new().unwrap();
        write(
            &t.path().join(".gitmodules"),
            "[submodule \"vendor/foo\"]\n\tpath = vendor/foo\n\turl = https://example.com\n",
        );
        write(
            &t.path().join("vendor/foo/belaf/config.toml"),
            "[repo]\nupstream_urls = []\n",
        );
        let matches = nested_monorepo(t.path());
        assert_eq!(matches.len(), 1);
        assert_eq!(
            matches[0].shape,
            DetectedShape::Hint(HintKind::NestedMonorepo)
        );
        assert_eq!(matches[0].path.escaped(), "vendor/foo");
    }

    #[test]
    fn nested_monorepo_detects_submodule_with_multiple_manifests() {
        let t = TempDir::new().unwrap();
        write(
            &t.path().join(".gitmodules"),
            "[submodule \"sub\"]\n\tpath = sub\n\turl = https://example.com\n",
        );
        write(
            &t.path().join("sub/Cargo.toml"),
            "[workspace]\nmembers = []\n",
        );
        write(
            &t.path().join("sub/package.json"),
            r#"{"name":"sub","version":"1.0.0"}"#,
        );
        let matches = nested_monorepo(t.path());
        assert_eq!(matches.len(), 1);
    }

    #[test]
    fn nested_monorepo_ignores_simple_submodules() {
        let t = TempDir::new().unwrap();
        write(
            &t.path().join(".gitmodules"),
            "[submodule \"plain\"]\n\tpath = plain\n\turl = https://example.com\n",
        );
        write(
            &t.path().join("plain/Cargo.toml"),
            "[package]\nname = \"plain\"\nversion = \"0.1.0\"\n",
        );
        let matches = nested_monorepo(t.path());
        assert!(matches.is_empty());
    }
}
