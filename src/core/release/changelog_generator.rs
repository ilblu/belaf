use anyhow::{Context, Result};
use std::{collections::BTreeMap, fs, path::Path};
use time::OffsetDateTime;

use super::commit_analyzer::{CategorizedCommit, ChangelogCategory};

#[derive(Debug)]
pub struct ChangelogEntry {
    pub version: String,
    pub date: String,
    pub categories: BTreeMap<ChangelogCategory, Vec<String>>,
}

impl ChangelogEntry {
    pub fn new(version: String) -> Self {
        let now = OffsetDateTime::now_utc();
        let date = format!(
            "{:04}-{:02}-{:02}",
            now.year(),
            now.month() as u8,
            now.day()
        );

        Self {
            version,
            date,
            categories: BTreeMap::new(),
        }
    }

    pub fn add_commits(&mut self, commits: &[CategorizedCommit]) {
        for commit in commits {
            self.categories
                .entry(commit.category)
                .or_default()
                .push(commit.format_for_changelog());
        }
    }

    pub fn to_markdown(&self) -> String {
        let mut output = format!("## [{}] - {}\n\n", self.version, self.date);

        for (category, items) in &self.categories {
            if items.is_empty() {
                continue;
            }

            output.push_str(&format!("### {}\n\n", category.as_str()));
            for item in items {
                output.push_str(item);
                output.push('\n');
            }
            output.push('\n');
        }

        output
    }
}

pub fn parse_existing_changelog(path: &Path) -> Result<String> {
    if !path.exists() {
        return Ok(String::new());
    }

    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read changelog at {}", path.display()))?;

    let lines: Vec<&str> = content.lines().collect();
    let mut start_idx = 0;

    for (idx, line) in lines.iter().enumerate() {
        if line.starts_with("## [") || line.starts_with("## Unreleased") {
            start_idx = idx;
            break;
        }
    }

    if start_idx == 0 && !lines.is_empty() {
        return Ok(content);
    }

    Ok(lines[start_idx..].join("\n"))
}

pub fn generate_changelog(
    project_name: &str,
    new_entry: &ChangelogEntry,
    existing_content: &str,
) -> String {
    let mut output = String::new();

    output.push_str("# Changelog\n\n");
    output.push_str(&format!(
        "All notable changes to {} will be documented in this file.\n\n",
        project_name
    ));
    output.push_str(
        "The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),\n\
        and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).\n\n",
    );

    output.push_str(&new_entry.to_markdown());

    if !existing_content.is_empty() {
        if !existing_content.starts_with("##") {
            output.push_str("## Older Versions\n\n");
        }
        output.push_str(existing_content);
        if !existing_content.ends_with('\n') {
            output.push('\n');
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::release::commit_analyzer::ChangelogCategory;

    #[test]
    fn test_changelog_entry_format() {
        let mut entry = ChangelogEntry::new("1.0.0".to_string());
        entry.categories.insert(
            ChangelogCategory::Added,
            vec!["- New feature X".to_string()],
        );
        entry
            .categories
            .insert(ChangelogCategory::Fixed, vec!["- Bug fix Y".to_string()]);

        let markdown = entry.to_markdown();
        assert!(markdown.contains("## [1.0.0]"));
        assert!(markdown.contains("### Added"));
        assert!(markdown.contains("### Fixed"));
    }

    #[test]
    fn test_generate_changelog_with_new_entry() {
        let mut entry = ChangelogEntry::new("1.0.0".to_string());
        entry
            .categories
            .insert(ChangelogCategory::Added, vec!["- Feature".to_string()]);

        let existing = "## [0.5.0] - 2025-01-01\n\n### Fixed\n\n- Old fix\n";
        let result = generate_changelog("test-project", &entry, existing);

        assert!(result.contains("# Changelog"));
        assert!(result.contains("## [1.0.0]"));
        assert!(result.contains("## [0.5.0]"));
        assert!(result.starts_with("# Changelog\n\n"));
    }

    #[test]
    fn test_parse_existing_changelog_preserves_entries() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let changelog_content = r#"# Changelog

All notable changes to this project will be documented in this file.

## [1.0.0] - 2025-01-01

### Added

- New feature

### Fixed

- Bug fix

## [0.5.0] - 2024-12-01

### Changed

- Updated dependency
"#;
        std::fs::write(tmp.path(), changelog_content).unwrap();

        let result = parse_existing_changelog(tmp.path()).unwrap();

        assert!(result.contains("## [1.0.0]"));
        assert!(result.contains("## [0.5.0]"));
        assert!(result.contains("### Added"));
        assert!(result.contains("### Fixed"));
        assert!(result.contains("### Changed"));
    }

    #[test]
    fn test_parse_nonexistent_changelog() {
        let non_existent_path = std::path::Path::new("/tmp/does_not_exist_changelog.md");
        let result = parse_existing_changelog(non_existent_path).unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn test_changelog_entry_multiple_categories() {
        let mut entry = ChangelogEntry::new("2.0.0".to_string());

        entry.categories.insert(
            ChangelogCategory::Added,
            vec![
                "- New API endpoint".to_string(),
                "- New CLI flag".to_string(),
            ],
        );
        entry.categories.insert(
            ChangelogCategory::Changed,
            vec!["- Updated dependencies".to_string()],
        );
        entry.categories.insert(
            ChangelogCategory::Deprecated,
            vec!["- Old API method".to_string()],
        );

        let markdown = entry.to_markdown();

        assert!(markdown.contains("## [2.0.0]"));
        assert!(markdown.contains("### Added"));
        assert!(markdown.contains("### Changed"));
        assert!(markdown.contains("### Deprecated"));
        assert!(markdown.contains("- New API endpoint"));
        assert!(markdown.contains("- New CLI flag"));
        assert!(markdown.contains("- Updated dependencies"));
        assert!(markdown.contains("- Old API method"));
    }
}
