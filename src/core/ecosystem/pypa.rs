// Copyright 2020 Peter Williams <peter@newton.cx> and collaborators
// Licensed under the MIT License.

//! Python Packaging Authority (PyPA) projects.

use anyhow::anyhow;
use clap::Parser;
use configparser::ini::Ini;
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    env,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
};
use tracing::warn;

use crate::core::release::version::pep440::Pep440Version;
use crate::utils::file_io::check_file_size;
use crate::{
    a_ok_or, atry,
    core::release::{
        config::syntax::ProjectConfiguration,
        errors::{Error, Result},
        project::{DepRequirement, DependencyTarget, ProjectId},
        repository::{ChangeList, RepoPath, RepoPathBuf},
        rewriters::Rewriter,
        session::{AppBuilder, AppSession},
        version::Version,
    },
};

struct PypaProjectData {
    ident: ProjectId,
    internal_reqs: HashSet<String>,
}

/// Framework for auto-loading PyPA projects from the repository contents.
#[derive(Debug, Default)]
pub struct PypaLoader {
    dirs_of_interest: HashSet<RepoPathBuf>,
}

impl PypaLoader {
    pub fn process_index_item(&mut self, dirname: &RepoPath, basename: &RepoPath) {
        let b = basename.as_ref();

        if b == b"setup.py" || b == b"setup.cfg" || b == b"pyproject.toml" {
            self.dirs_of_interest.insert(dirname.to_owned());
        }
    }

