use anyhow::anyhow;
use std::{
    fs::File,
    io::{Read, Write},
};
use tracing::warn;

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
pub struct ElixirLoader;

impl ElixirLoader {
    fn extract_app_name(contents: &str) -> Option<String> {
        for line in contents.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("app:") {
                if let Some(colon_pos) = trimmed.find(':') {
                    let app_part = trimmed[colon_pos + 1..].trim();
                    if let Some(app_part) = app_part.strip_prefix(':') {
                        let name = app_part.trim_end_matches(',').trim();
                        return Some(name.to_string());
                    }
                }
            }
        }
        None
    }

    fn extract_version(contents: &str) -> Option<String> {
        for line in contents.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("version:") {
                if let Some(colon_pos) = trimmed.find(':') {
                    let version_part = trimmed[colon_pos + 1..].trim();
                    if let Some(version_part) = version_part.strip_prefix('"') {
                        if let Some(end_quote) = version_part.find('"') {
                            return Some(version_part[..end_quote].to_string());
                        }
                    }
                }
            }
        }
        None
    }
}

impl FormatHandler for ElixirLoader {
    fn name(&self) -> &'static str {
        "elixir"
    }

    fn display_name(&self) -> &'static str {
        "Elixir"
    }

    fn is_manifest_file(&self, path: &RepoPath) -> bool {
        let (_, basename) = path.split_basename();
        basename.as_ref() == b"mix.exs"
    }

    fn parse_version(&self, content: &str) -> Result<String> {
        Self::extract_version(content).ok_or_else(|| anyhow!("no version field in mix.exs"))
    }

    fn default_version_field(&self) -> VersionFieldSpec {
        VersionFieldSpec::GenericRegex {
            pattern: r#"version:\s*"([^"]+)""#.to_string(),
            replace: r#"version: "{version}""#.to_string(),
        }
    }

    fn make_rewriter(
        &self,
        unit_id: ReleaseUnitId,
        manifest_path: RepoPathBuf,
    ) -> Box<dyn Rewriter> {
        Box::new(MixExsRewriter::new(unit_id, manifest_path))
    }

    fn discover_single(
        &self,
        repo: &Repository,
        manifest_path: &RepoPath,
    ) -> Result<DiscoveredUnit> {
        let fs_path = repo.resolve_workdir(manifest_path);
        let mut contents = String::new();
        let mut f = atry!(
            File::open(&fs_path);
            ["failed to open mix.exs file `{}`", fs_path.display()]
        );
        atry!(
            check_file_size(&f, &fs_path);
            ["file size check failed for `{}`", fs_path.display()]
        );
        atry!(
            f.read_to_string(&mut contents);
            ["failed to read mix.exs file `{}`", fs_path.display()]
        );

        let app_name = atry!(
            Self::extract_app_name(&contents)
                .ok_or_else(|| anyhow!("failed to extract app name from mix.exs"));
            ["failed to parse app name from `{}`", fs_path.display()]
        );
        let version_str = Self::extract_version(&contents).unwrap_or_else(|| {
            warn!(
                "failed to extract version from mix.exs `{}`, defaulting to 0.1.0",
                fs_path.display()
            );
            String::from("0.1.0")
        });
        let version = match semver::Version::parse(&version_str) {
            Ok(v) => Version::Semver(v),
            Err(_) => Version::Semver(semver::Version::new(0, 1, 0)),
        };

        let (prefix, _) = manifest_path.split_basename();
        let manifest = manifest_path.to_owned();
        let manifest_for_rw = manifest.clone();
        Ok(DiscoveredUnit {
            qnames: vec![app_name, "elixir".to_owned()],
            version,
            prefix: prefix.to_owned(),
            anchor_manifest: manifest,
            rewriter_factories: vec![Box::new(move |id| {
                Box::new(MixExsRewriter::new(id, manifest_for_rw))
            })],
            internal_deps: Vec::new(),
        })
    }
}

#[derive(Debug)]
pub struct MixExsRewriter {
    unit_id: ReleaseUnitId,
    repo_path: RepoPathBuf,
}

impl MixExsRewriter {
    pub fn new(unit_id: ReleaseUnitId, repo_path: RepoPathBuf) -> Self {
        MixExsRewriter { unit_id, repo_path }
    }
}

impl Rewriter for MixExsRewriter {
    fn rewrite(&self, app: &AppSession, changes: &mut ChangeList) -> Result<()> {
        let fs_path = app.repo.resolve_workdir(&self.repo_path);
        let unit = app.graph().lookup(self.unit_id);

        let mut contents = String::new();
        let mut f = atry!(
            File::open(&fs_path);
            ["failed to open mix.exs file `{}`", fs_path.display()]
        );

        atry!(
            f.read_to_string(&mut contents);
            ["failed to read mix.exs file `{}`", fs_path.display()]
        );

        drop(f);

        let new_version = unit.version.to_string();
        let mut new_contents = String::new();

        for line in contents.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("version:") {
                if let Some(indent) = line.find("version:") {
                    new_contents.push_str(&line[..indent]);
                    new_contents.push_str(&format!("version: \"{}\",\n", new_version));
                    continue;
                }
            }
            new_contents.push_str(line);
            new_contents.push('\n');
        }

        let new_af = atomicwrites::AtomicFile::new(
            &fs_path,
            atomicwrites::OverwriteBehavior::AllowOverwrite,
        );

        let r = new_af.write(|new_f| {
            new_f.write_all(new_contents.as_bytes())?;
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
    fn test_extract_app_name_simple() {
        let content = "defmodule MyApp.MixProject do\n  def project do\n    [\n      app: :my_app,\n      version: \"0.1.0\"\n    ]\n  end\nend";

        let result = ElixirLoader::extract_app_name(content);
        assert_eq!(result, Some("my_app".to_string()));
    }

    #[test]
    fn test_extract_app_name_with_whitespace() {
        let content = "  def project do\n    [\n      app:   :phoenix_app  ,\n      version: \"1.0.0\"\n    ]";

        let result = ElixirLoader::extract_app_name(content);
        assert_eq!(result, Some("phoenix_app".to_string()));
    }

    #[test]
    fn test_extract_app_name_not_found() {
        let content = "defmodule Test do\n  def hello, do: :world\nend";

        let result = ElixirLoader::extract_app_name(content);
        assert_eq!(result, None);
    }

    #[test]
    fn test_extract_version_simple() {
        let content = "defmodule MyApp.MixProject do\n  def project do\n    [\n      app: :my_app,\n      version: \"0.1.0\"\n    ]\n  end\nend";

        let result = ElixirLoader::extract_version(content);
        assert_eq!(result, Some("0.1.0".to_string()));
    }

    #[test]
    fn test_extract_version_with_whitespace() {
        let content = "    version:   \"1.2.3\"  ,";

        let result = ElixirLoader::extract_version(content);
        assert_eq!(result, Some("1.2.3".to_string()));
    }

    #[test]
    fn test_extract_version_semver_prerelease() {
        let content = "      version: \"2.0.0-rc.1\",";

        let result = ElixirLoader::extract_version(content);
        assert_eq!(result, Some("2.0.0-rc.1".to_string()));
    }

    #[test]
    fn test_extract_version_not_found() {
        let content = "defmodule Test do\n  def hello, do: :world\nend";

        let result = ElixirLoader::extract_version(content);
        assert_eq!(result, None);
    }
}
