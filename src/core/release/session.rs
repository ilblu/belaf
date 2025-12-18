// Copyright 2020 Peter Williams <peter@newton.cx> and collaborators
// Licensed under the MIT License.

//! State for the Belaf CLI application.

use anyhow::{anyhow, Context};
use std::collections::HashMap;
use thiserror::Error as ThisError;
use tracing::{error, info, warn};

use crate::{
    atry,
    core::release::{
        config::{syntax::ChangelogConfiguration, ConfigurationFile},
        errors::Result,
        graph::{ProjectGraph, ProjectGraphBuilder, RepoHistories},
        project::{DepRequirement, ProjectId},
        repository::{ChangeList, PathMatcher, ReleaseAvailability, Repository},
        version::Version,
    },
    utils::theme::ReleaseProgressBar,
};

#[derive(Clone, Debug, Default)]
pub struct NpmConfig {
    pub internal_dep_protocol: Option<String>,
}

/// Setting up a Belaf application session.
pub struct AppBuilder {
    pub repo: Repository,
    pub graph: ProjectGraphBuilder,

    is_ci: bool,
    populate_graph: bool,
    show_progress: bool,
}

fn detect_ci_environment() -> bool {
    std::env::var("CI").is_ok()
        || std::env::var("GITHUB_ACTIONS").is_ok()
        || std::env::var("GITLAB_CI").is_ok()
        || std::env::var("CIRCLECI").is_ok()
        || std::env::var("TRAVIS").is_ok()
        || std::env::var("JENKINS_URL").is_ok()
}

impl AppBuilder {
    /// Start initializing an application session.
    ///
    /// This first phase of initialization may fail if the environment doesn't
    /// associate the process with a proper Git repository with a work tree.
    pub fn new() -> Result<AppBuilder> {
        let repo = Repository::open_from_env()?;
        let graph = ProjectGraphBuilder::new();
        let is_ci = detect_ci_environment();

        Ok(AppBuilder {
            graph,
            repo,
            is_ci,
            populate_graph: true,
            show_progress: false,
        })
    }

    pub fn with_progress(mut self, show_progress: bool) -> Self {
        self.show_progress = show_progress;
        self
    }

    pub fn populate_graph(mut self, do_populate: bool) -> Self {
        self.populate_graph = do_populate;
        self
    }

    fn resolve_versions_from_tags(&mut self) -> Result<()> {
        let is_single_project = self.graph.project_count() == 1;

        for ident in self.graph.project_ids() {
            let proj = self.graph.lookup_mut(ident);

            let is_zero_version = matches!(
                &proj.version,
                Some(Version::Semver(v)) if v.major == 0
                    && v.minor == 0
                    && v.patch == 0
                    && v.pre.is_empty()
                    && v.build.is_empty()
            );

            if !is_zero_version {
                continue;
            }

            let Some(project_name) = proj.qnames.first().cloned() else {
                warn!(
                    "project at index {} has no qualified names, skipping version resolution",
                    ident
                );
                continue;
            };

            if let Some((_, tag_name)) = self
                .repo
                .find_latest_tag_for_project(&project_name, is_single_project)?
            {
                let version = Repository::parse_version_from_tag(&tag_name);
                if version.major != 0 || version.minor != 0 || version.patch != 0 {
                    info!(
                        "resolved version {} from tag '{}' for project '{}'",
                        version, tag_name, project_name
                    );
                    proj.version = Some(Version::Semver(version));
                }
            }
        }

        Ok(())
    }