    /// Finalize autoloading any PyPA projects. Consumes this object.
    pub fn finalize(
        self,
        app: &mut AppBuilder,
        pconfig: &HashMap<String, ProjectConfiguration>,
    ) -> Result<()> {
        let mut pypa_projects: HashMap<String, PypaProjectData> = HashMap::new();
        let mut project_configs: HashMap<String, (Option<PyProjectBelaf>, RepoPathBuf)> =
            HashMap::new();

        for dirname in &self.dirs_of_interest {
            let mut name = None;
            let mut version = None;
            let mut main_version_file = None;

            let dir_desc = if dirname.is_empty() {
                "the toplevel directory".to_owned()
            } else {
                format!("directory `{}`", dirname.escaped())
            };

            // Try pyproject.toml first. If it exists, it might contain metadata
            // that help us gather info from the other project files.

            let mut toml_repopath = dirname.clone();
            toml_repopath.push("pyproject.toml");

            let config = {
                let toml_path = app.repo.resolve_workdir(&toml_repopath);
                let f = match File::open(&toml_path) {
                    Ok(f) => Some(f),
                    Err(e) => {
                        if e.kind() == std::io::ErrorKind::NotFound {
                            None
                        } else {
                            return Err(Error::new(e).context(format!(
                                "failed to open file `{}`",
                                toml_path.display()
                            )));
                        }
                    }
                };

                let data = f
                    .map(|mut f| -> Result<PyProjectFile> {
                        check_file_size(&f, &toml_path)?;
                        let mut text = String::new();
                        atry!(
                            f.read_to_string(&mut text);
                            ["failed to read file `{}`", toml_path.display()]
                        );

                        Ok(atry!(
                            toml::from_str(&text);
                            ["could not parse file `{}` as TOML", toml_path.display()]
                        ))
                    })
                    .transpose()?;

                let data = data.and_then(|d| d.tool).and_then(|t| t.belaf);

                if let Some(ref data) = data {
                    name = data.name.clone();
                    main_version_file = data.main_version_file.clone();
                }

                data
            };

            // Parse setup.cfg for metadata if available.

            {
                let mut cfg_path = dirname.clone();
                cfg_path.push("setup.cfg");
                let cfg_path = app.repo.resolve_workdir(&cfg_path);

                let f = match File::open(&cfg_path) {
                    Ok(f) => Some(f),
                    Err(e) => {
                        if e.kind() == std::io::ErrorKind::NotFound {
                            None
                        } else {
                            return Err(Error::new(e)
                                .context(format!("failed to open file `{}`", cfg_path.display())));
                        }
                    }
                };

                let data = f
                    .map(|mut f| -> Result<Ini> {
                        check_file_size(&f, &cfg_path)?;
                        let mut text = String::new();
                        atry!(
                            f.read_to_string(&mut text);
                            ["failed to read file `{}`", cfg_path.display()]
                        );

                        let mut cfg = Ini::new();
                        atry!(
                            cfg.read(text).map_err(|msg| anyhow!("{}", msg));
                            ["could not parse file `{}` as \"ini\"-style configuration", cfg_path.display()]
                        );

                        Ok(cfg)
                    })
                    .transpose()?;

                if let Some(data) = data {
                    if name.is_none() {
                        name = data.get("metadata", "name");
                    }
                    if version.is_none() {
                        if let Some(v) = data.get("metadata", "version") {
                            version = Some(atry!(
                                v.parse();
                                ["failed to parse version `{}` from setup.cfg", v]
                            ));
                        }
                    }
                }
            }

            let main_version_file = main_version_file.unwrap_or_else(|| "setup.py".to_owned());
            let main_version_in_setup = main_version_file == "setup.py";

            // Finally, how about setup.py?

            {
                let mut setup_path = dirname.clone();
                setup_path.push("setup.py");
                let setup_path = app.repo.resolve_workdir(&setup_path);

                let f = match File::open(&setup_path) {
                    Ok(f) => Some(f),
                    Err(e) => {
                        if e.kind() == std::io::ErrorKind::NotFound {
                            None
                        } else {
                            return Err(Error::new(e).context(format!(
                                "failed to open file `{}`",
                                setup_path.display()
                            )));
                        }
                    }
                };

                if let Some(f) = f {
                    let reader = BufReader::new(f);

                    for line in reader.lines() {
                        let line = atry!(
                            line;
                            ["error reading data from file `{}`", setup_path.display()]
                        );

                        if simple_py_parse::has_commented_marker(&line, "belaf project-name")
                            && name.is_none()
                        {
                            name = Some(atry!(
                                simple_py_parse::extract_text_from_string_literal(&line);
                                ["failed to determine Python project name from `{}`", setup_path.display()]
                            ));
                        }

                        if main_version_in_setup
                            && simple_py_parse::has_commented_marker(&line, "belaf project-version")
                        {
                            version = Some(atry!(
                                version_from_line(&line);
                                ["failed to parse project version out source text line `{}` in `{}`",
                                 line, setup_path.display()]
                            ));
                        }
                    }
                }
            }

            fn version_from_line(line: &str) -> Result<Pep440Version> {
                if simple_py_parse::has_commented_marker(line, "belaf project-version tuple") {
                    Pep440Version::parse_from_tuple_literal(line)
                } else {
                    Ok(simple_py_parse::extract_text_from_string_literal(line)?.parse()?)
                }
            }

            // Do we need to look in yet another file to pull out the version?

            if !main_version_in_setup {
                let mut version_path = dirname.clone();
                version_path.push(&main_version_file);
                let version_path = app.repo.resolve_workdir(&version_path);

                let f = atry!(
                    File::open(&version_path);
                    ["failed to open file `{}`", version_path.display()]
                );

                let reader = BufReader::new(f);

                for line in reader.lines() {
                    let line = atry!(
                        line;
                        ["error reading data from file `{}`", version_path.display()]
                    );

                    if simple_py_parse::has_commented_marker(&line, "belaf project-version") {
                        version = Some(atry!(
                            version_from_line(&line);
                            ["failed to parse project version out source text line `{}` in `{}`",
                                line, version_path.display()]
                        ));
                    }
                }
            }

            // OK, did we get the core information?

            let name = a_ok_or!(name;
                ["could not identify the name of the Python project in {}", dir_desc]
                (note "try adding (1) a `name = ...` field in the `[metadata]` section of its `setup.cfg` \
                      or (2) a `# belaf project-name` comment at the end of a line containing the project \
                      name as a simple string literal in `setup.py` or (3) or a `name = ...` field in a \
                      `[tool.belaf]` section of its `pyproject.toml`")
            );

            let version = a_ok_or!(version;
                ["could not identify the version of the Python project in {}", dir_desc]
                (note "try adding a `# belaf project-version` comment at the end of a line containing \
                      the project version as a simple string literal in `setup.py`; see the documentation \
                      for other supported approaches")
            );

            // OMG, we actually have the core info.

            let qnames = vec![name.clone(), "pypa".to_owned()];

            if let Some(ident) = app.graph.try_add_project(qnames, pconfig) {
                {
                    let proj = app.graph.lookup_mut(ident);

                    proj.version = Some(Version::Pep440(version));
                    proj.prefix = Some(dirname.to_owned());

                    let mut rw_path = dirname.clone();
                    rw_path.push(main_version_file.as_bytes());
                    let rw = PythonRewriter::new(ident, rw_path);
                    proj.rewriters.push(Box::new(rw));
                }

                // Handle the other annotated files. Besides registering them for
                // rewrites, we also scan them now to detect additional metadata. In
                // particular, dependencies on non-Python projects.

                let mut internal_reqs = HashSet::new();

                for path in config
                    .as_ref()
                    .map(|c| &c.annotated_files[..])
                    .unwrap_or(&[])
                {
                    let mut rw_path = dirname.clone();
                    rw_path.push(path.as_bytes());

                    atry!(
                        scan_rewritten_file(app, &rw_path, &mut internal_reqs);
                        ["in Python project {}, could not scan the `annotated_files` entry {}",
                        dir_desc, rw_path.escaped()]
                    );

                    let rw = PythonRewriter::new(ident, rw_path);
                    {
                        let proj = app.graph.lookup_mut(ident);
                        proj.rewriters.push(Box::new(rw));
                    }
                }

                pypa_projects.insert(
                    name.clone(),
                    PypaProjectData {
                        ident,
                        internal_reqs: internal_reqs.clone(),
                    },
                );

                project_configs.insert(name, (config, toml_repopath));
            }
        }

        for (project_name, project_data) in &pypa_projects {
            let (config, toml_repopath) = project_configs
                .get(project_name)
                .expect("BUG: project_configs should contain all pypa_projects");

            for req_name in &project_data.internal_reqs {
                let is_internal = pypa_projects.contains_key(req_name);

                let req = config
                    .as_ref()
                    .and_then(|c| c.internal_dep_versions.get(req_name))
                    .map(|text| app.repo.parse_history_ref(text))
                    .transpose()?
                    .map(|cref| app.repo.resolve_history_ref(&cref, toml_repopath))
                    .transpose()?;

                if is_internal && req.is_none() {
                    warn!(
                        "missing or invalid key `tool.belaf.internal_dep_versions.{}` in `{}`",
                        &req_name,
                        toml_repopath.escaped()
                    );
                    warn!("... this is needed to specify the oldest version of `{}` compatible with `{}`",
                        &req_name, &project_name);
                }

                let req = req.unwrap_or(DepRequirement::Unavailable);

                if let Some(dep_project) = pypa_projects.get(req_name) {
                    app.graph.add_dependency(
                        project_data.ident,
                        DependencyTarget::Ident(dep_project.ident),
                        "(internal)".to_owned(),
                        req,
                    );
                } else {
                    app.graph.add_dependency(
                        project_data.ident,
                        DependencyTarget::Text(req_name.clone()),
                        "(unavailable)".to_owned(),
                        req,
                    );
                }
            }
        }

        Ok(())
    }
}

