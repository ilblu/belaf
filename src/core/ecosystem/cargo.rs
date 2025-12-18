// Copyright 2020-2021 Peter Williams <peter@newton.cx> and collaborators
// Licensed under the MIT License.

//! Cargo (Rust) projects.
//!
//! If we detect a Cargo.toml in the repo root, we use `cargo metadata` to slurp
//! information about all of the crates and their interdependencies.

use anyhow::anyhow;
use cargo_metadata::MetadataCommand;
use std::{
    collections::HashMap,
    fs::File,
    io::Write,
    path::{Path, PathBuf},
};
use toml_edit::{DocumentMut, Item, Table};
use tracing::info;

use crate::core::release::{
    config::syntax::ProjectConfiguration,
    errors::Result,
    project::{DepRequirement, DependencyTarget, ProjectId},
    repository::{ChangeList, RepoPath, RepoPathBuf},
    rewriters::Rewriter,
    session::{AppBuilder, AppSession},
    version::Version,
};
use crate::utils::file_io::read_config_file;
use crate::utils::theme::PhaseSpinner;

/// Framework for auto-loading Cargo projects from the repository contents.
#[derive(Debug, Default)]
pub struct CargoLoader {
    cargo_toml_paths: Vec<RepoPathBuf>,
}

impl CargoLoader {
    /// Process items in the Git index while auto-loading projects.
    /// Collects ALL Cargo.toml files to detect multiple workspace roots.
    pub fn process_index_item(&mut self, dirname: &RepoPath, basename: &RepoPath) {
        if basename.as_ref() != b"Cargo.toml" {
            return;
        }

        let mut full_path = dirname.to_owned();
        full_path.push(basename);
        self.cargo_toml_paths.push(full_path);
    }

    /// Finalize autoloading any Cargo projects. Consumes this object.
    ///
    /// Discovers ALL workspace roots and loads each one separately.
    /// Uses two-phase loading: first register all projects, then resolve dependencies.
    pub fn finalize(
        self,
        app: &mut AppBuilder,
        pconfig: &HashMap<String, ProjectConfiguration>,
    ) -> Result<()> {
        if self.cargo_toml_paths.is_empty() {
            return Ok(());
        }

        let workspace_data = self.discover_workspaces_with_metadata(app)?;

        if workspace_data.is_empty() {
            info!("no Cargo workspace roots found in repository");
            return Ok(());
        }

        info!("found {} Cargo workspace root(s)", workspace_data.len());

        let mut all_cargo_to_graph = HashMap::new();
        let mut name_to_project: HashMap<String, ProjectId> = HashMap::new();

        for (workspace_root, cargo_meta) in &workspace_data {
            info!("loading Cargo workspace: {}", workspace_root.display());

            self.register_workspace_projects(
                app,
                pconfig,
                cargo_meta,
                workspace_root,
                &mut all_cargo_to_graph,
                &mut name_to_project,
            )?;
        }

        for (_, cargo_meta) in &workspace_data {
            self.resolve_workspace_dependencies(
                app,
                cargo_meta,
                &all_cargo_to_graph,
                &name_to_project,
            )?;
        }

        Ok(())
    }