    /// Finish app initialization, yielding a full AppSession object.
    pub fn initialize(mut self) -> Result<AppSession> {
        // Start by loading the configuration file, if it exists. If it doesn't
        // we'll get a sensible default.

        let mut cfg_path = self.repo.resolve_config_dir();
        cfg_path.push("config.toml");
        let config = ConfigurationFile::get(&cfg_path).with_context(|| {
            format!(
                "failed to load repository config file `{}`",
                cfg_path.display()
            )
        })?;

        self.repo
            .apply_config(config.repo)
            .with_context(|| "failed to finalize repository setup")?;

        let proj_config = config.projects;

        // Now auto-detect everything in the repo index.

        if self.populate_graph {
            let mut cargo = crate::core::ecosystem::cargo::CargoLoader::default();
            #[cfg(feature = "csharp")]
            let mut csproj = crate::core::ecosystem::csproj::CsProjLoader::default();
            let mut npm = crate::core::ecosystem::npm::NpmLoader::default();
            let mut pypa = crate::core::ecosystem::pypa::PypaLoader::default();
            let mut go = crate::core::ecosystem::go::GoLoader::default();
            let mut elixir = crate::core::ecosystem::elixir::ElixirLoader::default();
            let mut swift = crate::core::ecosystem::swift::SwiftLoader::default();

            // Dumb hack around the borrowchecker to allow mutable reference to
            // the graph while iterating over the repo:
            let repo = self.repo;
            let mut graph = self.graph;
            let show_progress = self.show_progress;

            if show_progress {
                let total = repo.index_entry_count().unwrap_or(0);
                let mut progress = ReleaseProgressBar::new(total, "Scanning repository");

                repo.scan_paths_with_progress(|p, current, _total| {
                    progress.update(current);
                    let (dirname, basename) = p.split_basename();
                    cargo.process_index_item(dirname, basename);
                    #[cfg(feature = "csharp")]
                    csproj.process_index_item(&repo, p, dirname, basename)?;
                    npm.process_index_item(&repo, &mut graph, p, dirname, basename, &proj_config)?;
                    pypa.process_index_item(dirname, basename);
                    go.process_index_item(dirname, basename);
                    elixir.process_index_item(dirname, basename);
                    swift.process_index_item(dirname, basename);
                    Ok(())
                })?;

                progress.finish();
            } else {
                repo.scan_paths(|p| {
                    let (dirname, basename) = p.split_basename();
                    cargo.process_index_item(dirname, basename);
                    #[cfg(feature = "csharp")]
                    csproj.process_index_item(&repo, p, dirname, basename)?;
                    npm.process_index_item(&repo, &mut graph, p, dirname, basename, &proj_config)?;
                    pypa.process_index_item(dirname, basename);
                    go.process_index_item(dirname, basename);
                    elixir.process_index_item(dirname, basename);
                    swift.process_index_item(dirname, basename);
                    Ok(())
                })?;
            }

            self.repo = repo;
            self.graph = graph;
            // End dumb hack.

            cargo.finalize(&mut self, &proj_config)?;
            #[cfg(feature = "csharp")]
            csproj.finalize(&mut self, &proj_config)?;
            npm.finalize(&mut self, &proj_config)?;
            pypa.finalize(&mut self, &proj_config)?;
            go.finalize(&mut self, &proj_config)?;
            elixir.finalize(&mut self, &proj_config)?;
            swift.finalize(&mut self, &proj_config)?;

            self.resolve_versions_from_tags()?;
        }

        // Apply project config and compile the graph.

        let graph = atry!(
            self.graph.complete_loading();
            ["the project graph is invalid"]
        );

        Ok(AppSession {
            repo: self.repo,
            graph,
            npm_config: NpmConfig::default(),
            changelog_config: config.changelog,
            is_ci: self.is_ci,
        })
    }
}

/// An error returned when one project in the repository needs a newer release
/// of another project. The inner values are the user-facing names of the two
/// projects: the first named project depends on the second one.
#[derive(Debug, ThisError)]
#[error("unsatisfied internal requirement: `{0}` needs newer `{1}`")]
pub struct UnsatisfiedInternalRequirementError(pub String, pub String);

pub struct AppSession {
    pub repo: Repository,
    pub npm_config: NpmConfig,
    pub changelog_config: ChangelogConfiguration,
    graph: ProjectGraph,
    is_ci: bool,
}

impl AppSession {
    /// Create a new app session with totally default parameters
    pub fn initialize_default() -> Result<Self> {
        AppBuilder::new()?.initialize()
    }

    pub fn execution_environment(&self) -> Result<ExecutionEnvironment> {
        if self.is_ci {
            Ok(ExecutionEnvironment::Ci)
        } else {
            Ok(ExecutionEnvironment::NotCi)
        }
    }

    /// Check that the current process is running *outside* of a CI environment.
    pub fn ensure_not_ci(&self, force: bool) -> Result<()> {
        match self.execution_environment()? {
            ExecutionEnvironment::NotCi => Ok(()),

            _ => {
                warn!("CI environment detected; this is unexpected for this command");
                if force {
                    Ok(())
                } else {
                    Err(anyhow!(
                        "refusing to proceed (use \"force\" mode to override)",
                    ))
                }
            }
        }
    }

    /// Check that the working tree is completely clean. We allow untracked and
    /// ignored files but otherwise don't want any modifications, etc. Returns
    /// Ok if clean, an Err downcastable to DirtyRepositoryError if not. The
    /// error may have a different cause if, e.g., there is an I/O failure.
    pub fn ensure_fully_clean(&self) -> Result<()> {
        use crate::core::release::repository::DirtyRepositoryError;

        if let Some(changed_path) = self.repo.check_if_dirty(&[])? {
            Err(DirtyRepositoryError(changed_path).into())
        } else {
            Ok(())
        }
    }

