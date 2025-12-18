use anyhow::anyhow;
use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader},
};

// Swift Package Manager does not store version in Package.swift.
// Versions are derived from git tags (e.g., v1.0.0), similar to Go modules.
// Therefore, no Rewriter implementation is needed for Swift packages.

use crate::utils::file_io::check_file_size;
use crate::{
    atry,
    core::release::{
        config::syntax::ProjectConfiguration,
        errors::Result,
        repository::{RepoPath, RepoPathBuf},
        session::AppBuilder,
        version::Version,
    },
};

#[derive(Debug, Default)]
pub struct SwiftLoader {
    package_swift_paths: Vec<RepoPathBuf>,
}

impl SwiftLoader {
    pub fn process_index_item(&mut self, dirname: &RepoPath, basename: &RepoPath) {
        if basename.as_ref() != b"Package.swift" {
            return;
        }

        let mut path = dirname.to_owned();
        path.push(basename);
        self.package_swift_paths.push(path);
    }

    pub fn finalize(
        self,
        app: &mut AppBuilder,
        pconfig: &HashMap<String, ProjectConfiguration>,
    ) -> Result<()> {
        for package_swift_path in self.package_swift_paths {
            let (prefix, _) = package_swift_path.split_basename();
            let fs_path = app.repo.resolve_workdir(&package_swift_path);

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

            let qnames = vec![package_name, "swift".to_owned()];

            if let Some(ident) = app.graph.try_add_project(qnames, pconfig) {
                let proj = app.graph.lookup_mut(ident);
                proj.version = Some(Version::Semver(semver::Version::new(0, 0, 0)));
                proj.prefix = Some(prefix.to_owned());
            }
        }

        Ok(())
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
    fn test_process_index_item_detects_package_swift() {
        let mut loader = SwiftLoader::default();
        let dirname_buf = RepoPathBuf::new(b"MyLibrary");
        let basename_buf = RepoPathBuf::new(b"Package.swift");

        loader.process_index_item(dirname_buf.as_ref(), basename_buf.as_ref());

        assert_eq!(loader.package_swift_paths.len(), 1);
        assert_eq!(
            <RepoPathBuf as AsRef<[u8]>>::as_ref(&loader.package_swift_paths[0]),
            b"MyLibrary/Package.swift"
        );
    }

    #[test]
    fn test_process_index_item_ignores_other_files() {
        let mut loader = SwiftLoader::default();
        let dirname_buf = RepoPathBuf::new(b"Sources");
        let basename_buf = RepoPathBuf::new(b"main.swift");

        loader.process_index_item(dirname_buf.as_ref(), basename_buf.as_ref());

        assert_eq!(loader.package_swift_paths.len(), 0);
    }

    #[test]
    fn test_process_index_item_multiple_packages() {
        let mut loader = SwiftLoader::default();

        let lib1_dir = RepoPathBuf::new(b"LibraryA");
        let lib2_dir = RepoPathBuf::new(b"LibraryB");
        let lib3_dir = RepoPathBuf::new(b"packages/LibraryC");
        let package_swift = RepoPathBuf::new(b"Package.swift");

        loader.process_index_item(lib1_dir.as_ref(), package_swift.as_ref());
        loader.process_index_item(lib2_dir.as_ref(), package_swift.as_ref());
        loader.process_index_item(lib3_dir.as_ref(), package_swift.as_ref());

        assert_eq!(loader.package_swift_paths.len(), 3);
    }

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
