//! Pull Request content generation for release PRs.
//!
//! Generates formatted PR titles and bodies for release pull requests,
//! including version tables, ecosystem badges, and changelog summaries.
//!
//! # Generated PR Format
//!
//! ## Title Examples
//!
//! Single package:
//! ```text
//! chore(release): my-crate v1.2.0
//! ```
//!
//! Multiple packages (≤3):
//! ```text
//! chore(release): core v1.0.0, utils v2.1.0
//! ```
//!
//! Many packages (>3):
//! ```text
//! chore(release): 5 packages
//! ```
//!
//! ## Body Structure
//!
//! ```text
//! ## 🚀 Release Preparation
//!
//! This PR was automatically created by `belaf release prepare`.
//!
//! ### 📦 Packages
//!
//! | Package | Ecosystem | Version | Bump |
//! |---------|-----------|---------|------|
//! | **my-crate** | 🦀 Rust | `1.0.0` → `1.1.0` | 🟡 MINOR |
//!
//! ### 📝 Changelogs
//! [changelog content here]
//!
//! ### 📋 Release Manifest
//! 📄 `belaf/releases/release-20250605-123456.json`
//!
//! ---
//!
//! ### ✅ Next Steps
//! [automation steps]
//! ```

use std::collections::HashMap;

use crate::core::workflow::SelectedProject;

const MAX_PROJECTS_IN_TITLE: usize = 3;

/// Generates the PR title for a release.
///
/// # Output Examples
///
/// - Single: `chore(release): my-crate v1.2.0`
/// - Few: `chore(release): core v1.0.0, utils v2.1.0`
/// - Many: `chore(release): 5 packages`
pub fn generate_pr_title(projects: &[SelectedProject]) -> String {
    if projects.len() == 1 {
        let p = &projects[0];
        format!("chore(release): {} v{}", p.name, p.new_version)
    } else if projects.len() <= MAX_PROJECTS_IN_TITLE {
        let names: Vec<String> = projects
            .iter()
            .map(|p| format!("{} v{}", p.name, p.new_version))
            .collect();
        format!("chore(release): {}", names.join(", "))
    } else {
        format!("chore(release): {} packages", projects.len())
    }
}

/// Generates the PR body with version table, changelogs, and next steps.
///
/// # Sections
///
/// 1. **Packages table** - Shows each package with ecosystem badge, version diff, and bump badge
/// 2. **Changelogs** - Inline for single package, collapsible `<details>` for multiple
/// 3. **Manifest link** - Points to `belaf/releases/{filename}.json`
/// 4. **Next steps** - Documents GitHub App automation
///
/// # Badge Examples
///
/// Ecosystem badges: `🦀 Rust`, `📦 Node.js`, `🐍 Python`, `🐹 Go`
///
/// Bump badges: `🔴 **MAJOR**`, `🟡 MINOR`, `🟢 patch`
pub fn generate_pr_body(
    projects: &[SelectedProject],
    manifest_filename: &str,
    changelog_contents: &HashMap<String, String>,
) -> String {
    let mut body = String::new();

    body.push_str("## 🚀 Release Preparation\n\n");
    body.push_str("This PR was automatically created by `belaf release prepare`.\n\n");

    body.push_str("### 📦 Packages\n\n");
    body.push_str("| Package | Ecosystem | Version | Bump |\n");
    body.push_str("|---------|-----------|---------|------|\n");

    for project in projects {
        body.push_str(&format!(
            "| **{}** | {} | `{}` → `{}` | {} |\n",
            project.name,
            ecosystem_badge(project.ecosystem.display_name()),
            project.old_version,
            project.new_version,
            bump_badge(&project.bump_type)
        ));
    }

    body.push_str("\n### 📝 Changelogs\n\n");

    if projects.len() == 1 {
        let project = &projects[0];
        if let Some(changelog) = changelog_contents.get(&project.name) {
            body.push_str(changelog);
            body.push('\n');
        }
    } else {
        for project in projects {
            if let Some(changelog) = changelog_contents.get(&project.name) {
                body.push_str(&format!(
                    "<details>\n<summary><strong>{}</strong> - {} → {}</summary>\n\n",
                    project.name, project.old_version, project.new_version
                ));
                body.push_str(changelog);
                body.push_str("\n</details>\n\n");
            }
        }
    }

    body.push_str("### 📋 Release Manifest\n\n");
    body.push_str(&format!("📄 `belaf/releases/{}`\n\n", manifest_filename));

    body.push_str("---\n\n");
    body.push_str("### ✅ Next Steps\n\n");
    body.push_str("After merging this PR, the **belaf GitHub App** will automatically:\n");
    body.push_str("1. Create Git tags for each package\n");
    body.push_str("2. Create GitHub Releases with changelogs\n");
    body.push_str("3. Trigger any configured release workflows\n");

    body
}