    /// Check that the working tree is clean, excepting modifications to any
    /// files interpreted as changelogs. Returns Ok if clean, an Err
    /// downcastable to DirtyRepositoryError if not. The error may have a
    /// different cause if, e.g., there is an I/O failure.
    pub fn ensure_changelog_clean(&self) -> Result<()> {
        use crate::core::release::repository::DirtyRepositoryError;

        let mut matchers: Vec<Result<PathMatcher>> = self
            .graph
            .projects()
            .map(|p| p.changelog.create_path_matcher(p))
            .collect();
        let matchers: Result<Vec<PathMatcher>> = matchers.drain(..).collect();
        let matchers = matchers?;

        if let Some(changed_path) = self.repo.check_if_dirty(&matchers[..])? {
            Err(DirtyRepositoryError(changed_path).into())
        } else {
            Ok(())
        }
    }

    /// Get the graph of projects inside this app session.
    pub fn graph(&self) -> &ProjectGraph {
        &self.graph
    }

    /// Get the graph of projects inside this app session, mutably.
    pub fn graph_mut(&mut self) -> &mut ProjectGraph {
        &mut self.graph
    }

    /// Walk the project graph and solve internal dependencies.
    ///
    /// This method walks the graph in topologically-sorted order. For each
    /// project, the callback `process` is called, which should return true if a
    /// new release of the project is being scheduled. By the time the callback
    /// is called, the project's internal dependency information will have been
    /// updated: for DepRequirement::Commit deps, `resolved_version` will be a
    /// Some value containing the required version. It is possible that this
    /// version will be being released "right now".
    ///
    /// By the time the callback returns, the project's `version` field should
    /// have been updated with its reference version for this release process --
    /// which should be a new value, if the callback returns true.
    ///
    /// After processing all projects, the function will return an error if
    /// there are unsatisfiable internal dependencies. This can happen either
    /// because no sufficiently new release of the dependee exists (and it's not
    /// being released now), or the internal version requirement information
    /// hasn't been annotated.
    pub fn solve_internal_deps<F>(&mut self, mut process: F) -> Result<()>
    where
        F: FnMut(&mut Repository, &mut ProjectGraph, ProjectId) -> Result<bool>,
    {
        let mut new_versions: HashMap<ProjectId, Version> = HashMap::new();
        let toposorted_idents: Vec<_> = self.graph.toposorted().collect();
        let mut unsatisfied_deps = Vec::new();

        for ident in (toposorted_idents[..]).iter().copied() {
            // We can't conveniently navigate the deps while holding a mutable
            // ref to depending project, so do some lifetime futzing and buffer
            // up modifications to its dep info.

            unsatisfied_deps.clear();

            let mut resolved_versions = {
                let proj = self.graph.lookup(ident);
                let mut resolved_versions = Vec::new();

                for (idx, dep) in proj.internal_deps.iter().enumerate() {
                    match dep.belaf_requirement {
                        // If the requirement is of a specific commit, we need
                        // to resolve its corresponding release and/or make sure
                        // that the dependee project is also being released in
                        // this batch.
                        DepRequirement::Commit(ref cid) => {
                            let dependee_proj = self.graph.lookup(dep.ident);
                            let is_single_project = self.graph.projects().count() == 1;
                            let avail = self.repo.find_earliest_release_containing(
                                dependee_proj,
                                cid,
                                is_single_project,
                            )?;

                            let resolved = match avail {
                                ReleaseAvailability::NotAvailable => {
                                    unsatisfied_deps
                                        .push(dependee_proj.user_facing_name.to_string());
                                    dependee_proj.version.clone()
                                }

                                ReleaseAvailability::ExistingRelease(ref v) => v.clone(),

                                ReleaseAvailability::NewRelease => {
                                    if let Some(v) = new_versions.get(&dep.ident) {
                                        v.clone()
                                    } else {
                                        unsatisfied_deps
                                            .push(dependee_proj.user_facing_name.to_string());
                                        dependee_proj.version.clone()
                                    }
                                }
                            };

                            resolved_versions.push((idx, resolved));
                        }

                        DepRequirement::Manual(_) => {}

                        DepRequirement::Unavailable => {
                            let dependee_proj = self.graph.lookup(dep.ident);
                            unsatisfied_deps.push(dependee_proj.user_facing_name.to_string());
                            resolved_versions.push((idx, dependee_proj.version.clone()));
                        }
                    }
                }

                resolved_versions
            };

            {
                let proj = self.graph.lookup_mut(ident);

                for (idx, resolved) in resolved_versions.drain(..) {
                    proj.internal_deps[idx].resolved_version = Some(resolved);
                }
            }

            // Now, let the callback do its thing with the project, and tell us
            // if it gets a new release.

            let updated_version = atry!(
                process(&mut self.repo, &mut self.graph, ident);
                ["failed to solve internal dependencies of project `{}`", self.graph.lookup(ident).user_facing_name]
            );

            let proj = self.graph.lookup(ident);

            if updated_version {
                if !unsatisfied_deps.is_empty() {
                    return Err(UnsatisfiedInternalRequirementError(
                        proj.user_facing_name.to_string(),
                        unsatisfied_deps.join(", "),
                    )
                    .into());
                }

                new_versions.insert(ident, proj.version.clone());
            } else if !unsatisfied_deps.is_empty() {
                warn!(
                    "project `{}` has internal requirements that won't be satisfiable in the wild, \
                     but that's OK since it's not going to be released",
                    proj.user_facing_name
                );
            }
        }

        Ok(())
    }