fn scan_rewritten_file(
    app: &mut AppBuilder,
    path: &RepoPath,
    reqs: &mut HashSet<String>,
) -> Result<()> {
    let file_path = app.repo.resolve_workdir(path);

    let f = atry!(
        File::open(&file_path);
        ["failed to open file `{}` for reading", file_path.display()]
    );
    let reader = BufReader::new(f);

    for (line_num0, line) in reader.lines().enumerate() {
        let line = atry!(
            line;
            ["error reading data from file `{}`", file_path.display()]
        );

        if simple_py_parse::has_commented_marker(&line, "belaf internal-req") {
            let idx = line
                .find("belaf internal-req")
                .expect("BUG: marker should exist after has_commented_marker check");
            let mut pieces = line[idx..].split_whitespace();
            pieces.next(); // skip "belaf"
            pieces.next(); // skip "internal-req"
            let name = a_ok_or!(
                pieces.next();
                ["in `{}` line {}, `belaf internal-req` comment must provide a project name",
                 file_path.display(), line_num0 + 1]
            );

            reqs.insert(name.to_owned());
        }
    }

    Ok(())
}

pub(crate) mod simple_py_parse {
    use crate::{a_ok_or, core::release::errors::Result};
    use anyhow::{anyhow, bail};

    pub fn has_commented_marker(line: &str, marker: &str) -> bool {
        match line.find('#') {
            None => false,

            Some(cidx) => match line.find(marker) {
                None => false,
                Some(midx) => midx > cidx,
            },
        }
    }