    fn discover_workspaces_with_metadata(
        &self,
        app: &AppBuilder,
    ) -> Result<Vec<(PathBuf, cargo_metadata::Metadata)>> {
        let mut workspace_data = Vec::new();
        let mut seen_packages = std::collections::HashSet::new();

        for toml_repopath in &self.cargo_toml_paths {
            let toml_path = app.repo.resolve_workdir(toml_repopath);

            if !toml_path.exists() {
                continue;
            }

            let content = match read_config_file(&toml_path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let doc: DocumentMut = match content.parse() {
                Ok(d) => d,
                Err(_) => continue,
            };

            if doc.contains_key("workspace") {
                let mut cmd = MetadataCommand::new();
                cmd.manifest_path(&toml_path);
                cmd.features(cargo_metadata::CargoOpt::AllFeatures);

                let display_path = toml_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|name| {
                        toml_path
                            .parent()
                            .and_then(|p| p.file_name())
                            .and_then(|p| p.to_str())
                            .map(|parent| format!("{}/{}", parent, name))
                            .unwrap_or_else(|| name.to_string())
                    })
                    .unwrap_or_else(|| "Cargo.toml".to_string());

                let spinner = PhaseSpinner::new(format!("Loading {}", display_path));
                let result = cmd.exec();
                spinner.finish();

                if let Ok(meta) = result {
                    let mut has_new_packages = false;

                    for pkg in &meta.workspace_packages() {
                        let pkg_id = format!("{}:{}", pkg.name, pkg.version);
                        if seen_packages.insert(pkg_id) {
                            has_new_packages = true;
                        }
                    }

                    if has_new_packages {
                        workspace_data.push((toml_path.clone(), meta));
                    }
                }
            }
        }

        if workspace_data.is_empty() && !self.cargo_toml_paths.is_empty() {
            let first_toml = app.repo.resolve_workdir(&self.cargo_toml_paths[0]);
            let mut cmd = MetadataCommand::new();
            cmd.manifest_path(&first_toml);
            cmd.features(cargo_metadata::CargoOpt::AllFeatures);

            let display_path = first_toml
                .file_name()
                .and_then(|n| n.to_str())
                .map(|name| {
                    first_toml
                        .parent()
                        .and_then(|p| p.file_name())
                        .and_then(|p| p.to_str())
                        .map(|parent| format!("{}/{}", parent, name))
                        .unwrap_or_else(|| name.to_string())
                })
                .unwrap_or_else(|| "Cargo.toml".to_string());

            let spinner = PhaseSpinner::new(format!("Loading {}", display_path));
            let result = cmd.exec();
            spinner.finish();

            if let Ok(meta) = result {
                workspace_data.push((first_toml, meta));
            }
        }

        Ok(workspace_data)
    }

    fn is_workspace_project(&self, doc: &DocumentMut) -> bool {
        doc.get("workspace")
            .and_then(|ws| ws.as_table())
            .and_then(|ws_table| ws_table.get("package"))
            .and_then(|pkg| pkg.as_table())
            .and_then(|pkg_table| pkg_table.get("version"))
            .is_some()
    }

    fn register_workspace_projects(
        &self,
        app: &mut AppBuilder,
        pconfig: &HashMap<String, ProjectConfiguration>,
        cargo_meta: &cargo_metadata::Metadata,
        workspace_root: &Path,
        cargo_to_graph: &mut HashMap<cargo_metadata::PackageId, ProjectId>,
        name_to_project: &mut HashMap<String, ProjectId>,
    ) -> Result<()> {
        let content = read_config_file(workspace_root)?;
        let doc: DocumentMut = content.parse()?;
        let is_ws_project = self.is_workspace_project(&doc);

        if is_ws_project {
            info!(
                "workspace {} is a single project (has [workspace.package].version)",
                workspace_root.display()
            );

            let ws_name = doc
                .get("workspace")
                .and_then(|ws| ws.as_table())
                .and_then(|ws_table| ws_table.get("package"))
                .and_then(|pkg| pkg.as_table())
                .and_then(|pkg_table| pkg_table.get("name"))
                .and_then(|n| n.as_str())
                .map(|s| s.to_string())
                .or_else(|| {
                    workspace_root
                        .parent()
                        .and_then(|p| p.file_name())
                        .and_then(|n| n.to_str())
                        .map(|s| s.to_string())
                });

            let ws_version = doc
                .get("workspace")
                .and_then(|ws| ws.as_table())
                .and_then(|ws_table| ws_table.get("package"))
                .and_then(|pkg| pkg.as_table())
                .and_then(|pkg_table| pkg_table.get("version"))
                .and_then(|v| v.as_str())
                .and_then(|v| v.parse::<semver::Version>().ok());

            if let (Some(name), Some(version)) = (ws_name, ws_version) {
                let manifest_repopath = app.repo.convert_path(workspace_root)?;
                let (prefix, _) = manifest_repopath.split_basename();

                let qnames = vec![name.clone(), "cargo".to_owned()];

                if let Some(ident) = app.graph.try_add_project(qnames, pconfig) {
                    let proj = app.graph.lookup_mut(ident);

                    proj.version = Some(Version::Semver(version));
                    proj.prefix = Some(prefix.to_owned());

                    let workspace_member_ids: std::collections::HashSet<_> =
                        cargo_meta.workspace_members.iter().collect();

                    for pkg in &cargo_meta.packages {
                        if pkg.source.is_none() && workspace_member_ids.contains(&pkg.id) {
                            cargo_to_graph.insert(pkg.id.clone(), ident);
                            name_to_project.insert(pkg.name.to_string(), ident);
                        }
                    }

                    let cargo_rewrite = CargoRewriter::new(ident, manifest_repopath);
                    proj.rewriters.push(Box::new(cargo_rewrite));
                }
            }
        } else {
            info!(
                "workspace {} has separate projects for each member",
                workspace_root.display()
            );

            let workspace_member_ids: std::collections::HashSet<_> =
                cargo_meta.workspace_members.iter().collect();

            for pkg in &cargo_meta.packages {
                if pkg.source.is_some() {
                    continue;
                }

                if !workspace_member_ids.contains(&pkg.id) {
                    continue;
                }

                if cargo_to_graph.contains_key(&pkg.id) {
                    continue;
                }

                let manifest_repopath = app.repo.convert_path(&pkg.manifest_path)?;
                let (prefix, _) = manifest_repopath.split_basename();

                let qnames = vec![pkg.name.to_string(), "cargo".to_owned()];

                if let Some(ident) = app.graph.try_add_project(qnames, pconfig) {
                    let proj = app.graph.lookup_mut(ident);

                    proj.version = Some(Version::Semver(pkg.version.clone()));
                    proj.prefix = Some(prefix.to_owned());
                    cargo_to_graph.insert(pkg.id.clone(), ident);
                    name_to_project.insert(pkg.name.to_string(), ident);

                    let cargo_rewrite = CargoRewriter::new(ident, manifest_repopath);
                    proj.rewriters.push(Box::new(cargo_rewrite));
                }
            }
        }

        Ok(())
    }

