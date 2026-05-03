use anyhow::anyhow;
use std::{
    fs::File,
    io::{BufRead, BufReader, Write},
};

use crate::utils::file_io::check_file_size;
use crate::{
    atry,
    core::{
        ecosystem::format_handler::{scan_index_for_filename, DiscoveredUnit, FormatHandler},
        errors::Result,
        git::repository::{ChangeList, RepoPathBuf, Repository},
        release_unit::VersionFieldSpec,
        resolved_release_unit::ReleaseUnitId,
        rewriters::Rewriter,
        session::AppSession,
        version::Version,
    },
};

#[derive(Debug, Default)]
pub struct GoLoader;

impl FormatHandler for GoLoader {
    fn name(&self) -> &'static str {
        "go"
    }

    fn display_name(&self) -> &'static str {
        "Go"
    }

    fn manifest_filename(&self) -> &'static str {
        "go.mod"
    }

    fn default_version_field(&self) -> VersionFieldSpec {
        // go.mod doesn't carry the project's release version; the
        // canonical "version" lives in the git tag (`v1.2.3`). We
        // expose a `GenericRegex` placeholder so the
        // `version_field::read/write` dispatch doesn't panic, but it
        // is not normally exercised — Go releases are tag-driven.
        VersionFieldSpec::GenericRegex {
            pattern: r"^module\s+(.+?)\s*$".to_string(),
            replace: "module {version}".to_string(),
        }
    }

    fn tag_format_default(&self) -> &'static str {
        "{module}/v{version}"
    }

    fn tag_template_vars(&self) -> &'static [&'static str] {
        &["name", "version", "ecosystem", "module"]
    }

    fn make_rewriter(
        &self,
        unit_id: ReleaseUnitId,
        manifest_path: RepoPathBuf,
    ) -> Box<dyn Rewriter> {
        Box::new(GoModRewriter::new(unit_id, manifest_path))
    }

    fn discover_units(
        &self,
        repo: &Repository,
        configured_skip_paths: &[RepoPathBuf],
    ) -> Result<Vec<DiscoveredUnit>> {
        let go_mod_paths = scan_index_for_filename(repo, "go.mod", configured_skip_paths)?;
        let mut units = Vec::new();

        for go_mod_path in go_mod_paths {
            let (prefix, _) = go_mod_path.split_basename();
            let fs_path = repo.resolve_workdir(&go_mod_path);

            let f = atry!(
                File::open(&fs_path);
                ["failed to open go.mod file `{}`", fs_path.display()]
            );
            atry!(
                check_file_size(&f, &fs_path);
                ["file size check failed for `{}`", fs_path.display()]
            );
            let reader = BufReader::new(f);
            let mut module_name = None;

            for line_result in reader.lines() {
                let line = line_result?;
                let trimmed = line.trim();

                if let Some(stripped) = trimmed.strip_prefix("module ") {
                    module_name = Some(stripped.trim().to_string());
                    break;
                }
            }

            let module_name = atry!(
                module_name.ok_or_else(|| anyhow!("no module declaration found"));
                ["failed to parse module name from `{}`", fs_path.display()]
            );

            let manifest = go_mod_path.clone();
            units.push(DiscoveredUnit {
                qnames: vec![module_name, "go".to_owned()],
                version: Version::Semver(semver::Version::new(0, 0, 0)),
                prefix: prefix.to_owned(),
                anchor_manifest: go_mod_path,
                rewriter_factories: vec![Box::new(move |id| {
                    Box::new(GoModRewriter::new(id, manifest))
                })],
                internal_deps: Vec::new(),
            });
        }

        Ok(units)
    }
}

#[derive(Debug)]
pub struct GoModRewriter {
    unit_id: ReleaseUnitId,
    repo_path: RepoPathBuf,
}

impl GoModRewriter {
    pub fn new(unit_id: ReleaseUnitId, repo_path: RepoPathBuf) -> Self {
        GoModRewriter { unit_id, repo_path }
    }
}

impl Rewriter for GoModRewriter {
    fn rewrite(&self, app: &AppSession, changes: &mut ChangeList) -> Result<()> {
        let fs_path = app.repo.resolve_workdir(&self.repo_path);
        let _proj = app.graph().lookup(self.unit_id);

        let f = atry!(
            File::open(&fs_path);
            ["failed to open go.mod file `{}`", fs_path.display()]
        );

        let reader = BufReader::new(f);
        let mut lines = Vec::new();

        for line_result in reader.lines() {
            lines.push(line_result?);
        }

        let new_af = atomicwrites::AtomicFile::new(
            &fs_path,
            atomicwrites::OverwriteBehavior::AllowOverwrite,
        );

        let r = new_af.write(|new_f| {
            for line in &lines {
                writeln!(new_f, "{}", line)?;
            }
            Ok(())
        });

        changes.add_path(&self.repo_path);

        match r {
            Err(atomicwrites::Error::Internal(e)) => Err(e.into()),
            Err(atomicwrites::Error::User(e)) => Err(e),
            Ok(()) => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_extract_module_name_simple() {
        let content = "module github.com/user/project\n\ngo 1.21\n";
        let lines: Vec<_> = content.lines().collect();

        let mut module_name = None;
        for line in lines {
            let trimmed = line.trim();
            if let Some(stripped) = trimmed.strip_prefix("module ") {
                module_name = Some(stripped.trim().to_string());
                break;
            }
        }

        assert_eq!(module_name, Some("github.com/user/project".to_string()));
    }

    #[test]
    fn test_extract_module_name_with_whitespace() {
        let content = "  module   github.com/org/repo  \n\ngo 1.20\n";
        let lines: Vec<_> = content.lines().collect();

        let mut module_name = None;
        for line in lines {
            let trimmed = line.trim();
            if let Some(stripped) = trimmed.strip_prefix("module ") {
                module_name = Some(stripped.trim().to_string());
                break;
            }
        }

        assert_eq!(module_name, Some("github.com/org/repo".to_string()));
    }

    #[test]
    fn test_extract_module_name_not_first_line() {
        let content = "// Comment\n\nmodule example.com/myproject\n\ngo 1.21\n";
        let lines: Vec<_> = content.lines().collect();

        let mut module_name = None;
        for line in lines {
            let trimmed = line.trim();
            if let Some(stripped) = trimmed.strip_prefix("module ") {
                module_name = Some(stripped.trim().to_string());
                break;
            }
        }

        assert_eq!(module_name, Some("example.com/myproject".to_string()));
    }
}