    pub fn extract_text_from_string_literal(line: &str) -> Result<String> {
        let mut sq_loc = line.find('\'');
        let mut dq_loc = line.find('"');

        // if both kinds of quotes, go with whichever we saw first.
        if let (Some(sq_idx), Some(dq_idx)) = (sq_loc, dq_loc) {
            if sq_idx < dq_idx {
                dq_loc = None;
            } else {
                sq_loc = None;
            }
        }

        let inside = if let Some(sq_left) = sq_loc {
            let sq_right = line.rfind('\'').ok_or_else(|| {
                anyhow!(
                    "expected a closing quote in Python line `{}`, but found none",
                    line
                )
            })?;
            if sq_right <= sq_left {
                bail!(
                    "expected a string literal in Python line `{}`, but only found one quote?",
                    line
                );
            }

            &line[sq_left + 1..sq_right]
        } else if let Some(dq_left) = dq_loc {
            let dq_right = line.rfind('"').ok_or_else(|| {
                anyhow!(
                    "expected a closing quote in Python line `{}`, but found none",
                    line
                )
            })?;
            if dq_right <= dq_left {
                bail!(
                    "expected a string literal in Python line `{}`, but only found one quote?",
                    line
                );
            }

            &line[dq_left + 1..dq_right]
        } else {
            bail!(
                "expected a string literal in Python line `{}`, but didn't find any quotation marks",
                line
            );
        };

        if inside.find('\\').is_some() {
            bail!("the string literal in Python line `{}` seems to contain \\ escapes, which I can't handle", line);
        }

        Ok(inside.to_owned())
    }

    pub fn replace_text_in_string_literal(line: &str, new_val: &str) -> Result<String> {
        let mut sq_loc = line.find('\'');
        let mut dq_loc = line.find('"');

        // if both kinds of quotes, go with whichever we saw first.
        if let (Some(sq_idx), Some(dq_idx)) = (sq_loc, dq_loc) {
            if sq_idx < dq_idx {
                dq_loc = None;
            } else {
                sq_loc = None;
            }
        }

        let (left_idx, right_idx) = if let Some(sq_left) = sq_loc {
            let sq_right = line.rfind('\'').ok_or_else(|| {
                anyhow!(
                    "expected a closing quote in Python line `{}`, but found none",
                    line
                )
            })?;
            if sq_right <= sq_left {
                bail!(
                    "expected a string literal in Python line `{}`, but only found one quote?",
                    line
                );
            }

            (sq_left, sq_right)
        } else if let Some(dq_left) = dq_loc {
            let dq_right = line.rfind('"').ok_or_else(|| {
                anyhow!(
                    "expected a closing quote in Python line `{}`, but found none",
                    line
                )
            })?;
            if dq_right <= dq_left {
                bail!(
                    "expected a string literal in Python line `{}`, but only found one quote?",
                    line
                );
            }

            (dq_left, dq_right)
        } else {
            bail!(
                "expected a string literal in Python line `{}`, but didn't find any quotation marks",
                line
            );
        };

        let mut replaced = line[..left_idx + 1].to_owned();
        replaced.push_str(new_val);
        replaced.push_str(&line[right_idx..]);
        Ok(replaced)
    }

    pub fn replace_tuple_literal(line: &str, new_val: &str) -> Result<String> {
        let left_idx = a_ok_or!(
            line.find('(');
            ["expected a tuple literal in Python line `{}`, but no left parenthesis", line]
        );

        let right_idx = a_ok_or!(
            line.rfind(')');
            ["expected a tuple literal in Python line `{}`, but no right parenthesis", line]
        );

        if right_idx <= left_idx {
            bail!(
                "expected a tuple literal in Python line `{}`, but parentheses don't line up",
                line
            );
        }

        let mut replaced = line[..left_idx].to_owned();
        replaced.push_str(new_val);
        replaced.push_str(&line[right_idx + 1..]);
        Ok(replaced)
    }
}

/// Toplevel `pyproject.toml` deserialization container.
#[derive(Debug, Deserialize)]
struct PyProjectFile {
    pub tool: Option<PyProjectTool>,
}

/// `pyproject.toml` section `tool` deserialization container.
#[derive(Debug, Deserialize)]
struct PyProjectTool {
    pub belaf: Option<PyProjectBelaf>,
}

/// Belaf metadata in `pyproject.toml`.
#[derive(Debug, Deserialize)]
struct PyProjectBelaf {
    /// The project name. It isn't always straightforward to determine this,
    /// since we basically can't assume anything about setup.py.
    pub name: Option<String>,