    fn resolve_workspace_dependencies(
        &self,
        app: &mut AppBuilder,
        cargo_meta: &cargo_metadata::Metadata,
        cargo_to_graph: &HashMap<cargo_metadata::PackageId, ProjectId>,
        name_to_project: &HashMap<String, ProjectId>,
    ) -> Result<()> {
        let mut cargoid_to_index = HashMap::new();

        for (index, pkg) in cargo_meta.packages[..].iter().enumerate() {
            cargoid_to_index.insert(pkg.id.clone(), index);
        }

        let resolve = cargo_meta
            .resolve
            .as_ref()
            .ok_or_else(|| anyhow!("cargo metadata did not include dependency resolution"))?;

        let mut added_deps: std::collections::HashSet<(ProjectId, ProjectId)> =
            std::collections::HashSet::new();

        for node in &resolve.nodes {
            let pkg = &cargo_meta.packages[cargoid_to_index[&node.id]];

            if let Some(depender_id) = cargo_to_graph.get(&node.id) {
                let maybe_versions = pkg.metadata.get("internal_dep_versions");
                let manifest_repopath = app.repo.convert_path(&pkg.manifest_path)?;

                let dep_map: HashMap<_, _> = pkg
                    .dependencies
                    .iter()
                    .map(|cargo_dep| {
                        let name = cargo_dep.rename.as_ref().unwrap_or(&cargo_dep.name);
                        (name.clone(), cargo_dep.req.to_string())
                    })
                    .collect();

                for dep in &node.deps {
                    let normalized_name = dep.name.replace('_', "-");
                    let dependee_id = cargo_to_graph
                        .get(&dep.pkg)
                        .or_else(|| name_to_project.get(&dep.name))
                        .or_else(|| name_to_project.get(&normalized_name));

                    if let Some(dependee_id) = dependee_id {
                        if *dependee_id == *depender_id {
                            continue;
                        }

                        let dep_pair = (*depender_id, *dependee_id);
                        if !added_deps.insert(dep_pair) {
                            continue;
                        }

                        let literal = dep_map
                            .get(&dep.name)
                            .cloned()
                            .unwrap_or_else(|| "*".to_owned());

                        let req = maybe_versions
                            .and_then(|table| table.get(&dep.name))
                            .and_then(|nameval| nameval.as_str())
                            .map(|text| app.repo.parse_history_ref(text))
                            .transpose()?
                            .map(|cref| app.repo.resolve_history_ref(&cref, &manifest_repopath))
                            .transpose()?;

                        let req = req.unwrap_or(DepRequirement::Unavailable);

                        app.graph.add_dependency(
                            *depender_id,
                            DependencyTarget::Ident(*dependee_id),
                            literal,
                            req,
                        );
                    }
                }
            }
        }

        Ok(())
    }
}