fn ecosystem_badge(ecosystem: &str) -> String {
    match ecosystem {
        "Rust" => "🦀 Rust".to_string(),
        "Node.js" => "📦 Node.js".to_string(),
        "Python" => "🐍 Python".to_string(),
        "Go" => "🐹 Go".to_string(),
        "Elixir" => "💧 Elixir".to_string(),
        "C#" => "🔷 C#".to_string(),
        _ => ecosystem.to_string(),
    }
}

fn bump_badge(bump_type: &str) -> String {
    match bump_type.to_lowercase().as_str() {
        "major" => "🔴 **MAJOR**".to_string(),
        "minor" => "🟡 MINOR".to_string(),
        "patch" => "🟢 patch".to_string(),
        _ => bump_type.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ecosystem::types::EcosystemType;

    fn make_project(name: &str, old: &str, new: &str, bump: &str) -> SelectedProject {
        SelectedProject {
            name: name.to_string(),
            prefix: String::new(),
            old_version: old.to_string(),
            new_version: new.to_string(),
            bump_type: bump.to_string(),
            commits: vec![],
            ecosystem: EcosystemType::Cargo,
            cached_changelog: None,
        }
    }

    #[test]
    fn test_pr_title_single_project() {
        let projects = vec![make_project("my-crate", "1.0.0", "1.1.0", "minor")];
        let title = generate_pr_title(&projects);
        assert_eq!(title, "chore(release): my-crate v1.1.0");
    }

    #[test]
    fn test_pr_title_two_projects() {
        let projects = vec![
            make_project("core", "1.0.0", "1.1.0", "minor"),
            make_project("utils", "2.0.0", "2.0.1", "patch"),
        ];
        let title = generate_pr_title(&projects);
        assert_eq!(title, "chore(release): core v1.1.0, utils v2.0.1");
    }

    #[test]
    fn test_pr_title_three_projects() {
        let projects = vec![
            make_project("a", "1.0.0", "1.0.1", "patch"),
            make_project("b", "2.0.0", "2.1.0", "minor"),
            make_project("c", "3.0.0", "4.0.0", "major"),
        ];
        let title = generate_pr_title(&projects);
        assert_eq!(title, "chore(release): a v1.0.1, b v2.1.0, c v4.0.0");
    }

    #[test]
    fn test_pr_title_many_projects() {
        let projects = vec![
            make_project("a", "1.0.0", "1.0.1", "patch"),
            make_project("b", "2.0.0", "2.0.1", "patch"),
            make_project("c", "3.0.0", "3.0.1", "patch"),
            make_project("d", "4.0.0", "4.0.1", "patch"),
            make_project("e", "5.0.0", "5.0.1", "patch"),
        ];
        let title = generate_pr_title(&projects);
        assert_eq!(title, "chore(release): 5 packages");
    }

    #[test]
    fn test_pr_body_contains_packages_table() {
        let projects = vec![make_project("test-crate", "1.0.0", "2.0.0", "major")];
        let changelog_contents = HashMap::new();
        let body = generate_pr_body(&projects, "release-test.json", &changelog_contents);

        assert!(body.contains("## 🚀 Release Preparation"));
        assert!(body.contains("### 📦 Packages"));
        assert!(body.contains("| Package | Ecosystem | Version | Bump |"));
        assert!(body.contains("| **test-crate** |"));
        assert!(body.contains("`1.0.0` → `2.0.0`"));
        assert!(body.contains("🔴 **MAJOR**"));
    }

    #[test]
    fn test_pr_body_contains_manifest_link() {
        let projects = vec![make_project("test", "1.0.0", "1.0.1", "patch")];
        let body = generate_pr_body(&projects, "release-20250101-abc123.json", &HashMap::new());

        assert!(body.contains("### 📋 Release Manifest"));
        assert!(body.contains("📄 `belaf/releases/release-20250101-abc123.json`"));
    }

    #[test]
    fn test_pr_body_contains_next_steps() {
        let projects = vec![make_project("test", "1.0.0", "1.0.1", "patch")];
        let body = generate_pr_body(&projects, "release.json", &HashMap::new());

        assert!(body.contains("### ✅ Next Steps"));
        assert!(body.contains("belaf GitHub App"));
        assert!(body.contains("Create Git tags"));
        assert!(body.contains("Create GitHub Releases"));
    }

    #[test]
    fn test_pr_body_single_project_inline_changelog() {
        let projects = vec![make_project("my-crate", "1.0.0", "1.1.0", "minor")];
        let mut changelog_contents = HashMap::new();
        changelog_contents.insert(
            "my-crate".to_string(),
            "## Features\n- Added new feature".to_string(),
        );
        let body = generate_pr_body(&projects, "release.json", &changelog_contents);

        assert!(body.contains("### 📝 Changelogs"));
        assert!(body.contains("## Features"));
        assert!(body.contains("- Added new feature"));
        assert!(!body.contains("<details>"));
    }

    #[test]
    fn test_pr_body_multiple_projects_collapsible_changelogs() {
        let projects = vec![
            make_project("core", "1.0.0", "1.1.0", "minor"),
            make_project("utils", "2.0.0", "2.0.1", "patch"),
        ];
        let mut changelog_contents = HashMap::new();
        changelog_contents.insert("core".to_string(), "Core changes".to_string());
        changelog_contents.insert("utils".to_string(), "Utils fixes".to_string());
        let body = generate_pr_body(&projects, "release.json", &changelog_contents);

        assert!(body.contains("<details>"));
        assert!(body.contains("<summary><strong>core</strong>"));
        assert!(body.contains("<summary><strong>utils</strong>"));
        assert!(body.contains("</details>"));
    }

    #[test]
    fn test_ecosystem_badges() {
        assert_eq!(ecosystem_badge("Rust"), "🦀 Rust");
        assert_eq!(ecosystem_badge("Node.js"), "📦 Node.js");
        assert_eq!(ecosystem_badge("Python"), "🐍 Python");
        assert_eq!(ecosystem_badge("Go"), "🐹 Go");
        assert_eq!(ecosystem_badge("Elixir"), "💧 Elixir");
        assert_eq!(ecosystem_badge("C#"), "🔷 C#");
        assert_eq!(ecosystem_badge("Unknown"), "Unknown");
    }

    #[test]
    fn test_bump_badges() {
        assert_eq!(bump_badge("major"), "🔴 **MAJOR**");
        assert_eq!(bump_badge("MAJOR"), "🔴 **MAJOR**");
        assert_eq!(bump_badge("minor"), "🟡 MINOR");
        assert_eq!(bump_badge("Minor"), "🟡 MINOR");
        assert_eq!(bump_badge("patch"), "🟢 patch");
        assert_eq!(bump_badge("PATCH"), "🟢 patch");
        assert_eq!(bump_badge("custom"), "custom");
    }
}