    /// A fake version of `solve_internal_deps`. Rather than properly expressing
    /// internal version requirements, this manually assigns each internal
    /// dependency to match exactly the version of the depended-upon package.
    /// This functionality is needed for Lerna, which otherwise isn't clever
    /// enough to correctly detect the internal dependency.
    pub fn fake_internal_deps(&mut self) {
        let toposorted_idents: Vec<_> = self.graph.toposorted().collect();

        for ident in (toposorted_idents[..]).iter().copied() {
            let mut resolved_versions = {
                let proj = self.graph.lookup(ident);
                let mut resolved_versions = Vec::new();

                for (idx, dep) in proj.internal_deps.iter().enumerate() {
                    let dependee_proj = self.graph.lookup(dep.ident);
                    resolved_versions.push((idx, dependee_proj.version.clone()));
                }

                resolved_versions
            };

            {
                let proj = self.graph.lookup_mut(ident);

                for (idx, resolved) in resolved_versions.drain(..) {
                    proj.internal_deps[idx].belaf_requirement =
                        DepRequirement::Manual(resolved.to_string());
                    proj.internal_deps[idx].resolved_version = Some(resolved);
                }
            }
        }
    }

    pub fn apply_versions(&mut self, bump_specs: &HashMap<String, String>) -> Result<()> {
        let histories = self.graph.analyze_histories(&self.repo)?;

        self.solve_internal_deps(|_repo, graph, ident| {
            let proj = graph.lookup_mut(ident);
            let history = histories.lookup(ident);

            if let Some(tag_version) = history.release_version() {
                proj.version = proj.version.parse_like(tag_version.to_string())?;
            }

            let baseline_version = proj.version.clone();

            Ok(
                if let Some(bump_spec) = bump_specs.get(&proj.user_facing_name) {
                    let scheme = proj.version.parse_bump_scheme(bump_spec)?;
                    scheme.apply(&mut proj.version)?;
                    info!(
                        "{}: {} => {}",
                        proj.user_facing_name, baseline_version, proj.version
                    );
                    true
                } else {
                    info!(
                        "{}: unchanged from {}",
                        proj.user_facing_name, baseline_version
                    );
                    false
                },
            )
        })
        .with_context(|| "failed to solve internal dependencies")?;

        Ok(())
    }

    /// Rewrite everyone's metadata to match our internal state.
    pub fn rewrite(&self) -> Result<ChangeList> {
        let mut changes = ChangeList::default();

        for ident in self.graph.toposorted() {
            let proj = self.graph.lookup(ident);

            for rw in &proj.rewriters {
                rw.rewrite(self, &mut changes)?;
            }
        }

        Ok(changes)
    }

    /// Like rewrite(), but only for the special Belaf requirements metadata.
    /// This is convenience functionality not needed for the main workflows.
    pub fn rewrite_belaf_requirements(&self) -> Result<ChangeList> {
        let mut changes = ChangeList::default();

        for ident in self.graph.toposorted() {
            let proj = self.graph.lookup(ident);

            for rw in &proj.rewriters {
                rw.rewrite_belaf_requirements(self, &mut changes)?;
            }
        }

        Ok(changes)
    }

    pub fn analyze_histories(&self) -> Result<RepoHistories> {
        self.graph.analyze_histories(&self.repo)
    }
}

pub enum ExecutionEnvironment {
    Ci,
    NotCi,
}