/// Rewrite Cargo.toml to include real version numbers.
#[derive(Debug)]
pub struct CargoRewriter {
    proj_id: ProjectId,
    toml_path: RepoPathBuf,
}

impl CargoRewriter {
    /// Create a new Cargo.toml rewriter.
    pub fn new(proj_id: ProjectId, toml_path: RepoPathBuf) -> Self {
        CargoRewriter { proj_id, toml_path }
    }
}

impl Rewriter for CargoRewriter {
    fn rewrite(&self, app: &AppSession, changes: &mut ChangeList) -> Result<()> {
        // Parse the current Cargo.toml using toml_edit so we can rewrite it
        // with minimal deltas.
        let toml_path = app.repo.resolve_workdir(&self.toml_path);
        let s = read_config_file(&toml_path)?;
        let mut doc: DocumentMut = s.parse()?;

        // Helper table for applying internal deps. Note that we use the 0'th
        // qname, not the user-facing name, since that is what is used in
        // Cargo-land.

        let proj = app.graph().lookup(self.proj_id);
        let mut internal_reqs = HashMap::new();

        for dep in &proj.internal_deps[..] {
            let req_text = match dep.belaf_requirement {
                DepRequirement::Manual(ref t) => t.clone(),

                DepRequirement::Commit(_) => {
                    if let Some(ref v) = dep.resolved_version {
                        // Hack: For versions before 1.0, semver treats minor
                        // versions as incompatible: ^0.1 is not compatible with
                        // 0.2. This busts our paradigm. We can work around by
                        // using explicit greater-than expressions.
                        let v = v.to_string();
                        if v.starts_with("0.") {
                            format!(">={v},<1")
                        } else {
                            format!("^{v}")
                        }
                    } else {
                        continue;
                    }
                }

                DepRequirement::Unavailable => continue,
            };

            internal_reqs.insert(
                app.graph().lookup(dep.ident).qualified_names()[0].clone(),
                req_text,
            );
        }

        // Update the project version

        {
            let ct_root = doc.as_table_mut();
            let is_workspace =
                ct_root.contains_key("workspace") && !ct_root.contains_key("package");

            if is_workspace {
                let ws_pkg = ct_root
                    .get_mut("workspace")
                    .and_then(|ws| ws.as_table_mut())
                    .and_then(|ws| ws.get_mut("package"))
                    .and_then(|pkg| pkg.as_table_mut())
                    .ok_or_else(|| {
                        anyhow!(
                            "no [workspace.package] section in {}",
                            self.toml_path.escaped()
                        )
                    })?;
                ws_pkg["version"] = toml_edit::value(proj.version.to_string());
            } else {
                let pkg = ct_root
                    .get_mut("package")
                    .and_then(|i| i.as_table_mut())
                    .ok_or_else(|| {
                        anyhow!("no [package] section in {}", self.toml_path.escaped())
                    })?;
                pkg["version"] = toml_edit::value(proj.version.to_string());
            }

            // Rewrite any internal dependencies. These may be found in three
            // main tables and a nested table of potential target-specific
            // tables.

            for tblname in &["dependencies", "dev-dependencies", "build-dependencies"] {
                if let Some(tbl) = ct_root.get_mut(tblname).and_then(|i| i.as_table_mut()) {
                    rewrite_deptable(&internal_reqs, tbl)?;
                }
            }

            if let Some(ct_target) = ct_root.get_mut("target").and_then(|i| i.as_table_mut()) {
                // As far as I can tell, no way to iterate over the table while mutating
                // its values?
                let target_specs = ct_target
                    .iter()
                    .map(|(k, _v)| k.to_owned())
                    .collect::<Vec<_>>();

                for target_spec in &target_specs[..] {
                    if let Some(tbl) = ct_target
                        .get_mut(target_spec)
                        .and_then(|i| i.as_table_mut())
                    {
                        rewrite_deptable(&internal_reqs, tbl)?;
                    }
                }
            }
        }

        fn rewrite_deptable(
            internal_reqs: &HashMap<String, String>,
            tbl: &mut toml_edit::Table,
        ) -> Result<()> {
            let deps = tbl.iter().map(|(k, _v)| k.to_owned()).collect::<Vec<_>>();

            for dep in &deps[..] {
                // ??? renamed internal deps? We could save rename informaion
                // from cargo-metadata when we load everything.

                if let Some(req_text) = internal_reqs.get(dep) {
                    if let Some(dep_tbl) = tbl.get_mut(dep).and_then(|i| i.as_table_mut()) {
                        dep_tbl["version"] = toml_edit::value(req_text.clone());
                    } else if let Some(dep_tbl) =
                        tbl.get_mut(dep).and_then(|i| i.as_inline_table_mut())
                    {
                        // Can't just index inline tables???
                        if let Some(val) = dep_tbl.get_mut("version") {
                            *val = req_text.clone().into();
                        } else {
                            dep_tbl.get_or_insert("version", req_text.clone());
                        }
                    } else {
                        return Err(anyhow!(
                            "unexpected internal dependency item in a Cargo.toml: {:?}",
                            tbl.get(dep)
                        ));
                    }
                }
            }

            Ok(())
        }

        // Rewrite.

        {
            let mut f = File::create(&toml_path)?;
            write!(f, "{doc}")?;
            changes.add_path(&self.toml_path);
        }

        Ok(())
    }