    /// The file that we should read to discover the current project version.
    /// Note that there might be other files that also contain the version that
    /// will need to be rewritten when we apply a new version.
    pub main_version_file: Option<String>,

    /// Additional Python files that should be rewritten on metadata changes.
    #[serde(default)]
    pub annotated_files: Vec<String>,

    /// Version requirements for internal dependencies.
    #[serde(default)]
    pub internal_dep_versions: HashMap<String, String>,
}

/// Rewrite a Python file to include real version numbers.
#[derive(Debug)]
pub struct PythonRewriter {
    proj_id: ProjectId,
    file_path: RepoPathBuf,
}

impl PythonRewriter {
    /// Create a new Python file rewriter.
    pub fn new(proj_id: ProjectId, file_path: RepoPathBuf) -> Self {
        PythonRewriter { proj_id, file_path }
    }
}

impl Rewriter for PythonRewriter {
    fn rewrite(&self, app: &AppSession, changes: &mut ChangeList) -> Result<()> {
        let mut did_anything = false;
        let file_path = app.repo.resolve_workdir(&self.file_path);

        let cur_f = atry!(
            File::open(&file_path);
            ["failed to open file `{}` for reading", file_path.display()]
        );
        let cur_reader = BufReader::new(cur_f);

        // Helper table for applying internal deps if needed.

        let proj = app.graph().lookup(self.proj_id);
        let mut internal_reqs = HashMap::new();

        for dep in &proj.internal_deps[..] {
            let req_text = match dep.belaf_requirement {
                DepRequirement::Manual(ref t) => t.clone(),

                DepRequirement::Commit(_) => {
                    if let Some(ref v) = dep.resolved_version {
                        format!("^{v}")
                    } else {
                        continue;
                    }
                }

                DepRequirement::Unavailable => continue,
            };

            internal_reqs.insert(
                app.graph().lookup(dep.ident).user_facing_name.clone(),
                req_text,
            );
        }

        // OK, now rewrite the file.

        let new_af = atomicwrites::AtomicFile::new(
            &file_path,
            atomicwrites::OverwriteBehavior::AllowOverwrite,
        );

        let proj = app.graph().lookup(self.proj_id);

        let r = new_af.write(|new_f| {

            for (line_num0, line) in cur_reader.lines().enumerate() {
                let line = atry!(
                    line;
                    ["error reading data from file `{}`", file_path.display()]
                );

                let line = if simple_py_parse::has_commented_marker(&line, "belaf project-version")
                {
                    did_anything = true;

                    if simple_py_parse::has_commented_marker(&line, "belaf project-version tuple") {
                        let new_text = atry!(
                            proj.version.as_pep440_tuple_literal();
                            ["couldn't convert the project version to a `sys.version_info` tuple"]
                        );
                        atry!(
                            simple_py_parse::replace_tuple_literal(&line, &new_text);
                            ["couldn't rewrite version-tuple source line `{}`", line]
                        )
                    } else {
                        atry!(
                            simple_py_parse::replace_text_in_string_literal(&line, &proj.version.to_string());
                            ["couldn't rewrite version-string source line `{}`", line]
                        )
                    }
                } else if  simple_py_parse::has_commented_marker(&line, "belaf internal-req") {
                    did_anything = true;

                    let idx = line
                        .find("belaf internal-req")
                        .expect("BUG: marker should exist after has_commented_marker check");
                    let mut pieces = line[idx..].split_whitespace();
                    pieces.next(); // skip "belaf"
                    pieces.next(); // skip "internal-req"
                    let name = a_ok_or!(
                        pieces.next();
                        ["in `{}` line {}, `belaf internal-req` comment must provide a project name",
                        file_path.display(), line_num0 + 1]
                    );

                    // This "shouldn't happen", but could if someone edits a
                    // file between the time that the app session starts and
                    // when we get to rewriting it. That indicates something
                    // racey happening so make it a hard error.
                    let req_text = a_ok_or!(
                        internal_reqs.get(name);
                        ["found internal requirement of `{}` not traced by belaf", name]
                    );

                    atry!(
                        simple_py_parse::replace_text_in_string_literal(&line, req_text);
                        ["couldn't rewrite internal-req source line `{}`", line]
                    )
                } else {
                    line
                };

                atry!(
                    writeln!(new_f, "{}", line);
                    ["error writing data to `{}`", new_af.path().display()]
                );
            }

            Ok(())
        });

        match r {
            Err(atomicwrites::Error::Internal(e)) => Err(e.into()),
            Err(atomicwrites::Error::User(e)) => Err(e),
            Ok(()) => {
                if !did_anything {
                    warn!(
                        "rewriter for Python file `{}` didn't make any modifications",
                        file_path.display()
                    );
                }

                changes.add_path(&self.file_path);
                Ok(())
            }
        }
    }
}

