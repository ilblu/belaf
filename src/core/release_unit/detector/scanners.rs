//! Per-detector scan functions. Each one takes the repo's working
//! directory and returns the matches it found. Pure-functional —
//! filesystem only, no git access.

use std::path::{Path, PathBuf};

use crate::core::git::repository::RepoPathBuf;

use super::walk::{
    cargo_toml_has_package_section, cargo_toml_has_workspace_section, file_contains_line,
    file_contains_pattern, find_dirs_with_files_set, find_dirs_with_subdir_pattern,
    list_subdirs_with_file, relative_repopath, walk_capped,
};
use super::{
    BundleKind, DetectedShape, DetectorMatch, ExtKind, HexagonalPrimary, HintKind,
    JvmVersionSource, SingleProjectEcosystem,
};

// F.1 — Hexagonal cargo

pub(super) fn hexagonal_cargo(workdir: &Path) -> Vec<DetectorMatch> {
    let mut out = Vec::new();
    let crates_dirs = find_dirs_with_subdir_pattern(workdir, "crates");
    for crates_dir in crates_dirs {
        let service_dir = match crates_dir.parent() {
            Some(p) => p,
            None => continue,
        };
        let cargo_subs = list_subdirs_with_file(&crates_dir, "Cargo.toml");
        if cargo_subs.len() < 2 {
            continue;
        }
        let basename = service_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        let primaries = [
            ("bin", HexagonalPrimary::Bin),
            ("lib", HexagonalPrimary::Lib),
            ("workers", HexagonalPrimary::Workers),
            (basename, HexagonalPrimary::BaseName),
        ];
        let mut found_primary: Option<HexagonalPrimary> = None;
        for (sub, kind) in primaries {
            let sub_cargo = crates_dir.join(sub).join("Cargo.toml");
            if sub_cargo.exists() && cargo_toml_has_package_section(&sub_cargo) {
                found_primary = Some(kind);
                break;
            }
        }
        let Some(primary) = found_primary else {
            continue;
        };

        let repopath = match relative_repopath(workdir, service_dir) {
            Some(r) => r,
            None => continue,
        };
        out.push(DetectorMatch {
            shape: DetectedShape::Bundle(BundleKind::HexagonalCargo { primary }),
            path: repopath,
            note: Some(format!(
                "primary crate: {}",
                primary_label(primary, basename)
            )),
        });
    }
    out
}

fn primary_label(p: HexagonalPrimary, basename: &str) -> &str {
    match p {
        HexagonalPrimary::Bin => "bin",
        HexagonalPrimary::Lib => "lib",
        HexagonalPrimary::Workers => "workers",
        HexagonalPrimary::BaseName => basename,
    }
}

// F.2 — Tauri (single-source vs legacy multi-file)

pub(super) fn tauri(workdir: &Path) -> Vec<DetectorMatch> {
    let mut out = Vec::new();
    for triplet_root in find_dirs_with_files_set(
        workdir,
        &[
            "package.json",
            "src-tauri/Cargo.toml",
            "src-tauri/tauri.conf.json",
        ],
    ) {
        let conf_path = triplet_root.join("src-tauri/tauri.conf.json");
        let single_source = is_tauri_single_source(&conf_path);
        let repopath = match relative_repopath(workdir, &triplet_root) {
            Some(r) => r,
            None => continue,
        };
        out.push(DetectorMatch {
            shape: DetectedShape::Bundle(BundleKind::Tauri { single_source }),
            path: repopath,
            note: Some(if single_source {
                "single-source (version derived from package.json)".to_string()
            } else {
                "legacy multi-file (3 files hand-managed)".to_string()
            }),
        });
    }
    out
}

static TAURI_PATH_REF_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r#""version"\s*:\s*"\.\./[^"]+\.json""#).expect("static regex must compile")
});
static TAURI_ANY_VERSION_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r#""version"\s*:\s*"[^"]+""#).expect("static regex must compile")
});