    /// Rewriting just the special Belaf requirement metadata.
    fn rewrite_belaf_requirements(&self, app: &AppSession, changes: &mut ChangeList) -> Result<()> {
        // Short-circuit if no deps. Note that we can only do this if,
        // as done below, we don't clear unexpected entries in the
        // internal_dep_versions block. Should we do that?

        if app.graph().lookup(self.proj_id).internal_deps.is_empty() {
            return Ok(());
        }

        // Load

        let toml_path = app.repo.resolve_workdir(&self.toml_path);
        let s = read_config_file(&toml_path)?;
        let mut doc: DocumentMut = s.parse()?;

        // Modify.

        {
            let ct_root = doc.as_table_mut();
            let is_workspace =
                ct_root.contains_key("workspace") && !ct_root.contains_key("package");

            let metadata_parent = if is_workspace {
                ct_root
                    .get_mut("workspace")
                    .and_then(|ws| ws.as_table_mut())
                    .ok_or_else(|| {
                        anyhow!("no [workspace] section in {}", self.toml_path.escaped())
                    })?
            } else {
                ct_root
                    .get_mut("package")
                    .and_then(|i| i.as_table_mut())
                    .ok_or_else(|| {
                        anyhow!("no [package] section in {}", self.toml_path.escaped())
                    })?
            };

            let tbl = metadata_parent
                .entry("metadata")
                .or_insert_with(|| Item::Table(Table::new()))
                .as_table_mut()
                .ok_or_else(|| {
                    anyhow!(
                        "failed to create [metadata] section in {}",
                        self.toml_path.escaped()
                    )
                })?;

            let tbl = tbl
                .entry("internal_dep_versions")
                .or_insert_with(|| Item::Table(Table::new()))
                .as_table_mut()
                .ok_or_else(|| {
                    anyhow!(
                        "failed to create [metadata.internal_dep_versions] in {}",
                        self.toml_path.escaped()
                    )
                })?;

            let graph = app.graph();
            let proj = graph.lookup(self.proj_id);

            for dep in &proj.internal_deps {
                let target = &graph.lookup(dep.ident).qualified_names()[0];

                let spec = match &dep.belaf_requirement {
                    DepRequirement::Commit(cid) => cid.to_string(),
                    DepRequirement::Manual(t) => format!("manual:{t}"),
                    DepRequirement::Unavailable => continue,
                };

                tbl[target] = toml_edit::value(spec);
            }
        }

        // Rewrite.

        {
            let mut f = File::create(&toml_path)?;
            write!(f, "{doc}")?;
            changes.add_path(&self.toml_path);
        }

        Ok(())
    }
}