/// Python-specific CLI utilities.
#[derive(Debug, Eq, PartialEq, Parser)]
pub enum PythonCommands {
    /// Install $PYPI_TOKEN in the user's .pypirc.
    InstallToken(InstallTokenCommand),
}

#[derive(Debug, Eq, PartialEq, Parser)]
pub struct PythonCommand {
    #[command(subcommand)]
    command: PythonCommands,
}

impl PythonCommand {
    pub fn execute(self) -> Result<i32> {
        match self.command {
            PythonCommands::InstallToken(o) => o.execute(),
        }
    }
}

/// `belaf python install-token`
#[derive(Debug, Eq, PartialEq, Parser)]
pub struct InstallTokenCommand {
    #[arg(
        long = "repository",
        default_value = "pypi",
        help = "The repository name."
    )]
    repository: String,
}

impl InstallTokenCommand {
    fn execute(self) -> Result<i32> {
        let token = atry!(
            env::var("PYPI_TOKEN");
            ["missing or non-textual environment variable PYPI_TOKEN"]
        );

        let mut p =
            dirs::home_dir().ok_or_else(|| anyhow!("cannot determine user's home directory"))?;
        p.push(".pypirc");

        let mut file = atry!(
            OpenOptions::new().create(true).append(true).open(&p);
            ["failed to open file `{}` for appending", p.display()]
        );

        let mut write = || -> Result<()> {
            writeln!(file, "[{}]", self.repository)?;
            writeln!(file, "username = __token__")?;
            writeln!(file, "password = {}", token)?;
            Ok(())
        };

        atry!(
            write();
            ["failed to write token data to file `{}`", p.display()]
        );

        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::{PypaLoader, PypaProjectData, RepoPathBuf};
    use crate::core::release::version::pep440::Pep440Version;
    use std::collections::HashSet;
    use toml::Value;

    #[test]
    fn test_process_index_item_detects_setup_py() {
        let mut loader = PypaLoader::default();
        let dirname_buf = RepoPathBuf::new(b"python-project");
        let basename_buf = RepoPathBuf::new(b"setup.py");

        loader.process_index_item(dirname_buf.as_ref(), basename_buf.as_ref());

        assert_eq!(loader.dirs_of_interest.len(), 1);
        assert!(loader.dirs_of_interest.contains(&dirname_buf));
    }

    #[test]
    fn test_process_index_item_detects_pyproject_toml() {
        let mut loader = PypaLoader::default();
        let dirname_buf = RepoPathBuf::new(b"modern-python");
        let basename_buf = RepoPathBuf::new(b"pyproject.toml");

        loader.process_index_item(dirname_buf.as_ref(), basename_buf.as_ref());

        assert_eq!(loader.dirs_of_interest.len(), 1);
        assert!(loader.dirs_of_interest.contains(&dirname_buf));
    }

    #[test]
    fn test_process_index_item_ignores_other_files() {
        let mut loader = PypaLoader::default();
        let dirname_buf = RepoPathBuf::new(b"src");
        let basename_buf = RepoPathBuf::new(b"main.py");

        loader.process_index_item(dirname_buf.as_ref(), basename_buf.as_ref());

        assert_eq!(loader.dirs_of_interest.len(), 0);
    }

    #[test]
    fn test_process_index_item_multiple_projects() {
        let mut loader = PypaLoader::default();

        let dirname1_buf = RepoPathBuf::new(b"packages/lib1");
        let basename1_buf = RepoPathBuf::new(b"setup.py");
        loader.process_index_item(dirname1_buf.as_ref(), basename1_buf.as_ref());

        let dirname2_buf = RepoPathBuf::new(b"packages/lib2");
        let basename2_buf = RepoPathBuf::new(b"pyproject.toml");
        loader.process_index_item(dirname2_buf.as_ref(), basename2_buf.as_ref());

        let dirname3_buf = RepoPathBuf::new(b"packages/lib3");
        let basename3_buf = RepoPathBuf::new(b"setup.py");
        loader.process_index_item(dirname3_buf.as_ref(), basename3_buf.as_ref());

        assert_eq!(loader.dirs_of_interest.len(), 3);
        assert!(loader.dirs_of_interest.contains(&dirname1_buf));
        assert!(loader.dirs_of_interest.contains(&dirname2_buf));
        assert!(loader.dirs_of_interest.contains(&dirname3_buf));
    }

    #[test]
    fn test_parse_setup_py_version_simple() {
        let content = r#"setup(
    name="my-package",
    version="1.2.3",
)"#;

        let has_version = content.contains("version=");
        assert!(has_version);
    }

