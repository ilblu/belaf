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

use crate::core::workflow::SelectedReleaseUnit;

const MAX_PROJECTS_IN_TITLE: usize = 3;

/// Generates the PR title for a release.
///
/// # Output Examples
///
/// - Single: `chore(release): my-crate v1.2.0`
/// - Few: `chore(release): core v1.0.0, utils v2.1.0`
/// - Many: `chore(release): 5 packages`
pub fn generate_pr_title(projects: &[SelectedReleaseUnit]) -> String {
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

/// The most a pull request body may hold. GitHub refuses anything longer
/// with `422 body is too long (maximum is 65536 characters)`.
pub const GITHUB_PR_BODY_LIMIT: usize = 65_536;

/// What [`generate_pr_body`] stays under, measured in bytes. Bytes are never
/// fewer than the characters or UTF-16 units GitHub counts, and the gap to
/// [`GITHUB_PR_BODY_LIMIT`] leaves room for whatever GitHub adds on its side.
pub const PR_BODY_BUDGET: usize = 60_000;

/// Room kept free while the package table is written, so the manifest link,
/// the next steps and the changelog references always fit after it.
const TABLE_RESERVE: usize = 4_000;

/// Generates the PR body with version table, changelogs, and next steps.
///
/// # Sections
///
/// 1. **Packages table** - Shows each package with ecosystem badge, version diff, and bump badge
/// 2. **Changelogs** - Inline for single package, collapsible `<details>` for multiple
/// 3. **Manifest link** - Points to `belaf/releases/{filename}.json`
/// 4. **Next steps** - Documents GitHub App automation
///
/// # Size
///
/// The body never exceeds [`PR_BODY_BUDGET`]. A week of commits across a
/// monorepo with twenty units easily writes twice GitHub's limit in
/// changelogs, so changelogs are included in package order while they fit,
/// and every package that no longer fits is listed with the `CHANGELOG.md`
/// its entry was written to. The release commit carries those files and the
/// GitHub App puts the full text into each tag's release, so nothing is lost.
///
/// # Badge Examples
///
/// Ecosystem badges: `🦀 Rust`, `📦 Node.js`, `🐍 Python`, `🐹 Go`
///
/// Bump badges: `🔴 **MAJOR**`, `🟡 MINOR`, `🟢 patch`
pub fn generate_pr_body(
    projects: &[SelectedReleaseUnit],
    manifest_filename: &str,
    changelog_contents: &HashMap<String, String>,
    changelog_files: &HashMap<String, String>,
) -> String {
    let tail = body_tail(manifest_filename);
    let mut body = String::new();

    body.push_str("## 🚀 Release Preparation\n\n");
    body.push_str("This PR was automatically created by `belaf release prepare`.\n\n");
    push_packages_table(
        &mut body,
        projects,
        PR_BODY_BUDGET - tail.len() - TABLE_RESERVE,
    );

    let with_changelog: Vec<&SelectedReleaseUnit> = projects
        .iter()
        .filter(|p| changelog_contents.contains_key(&p.name))
        .collect();

    if !with_changelog.is_empty() {
        body.push_str("\n### 📝 Changelogs\n\n");
        push_changelogs(
            &mut body,
            &with_changelog,
            changelog_contents,
            changelog_files,
            PR_BODY_BUDGET - tail.len(),
        );
    } else {
        body.push('\n');
    }

    body.push_str(&tail);
    debug_assert!(body.len() <= PR_BODY_BUDGET, "PR body over budget");
    body
}

fn body_tail(manifest_filename: &str) -> String {
    let mut tail = String::new();
    tail.push_str("### 📋 Release Manifest\n\n");
    tail.push_str(&format!("📄 `belaf/releases/{}`\n\n", manifest_filename));
    tail.push_str("---\n\n");
    tail.push_str("### ✅ Next Steps\n\n");
    tail.push_str("After merging this PR, the **belaf GitHub App** will automatically:\n");
    tail.push_str("1. Create Git tags for each package\n");
    tail.push_str("2. Create GitHub Releases with changelogs\n");
    tail.push_str("3. Trigger any configured release workflows\n");
    tail
}

fn push_packages_table(body: &mut String, projects: &[SelectedReleaseUnit], limit: usize) {
    body.push_str("### 📦 Packages\n\n");
    body.push_str("| Package | Ecosystem | Version | Bump |\n");
    body.push_str("|---------|-----------|---------|------|\n");

    for (index, project) in projects.iter().enumerate() {
        let row = format!(
            "| **{}** | {} | `{}` → `{}` | {} |\n",
            project.name,
            ecosystem_badge(project.ecosystem.display_name()),
            project.old_version,
            project.new_version,
            bump_badge(&project.bump_type)
        );
        if body.len() + row.len() > limit {
            body.push_str(&format!(
                "\n…and {} more packages, all listed in the release manifest below.\n",
                projects.len() - index
            ));
            return;
        }
        body.push_str(&row);
    }
}

fn changelog_block(project: &SelectedReleaseUnit, changelog: &str, inline: bool) -> String {
    if inline {
        return format!("{}\n", changelog);
    }
    format!(
        "<details>\n<summary><strong>{}</strong> - {} → {}</summary>\n\n{}\n</details>\n\n",
        project.name, project.old_version, project.new_version, changelog
    )
}

fn reference_line(
    project: &SelectedReleaseUnit,
    changelog_files: &HashMap<String, String>,
) -> String {
    match changelog_files.get(&project.name) {
        Some(path) => format!(
            "- **{}** {}: `{}`\n",
            project.name, project.new_version, path
        ),
        None => format!("- **{}** {}\n", project.name, project.new_version),
    }
}

const OMITTED_HEADING: &str = "Not shown here, because GitHub caps a pull request description at 65 536 characters. \
Each entry is in the file named, in this pull request's diff, and becomes the release notes of its tag:\n\n";

const OMITTED_SUMMARY: &str =
    "The remaining changelogs are in each package's `CHANGELOG.md` in this \
pull request's diff.\n\n";

/// Adds changelogs in package order while they fit under `limit`, and
/// references the rest. Room for every reference still to be written is
/// held back before a changelog is admitted, so the references always fit.
fn push_changelogs(
    body: &mut String,
    projects: &[&SelectedReleaseUnit],
    changelog_contents: &HashMap<String, String>,
    changelog_files: &HashMap<String, String>,
    limit: usize,
) {
    let inline = projects.len() == 1;
    let references: Vec<String> = projects
        .iter()
        .map(|p| reference_line(p, changelog_files))
        .collect();

    let omitted_frame = OMITTED_HEADING.len() + 1;
    let mut pending_references: usize = references.iter().map(String::len).sum();
    let mut omitted: Vec<usize> = Vec::new();
    let mut omitted_len = 0;

    for (index, project) in projects.iter().enumerate() {
        pending_references -= references[index].len();
        let block = changelog_block(project, &changelog_contents[&project.name], inline);
        let held_back = omitted_frame + omitted_len + references[index].len() + pending_references;
        if body.len() + block.len() + held_back <= limit {
            body.push_str(&block);
        } else {
            omitted_len += references[index].len();
            omitted.push(index);
        }
    }

    if omitted.is_empty() {
        return;
    }

    if body.len() + omitted_frame + omitted_len <= limit {
        body.push_str(OMITTED_HEADING);
        for index in omitted {
            body.push_str(&references[index]);
        }
        body.push('\n');
    } else {
        body.push_str(OMITTED_SUMMARY);
    }
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
    use crate::core::wire::known::Ecosystem;

    fn make_project(name: &str, old: &str, new: &str, bump: &str) -> SelectedReleaseUnit {
        SelectedReleaseUnit {
            ident: 0,
            name: name.to_string(),
            prefix: String::new(),
            old_version: old.to_string(),
            new_version: new.to_string(),
            bump_type: bump.to_string(),
            is_prerelease: false,
            commits: vec![],
            ecosystem: Ecosystem::classify("cargo"),
            cached_changelog: None,
            cascade_inputs: Vec::new(),
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
        let body = generate_pr_body(
            &projects,
            "release-test.json",
            &changelog_contents,
            &HashMap::new(),
        );

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
        let body = generate_pr_body(
            &projects,
            "release-20250101-abc123.json",
            &HashMap::new(),
            &HashMap::new(),
        );

        assert!(body.contains("### 📋 Release Manifest"));
        assert!(body.contains("📄 `belaf/releases/release-20250101-abc123.json`"));
    }

    #[test]
    fn test_pr_body_contains_next_steps() {
        let projects = vec![make_project("test", "1.0.0", "1.0.1", "patch")];
        let body = generate_pr_body(&projects, "release.json", &HashMap::new(), &HashMap::new());

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
        let body = generate_pr_body(
            &projects,
            "release.json",
            &changelog_contents,
            &HashMap::new(),
        );

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
        let body = generate_pr_body(
            &projects,
            "release.json",
            &changelog_contents,
            &HashMap::new(),
        );

        assert!(body.contains("<details>"));
        assert!(body.contains("<summary><strong>core</strong>"));
        assert!(body.contains("<summary><strong>utils</strong>"));
        assert!(body.contains("</details>"));
    }

    fn changelog_of(len: usize) -> String {
        let line = "- **scope:** a commit subject long enough to look like a real one\n";
        line.repeat(len / line.len() + 1)
    }

    #[test]
    fn test_pr_body_stays_under_github_limit_for_a_big_week() {
        let projects: Vec<_> = (0..29)
            .map(|i| make_project(&format!("unit-{i:02}"), "0.1.0", "0.1.1", "patch"))
            .collect();
        let mut contents = HashMap::new();
        let mut files = HashMap::new();
        for p in &projects {
            contents.insert(p.name.clone(), changelog_of(5_000));
            files.insert(p.name.clone(), format!("apps/{}/CHANGELOG.md", p.name));
        }

        let body = generate_pr_body(&projects, "release.json", &contents, &files);

        assert!(body.len() <= PR_BODY_BUDGET, "body is {} bytes", body.len());
        assert!(
            body.contains("| **unit-28** |"),
            "the table lists every package"
        );
        assert!(body.contains("<summary><strong>unit-00</strong>"));
        assert!(body.contains("- **unit-28** 0.1.1: `apps/unit-28/CHANGELOG.md`"));
        assert!(!body.contains("<summary><strong>unit-28</strong>"));
        assert!(body.contains("belaf/releases/release.json"));
        assert!(body.contains("### ✅ Next Steps"));
        for p in &projects {
            let shown = body.contains(&format!("<summary><strong>{}</strong>", p.name));
            let referenced = body.contains(&format!("apps/{}/CHANGELOG.md", p.name));
            assert!(
                shown ^ referenced,
                "{} is either shown or referenced",
                p.name
            );
        }
    }

    #[test]
    fn test_pr_body_references_a_single_oversized_changelog() {
        let projects = vec![make_project("big", "1.0.0", "1.1.0", "minor")];
        let mut contents = HashMap::new();
        contents.insert("big".to_string(), changelog_of(200_000));
        let mut files = HashMap::new();
        files.insert("big".to_string(), "CHANGELOG.md".to_string());

        let body = generate_pr_body(&projects, "release.json", &contents, &files);

        assert!(body.len() <= PR_BODY_BUDGET);
        assert!(body.contains("- **big** 1.1.0: `CHANGELOG.md`"));
    }

    #[test]
    fn test_pr_body_small_release_has_no_reference_section() {
        let projects = vec![
            make_project("core", "1.0.0", "1.1.0", "minor"),
            make_project("utils", "2.0.0", "2.0.1", "patch"),
        ];
        let mut contents = HashMap::new();
        contents.insert("core".to_string(), "Core changes".to_string());
        contents.insert("utils".to_string(), "Utils fixes".to_string());

        let body = generate_pr_body(&projects, "release.json", &contents, &HashMap::new());

        assert!(!body.contains("Not shown here"));
    }

    #[test]
    fn test_pr_body_caps_the_table_for_thousands_of_packages() {
        let projects: Vec<_> = (0..3_000)
            .map(|i| {
                make_project(
                    &format!("package-with-a-long-name-{i:04}"),
                    "1.0.0",
                    "1.0.1",
                    "patch",
                )
            })
            .collect();

        let body = generate_pr_body(&projects, "release.json", &HashMap::new(), &HashMap::new());

        assert!(body.len() <= PR_BODY_BUDGET);
        assert!(body.contains("more packages, all listed in the release manifest"));
        assert!(body.contains("belaf/releases/release.json"));
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
