use anyhow::anyhow;
use std::{
    fs::File,
    io::{BufRead, BufReader},
};

// Swift Package Manager does not store version in Package.swift.
// Versions are derived from git tags (e.g., v1.0.0), similar to Go modules.
// Therefore, no Rewriter implementation is needed for Swift packages.

use crate::utils::file_io::check_file_size;
use crate::{
    atry,
    core::{
        ecosystem::format_handler::{DiscoveredUnit, FormatHandler},
        errors::Result,
        git::repository::{ChangeList, RepoPath, RepoPathBuf, Repository},
        release_unit::VersionFieldSpec,
        resolved_release_unit::ReleaseUnitId,
        rewriters::Rewriter,
        session::AppSession,
        version::Version,
    },
};

#[derive(Debug, Default)]
pub struct SwiftLoader;

/// Swift Package Manager doesn't store project versions in
/// `Package.swift`; releases are tag-driven (similar to Go modules).
/// `make_rewriter` returns a no-op rewriter that the bump pipeline
/// will run; the actual release artefact is the git tag.
#[derive(Debug)]
pub struct SwiftNoOpRewriter;

impl Rewriter for SwiftNoOpRewriter {
    fn rewrite(&self, _app: &AppSession, _changes: &mut ChangeList) -> Result<()> {
        Ok(())
    }
}

impl FormatHandler for SwiftLoader {
    fn name(&self) -> &'static str {
        "swift"
    }

    fn display_name(&self) -> &'static str {
        "Swift"
    }

    fn is_manifest_file(&self, path: &RepoPath) -> bool {
        let (_, basename) = path.split_basename();
        basename.as_ref() == b"Package.swift"
    }

    fn parse_version(&self, _content: &str) -> Result<String> {
        // Swift releases are git-tag-driven.
        Ok("0.0.0".to_string())
    }

    fn default_version_field(&self) -> VersionFieldSpec {
        VersionFieldSpec::GenericRegex {
            pattern: r"//\s*belaf-version\s*=\s*(.+)".to_string(),
            replace: "// belaf-version = {version}".to_string(),
        }
    }

    fn make_rewriter(
        &self,
        _unit_id: ReleaseUnitId,
        _manifest_path: RepoPathBuf,
    ) -> Box<dyn Rewriter> {
        Box::new(SwiftNoOpRewriter)
    }

    fn discover_single(
        &self,
        repo: &Repository,
        manifest_path: &RepoPath,
    ) -> Result<DiscoveredUnit> {
        let fs_path = repo.resolve_workdir(manifest_path);
        let f = atry!(
            File::open(&fs_path);
            ["failed to open Package.swift file `{}`", fs_path.display()]
        );
        atry!(
            check_file_size(&f, &fs_path);
            ["file size check failed for `{}`", fs_path.display()]
        );
        let reader = BufReader::new(f);
        let mut package_name = None;
        for line_result in reader.lines() {
            let line = line_result?;
            if let Some(name) = extract_package_name(&line) {
                package_name = Some(name);
                break;
            }
        }
        let package_name = atry!(
            package_name.ok_or_else(|| anyhow!("no package name declaration found"));
            ["failed to parse package name from `{}`", fs_path.display()]
        );
        let (prefix, _) = manifest_path.split_basename();
        Ok(DiscoveredUnit {
            qnames: vec![package_name, "swift".to_owned()],
            version: Version::Semver(semver::Version::new(0, 0, 0)),
            prefix: prefix.to_owned(),
            anchor_manifest: manifest_path.to_owned(),
            rewriter_factories: vec![Box::new(|_id| Box::new(SwiftNoOpRewriter))],
            internal_deps: Vec::new(),
        })
    }
}

fn extract_package_name(line: &str) -> Option<String> {
    let trimmed = line.trim();

    if trimmed.starts_with("//") {
        return None;
    }

    if !trimmed.contains("name:") && !trimmed.contains("name :") {
        return None;
    }

    if let Some(name_start) = trimmed.find("name") {
        let after_name = &trimmed[name_start + 4..];
        let after_colon = after_name.trim_start().strip_prefix(':')?;
        let after_colon = after_colon.trim_start();

        let quote_char = if after_colon.starts_with('"') {
            '"'
        } else {
            return None;
        };

        let content_start = after_colon.strip_prefix(quote_char)?;
        let end_quote = content_start.find(quote_char)?;
        let name = &content_start[..end_quote];

        if !name.is_empty() {
            return Some(name.to_string());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_package_name_simple() {
        let line = r#"    name: "MyPackage","#;
        assert_eq!(extract_package_name(line), Some("MyPackage".to_string()));
    }

    #[test]
    fn test_extract_package_name_with_spaces() {
        let line = r#"        name:   "SwiftLibrary"  ,"#;
        assert_eq!(extract_package_name(line), Some("SwiftLibrary".to_string()));
    }

    #[test]
    fn test_extract_package_name_with_colon_space() {
        let line = r#"    name : "AnotherLib","#;
        assert_eq!(extract_package_name(line), Some("AnotherLib".to_string()));
    }

    #[test]
    fn test_extract_package_name_full_line() {
        let line = r#"let package = Package(name: "FullExample","#;
        assert_eq!(extract_package_name(line), Some("FullExample".to_string()));
    }

    #[test]
    fn test_extract_package_name_no_match() {
        let line = "import PackageDescription";
        assert_eq!(extract_package_name(line), None);
    }

    #[test]
    fn test_extract_package_name_comment_line() {
        let line = "// name: \"CommentedOut\"";
        assert_eq!(extract_package_name(line), None);
    }

    #[test]
    fn test_extract_package_name_empty_name() {
        let line = r#"    name: "","#;
        assert_eq!(extract_package_name(line), None);
    }
}