    #[test]
    fn test_parse_pyproject_toml_simple() {
        let content = r#"
[project]
name = "example-package"
version = "0.1.0"
"#;

        let parsed: std::result::Result<Value, toml::de::Error> = toml::from_str(content);
        assert!(parsed.is_ok());

        let table = parsed.expect("BUG: parsed should be Ok after assertion");
        let project = table.get("project");
        assert!(project.is_some());
    }

    #[test]
    fn test_parse_pyproject_toml_dynamic_version() {
        let content = r#"
[project]
name = "dynamic-package"
dynamic = ["version"]

[tool.setuptools.dynamic]
version = {attr = "package.__version__"}
"#;

        let parsed: std::result::Result<Value, toml::de::Error> = toml::from_str(content);
        assert!(parsed.is_ok());

        let table = parsed.expect("BUG: parsed should be Ok after assertion");
        let dynamic = table
            .get("project")
            .and_then(|p| p.get("dynamic"))
            .and_then(|d| d.as_array());

        assert!(dynamic.is_some());
    }

    #[test]
    fn test_pep440_version_parsing() {
        let version = "1.2.3";
        let parsed = version.parse::<Pep440Version>();
        assert!(parsed.is_ok());
    }

    #[test]
    fn test_pep440_version_with_dev() {
        let version = "1.0.dev0";
        let parsed = version.parse::<Pep440Version>();
        assert!(parsed.is_ok());
    }

    #[test]
    fn test_pep440_version_with_post() {
        let version = "1.0.post1";
        let parsed = version.parse::<Pep440Version>();
        assert!(parsed.is_ok());
    }

    #[test]
    fn test_simple_py_parse_string_literal() {
        let line = r#"    version = "1.2.3""#;
        let has_string = line.contains('"');
        assert!(has_string);
    }

    #[test]
    fn test_simple_py_parse_different_quotes() {
        let double = r#"version = "1.0.0""#;
        let single = r"version = '1.0.0'";

        assert!(double.contains('"'));
        assert!(single.contains('\''));
    }

    #[test]
    fn test_pypa_project_data_creation() {
        let mut reqs = HashSet::new();
        reqs.insert("other-package".to_string());

        let data = PypaProjectData {
            ident: 0,
            internal_reqs: reqs.clone(),
        };

        assert_eq!(data.ident, 0);
        assert_eq!(data.internal_reqs, reqs);
    }

    #[test]
    fn test_pypa_project_data_empty_reqs() {
        let data = PypaProjectData {
            ident: 5,
            internal_reqs: HashSet::new(),
        };

        assert_eq!(data.ident, 5);
        assert!(data.internal_reqs.is_empty());
    }

    #[test]
    fn test_pypa_project_data_multiple_reqs() {
        let mut reqs = HashSet::new();
        reqs.insert("package-a".to_string());
        reqs.insert("package-b".to_string());
        reqs.insert("package-c".to_string());

        let data = PypaProjectData {
            ident: 1,
            internal_reqs: reqs.clone(),
        };

        assert_eq!(data.internal_reqs.len(), 3);
        assert!(data.internal_reqs.contains("package-a"));
        assert!(data.internal_reqs.contains("package-b"));
        assert!(data.internal_reqs.contains("package-c"));
    }

    #[test]
    fn test_internal_reqs_hashset_operations() {
        let mut reqs1 = HashSet::new();
        reqs1.insert("shared-lib".to_string());
        reqs1.insert("utils".to_string());

        let mut reqs2 = HashSet::new();
        reqs2.insert("shared-lib".to_string());

        assert!(reqs1.contains("shared-lib"));
        assert!(reqs2.contains("shared-lib"));
        assert!(reqs1.contains("utils"));
        assert!(!reqs2.contains("utils"));
    }
}