fn is_tauri_single_source(conf_path: &Path) -> bool {
    let content = match std::fs::read_to_string(conf_path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    if TAURI_PATH_REF_RE.is_match(&content) {
        return true;
    }
    !TAURI_ANY_VERSION_RE.is_match(&content)
}

// F.3 — JVM library

pub(super) fn jvm_library(workdir: &Path) -> Vec<DetectorMatch> {
    let mut out = Vec::new();
    let candidates = collect_jvm_candidates(workdir);
    for dir in candidates {
        let gp = dir.join("gradle.properties");
        let bgk = dir.join("build.gradle.kts");
        let bg = dir.join("build.gradle");

        let kind = if gp.exists() && file_contains_line(&gp, "version=") {
            JvmVersionSource::GradleProperties
        } else if (bgk.exists() && file_contains_pattern(&bgk, r#"version\s*=\s*""#))
            || (bg.exists() && file_contains_pattern(&bg, r#"version\s*=\s*""#))
        {
            JvmVersionSource::BuildGradleKtsLiteral
        } else if bgk.exists() || bg.exists() {
            JvmVersionSource::PluginManaged
        } else {
            continue;
        };

        let repopath = match relative_repopath(workdir, &dir) {
            Some(r) => r,
            None => continue,
        };
        out.push(DetectorMatch {
            shape: DetectedShape::Bundle(BundleKind::JvmLibrary {
                version_source: kind.clone(),
            }),
            path: repopath,
            note: Some(jvm_label(&kind).to_string()),
        });
    }
    out
}

fn jvm_label(s: &JvmVersionSource) -> &'static str {
    match s {
        JvmVersionSource::GradleProperties => "gradle.properties (recommended)",
        JvmVersionSource::BuildGradleKtsLiteral => "literal version in build.gradle(.kts)",
        JvmVersionSource::PluginManaged => "plugin-managed (suggest external_versioner)",
    }
}

fn collect_jvm_candidates(workdir: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(workdir.join("sdks")) {
        for e in entries.flatten() {
            if e.path().is_dir() {
                dirs.push(e.path());
            }
        }
    }
    if let Ok(entries) = std::fs::read_dir(workdir.join("libs")) {
        for e in entries.flatten() {
            if e.path().is_dir() {
                dirs.push(e.path());
            }
        }
    }
    if workdir.join("gradle.properties").exists() || workdir.join("build.gradle.kts").exists() {
        dirs.push(workdir.to_path_buf());
    }
    dirs
}

// F.4 — Mobile app (warning only)

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

// F.6 — Nested npm workspace

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

// F.7 — SDK cascade members

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

// F.8 — Single-project repo

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

// F.9 — Nested monorepo (git submodule with its own belaf config)

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
    fn hexagonal_cargo_detects_bin_primary() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("apps/services/aura/crates/bin/Cargo.toml"),
            "[package]\nname = \"aura-bin\"\nversion = \"0.1.0\"\n",
        );
        write(
            &root.join("apps/services/aura/crates/api/Cargo.toml"),
            "[package]\nname = \"aura-api\"\nversion = \"0.1.0\"\n",
        );
        let matches = hexagonal_cargo(root);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].path.escaped(), "apps/services/aura");
        match &matches[0].shape {
            DetectedShape::Bundle(BundleKind::HexagonalCargo { primary }) => {
                assert_eq!(*primary, HexagonalPrimary::Bin);
            }
            _ => panic!("expected HexagonalCargo"),
        }
    }

    #[test]
    fn hexagonal_cargo_detects_workers_fallback() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("apps/services/mondo/crates/workers/Cargo.toml"),
            "[package]\nname = \"mondo-workers\"\nversion = \"0.1.0\"\n",
        );
        write(
            &root.join("apps/services/mondo/crates/core/Cargo.toml"),
            "[package]\nname = \"mondo-core\"\nversion = \"0.1.0\"\n",
        );
        let matches = hexagonal_cargo(root);
        assert_eq!(matches.len(), 1);
        match &matches[0].shape {
            DetectedShape::Bundle(BundleKind::HexagonalCargo { primary }) => {
                assert_eq!(*primary, HexagonalPrimary::Workers);
            }
            _ => panic!("expected HexagonalCargo"),
        }
    }

    #[test]
    fn hexagonal_cargo_skips_when_only_one_crate_subdir() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("apps/foo/crates/bin/Cargo.toml"),
            "[package]\nname = \"foo\"\nversion = \"0.1.0\"\n",
        );
        let matches = hexagonal_cargo(root);
        assert!(matches.is_empty());
    }

    #[test]
    fn tauri_single_source_via_path_ref() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("apps/desktop/package.json"),
            r#"{"version":"0.1.0"}"#,
        );
        write(
            &root.join("apps/desktop/src-tauri/Cargo.toml"),
            "[package]\nname = \"desktop\"\nversion = \"0.0.0\"\n",
        );
        write(
            &root.join("apps/desktop/src-tauri/tauri.conf.json"),
            r#"{"productName":"desktop","version":"../package.json"}"#,
        );
        let matches = tauri(root);
        assert_eq!(matches.len(), 1);
        match &matches[0].shape {
            DetectedShape::Bundle(BundleKind::Tauri { single_source }) => assert!(*single_source),
            _ => panic!("expected Tauri"),
        }
    }

    #[test]
    fn tauri_legacy_multi_file() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("apps/desktop/package.json"),
            r#"{"version":"0.1.0"}"#,
        );
        write(
            &root.join("apps/desktop/src-tauri/Cargo.toml"),
            "[package]\nname = \"desktop\"\nversion = \"0.1.0\"\n",
        );
        write(
            &root.join("apps/desktop/src-tauri/tauri.conf.json"),
            r#"{"productName":"desktop","version":"0.1.0"}"#,
        );
        let matches = tauri(root);
        assert_eq!(matches.len(), 1);
        match &matches[0].shape {
            DetectedShape::Bundle(BundleKind::Tauri { single_source }) => assert!(!*single_source),
            _ => panic!("expected Tauri"),
        }
    }

    #[test]
    fn jvm_library_gradle_properties() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("sdks/kotlin/gradle.properties"),
            "version=0.1.0\n",
        );
        let matches = jvm_library(root);
        assert_eq!(matches.len(), 1);
        match &matches[0].shape {
            DetectedShape::Bundle(BundleKind::JvmLibrary { version_source }) => {
                assert_eq!(*version_source, JvmVersionSource::GradleProperties);
            }
            _ => panic!("expected JvmLibrary"),
        }
    }

    #[test]
    fn jvm_library_build_gradle_kts_literal() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("sdks/java/build.gradle.kts"),
            "plugins {}\nversion = \"0.1.0\"\n",
        );
        let matches = jvm_library(root);
        assert_eq!(matches.len(), 1);
        match &matches[0].shape {
            DetectedShape::Bundle(BundleKind::JvmLibrary { version_source }) => {
                assert_eq!(*version_source, JvmVersionSource::BuildGradleKtsLiteral);
            }
            _ => panic!("expected JvmLibrary"),
        }
    }

    #[test]
    fn jvm_library_plugin_managed() {
        let t = TempDir::new().unwrap();
        let root = t.path();
        write(
            &root.join("sdks/javakt/build.gradle.kts"),
            "plugins { id(\"io.github.reactivecircus.app-versioning\") version \"1.3.1\" }\n",
        );
        let matches = jvm_library(root);
        assert_eq!(matches.len(), 1);
        match &matches[0].shape {
            DetectedShape::Bundle(BundleKind::JvmLibrary { version_source }) => {
                assert_eq!(*version_source, JvmVersionSource::PluginManaged);
            }
            _ => panic!("expected JvmLibrary"),
        }
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
            .filter(|m| {
                matches!(m.shape, DetectedShape::ExternallyManaged(ExtKind::MobileIos))
            })
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
        assert_eq!(matches[0].shape, DetectedShape::Hint(HintKind::NestedMonorepo));
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
