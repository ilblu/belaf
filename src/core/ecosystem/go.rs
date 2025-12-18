use anyhow::anyhow;
use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader, Write},
};

use crate::utils::file_io::check_file_size;
use crate::{
    atry,
    core::release::{
        config::syntax::ProjectConfiguration,
        errors::Result,
        project::ProjectId,
        repository::{ChangeList, RepoPath, RepoPathBuf},
        rewriters::Rewriter,
        session::{AppBuilder, AppSession},
        version::Version,
    },
};

#[derive(Debug, Default)]
pub struct GoLoader {
    go_mod_paths: Vec<RepoPathBuf>,
}

impl GoLoader {
    pub fn process_index_item(&mut self, dirname: &RepoPath, basename: &RepoPath) {
        if basename.as_ref() != b"go.mod" {
            return;
        }

        let mut path = dirname.to_owned();
        path.push(basename);
        self.go_mod_paths.push(path);
    }

    pub fn finalize(
        self,
        app: &mut AppBuilder,
        pconfig: &HashMap<String, ProjectConfiguration>,
    ) -> Result<()> {
        for go_mod_path in self.go_mod_paths {
            let (prefix, _) = go_mod_path.split_basename();
            let fs_path = app.repo.resolve_workdir(&go_mod_path);

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

            let qnames = vec![module_name, "go".to_owned()];

            if let Some(ident) = app.graph.try_add_project(qnames, pconfig) {
                let proj = app.graph.lookup_mut(ident);
                proj.version = Some(Version::Semver(semver::Version::new(0, 0, 0)));
                proj.prefix = Some(prefix.to_owned());

                let go_rewrite = GoModRewriter::new(ident, go_mod_path);
                proj.rewriters.push(Box::new(go_rewrite));
            }
        }

        Ok(())
    }
}

#[derive(Debug)]
pub struct GoModRewriter {
    proj_id: ProjectId,
    repo_path: RepoPathBuf,
}

impl GoModRewriter {
    pub fn new(proj_id: ProjectId, repo_path: RepoPathBuf) -> Self {
        GoModRewriter { proj_id, repo_path }
    }
}

impl Rewriter for GoModRewriter {
    fn rewrite(&self, app: &AppSession, changes: &mut ChangeList) -> Result<()> {
        let fs_path = app.repo.resolve_workdir(&self.repo_path);
        let _proj = app.graph().lookup(self.proj_id);

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
    use super::*;

    #[test]
    fn test_process_index_item_detects_go_mod() {
        let mut loader = GoLoader::default();
        let dirname_buf = RepoPathBuf::new(b"backend");
        let basename_buf = RepoPathBuf::new(b"go.mod");

        loader.process_index_item(dirname_buf.as_ref(), basename_buf.as_ref());

        assert_eq!(loader.go_mod_paths.len(), 1);
        assert_eq!(
            <RepoPathBuf as AsRef<[u8]>>::as_ref(&loader.go_mod_paths[0]),
            b"backend/go.mod"
        );
    }

    #[test]
    fn test_process_index_item_ignores_other_files() {
        let mut loader = GoLoader::default();
        let dirname_buf = RepoPathBuf::new(b"backend");
        let basename_buf = RepoPathBuf::new(b"main.go");

        loader.process_index_item(dirname_buf.as_ref(), basename_buf.as_ref());

        assert_eq!(loader.go_mod_paths.len(), 0);
    }

    #[test]
    fn test_process_index_item_multiple_modules() {
        let mut loader = GoLoader::default();

        let backend_dir = RepoPathBuf::new(b"backend");
        let frontend_dir = RepoPathBuf::new(b"frontend");
        let api_dir = RepoPathBuf::new(b"api");
        let go_mod = RepoPathBuf::new(b"go.mod");

        loader.process_index_item(backend_dir.as_ref(), go_mod.as_ref());
        loader.process_index_item(frontend_dir.as_ref(), go_mod.as_ref());
        loader.process_index_item(api_dir.as_ref(), go_mod.as_ref());

        assert_eq!(loader.go_mod_paths.len(), 3);
    }

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
