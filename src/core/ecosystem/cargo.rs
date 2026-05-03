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

use crate::core::{
    ecosystem::format_handler::{
        is_path_inside_any, scan_index_for_filename, DiscoveredUnit, FormatHandler, RawInternalDep,
    },
    errors::Result,
    git::repository::{ChangeList, RepoPathBuf, Repository},
    release_unit::VersionFieldSpec,
    resolved_release_unit::{DepRequirement, ReleaseUnitId},
    rewriters::Rewriter,
    session::AppSession,
    version::Version,
};
use crate::utils::file_io::read_config_file;
use crate::utils::theme::PhaseSpinner;

/// Stateless cargo `FormatHandler`. The struct exists only as a
/// trait-object handle for the registry; all per-scan state lives in
/// local variables inside `discover_units`.
#[derive(Debug, Default)]
pub struct CargoLoader;

impl CargoLoader {
    /// Direct-parse fallback (Bazel-friendly). Reads `[package].name`
    /// and `[package].version` from a Cargo.toml without invoking
    /// `cargo metadata`. Used when metadata fails on hermetic C deps
    /// or non-cargo-managed workspaces.
    ///
    /// Skips: virtual workspace roots (no `[package]` section),
    /// non-semver versions, manifests with `version.workspace = true`
    /// where the workspace root has no `[workspace.package].version`.
    /// No inter-project dependency resolution — that's the trade-off
    /// callers accept for Bazel-coexistence.
    fn direct_parse_unit(
        &self,
        repo: &Repository,
        toml_repopath: &RepoPathBuf,
    ) -> Result<Option<DiscoveredUnit>> {
        let toml_abs = repo.resolve_workdir(toml_repopath);
        if !toml_abs.exists() {
            return Ok(None);
        }

        let Ok(content) = read_config_file(&toml_abs) else {
            return Ok(None);
        };
        let Ok(doc) = content.parse::<DocumentMut>() else {
            return Ok(None);
        };

        let Some(pkg) = doc.get("package").and_then(|v| v.as_table()) else {
            return Ok(None);
        };

        let name = pkg
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if name.is_empty() {
            return Ok(None);
        }

        let version_str = pkg.get("version").and_then(|v| v.as_str());
        let Some(version) = version_str.and_then(|s| s.parse::<semver::Version>().ok()) else {
            return Ok(None);
        };

        let (prefix, _) = toml_repopath.split_basename();
        let manifest = toml_repopath.clone();
        Ok(Some(DiscoveredUnit {
            qnames: vec![name, "cargo".to_owned()],
            version: Version::Semver(version),
            prefix: prefix.to_owned(),
            anchor_manifest: manifest.clone(),
            rewriter_factories: vec![Box::new(move |id| {
                Box::new(CargoRewriter::new(id, manifest))
            })],
            internal_deps: Vec::new(),
        }))
    }

    fn discover_workspaces_with_metadata(
        &self,
        repo: &Repository,
        cargo_toml_paths: &[RepoPathBuf],
    ) -> Result<Vec<(PathBuf, cargo_metadata::Metadata)>> {
        let mut workspace_data = Vec::new();
        let mut seen_packages = std::collections::HashSet::new();

        for toml_repopath in cargo_toml_paths {
            let toml_path = repo.resolve_workdir(toml_repopath);

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

        if workspace_data.is_empty() && !cargo_toml_paths.is_empty() {
            let first_toml = repo.resolve_workdir(&cargo_toml_paths[0]);
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

    /// Build `DiscoveredUnit`s for one cargo workspace.
    ///
    /// Returns a map (cargo PackageId → index in the returned Vec) so
    /// the caller's dependency-resolution pass can wire `internal_deps`
    /// without re-walking metadata.
    fn units_from_workspace(
        &self,
        repo: &Repository,
        cargo_meta: &cargo_metadata::Metadata,
        workspace_root: &Path,
        skip_list: &[RepoPathBuf],
    ) -> Result<(
        Vec<DiscoveredUnit>,
        HashMap<cargo_metadata::PackageId, usize>,
    )> {
        let content = read_config_file(workspace_root)?;
        let doc: DocumentMut = content.parse()?;
        let is_ws_project = self.is_workspace_project(&doc);

        let mut units: Vec<DiscoveredUnit> = Vec::new();
        let mut pkgid_to_index: HashMap<cargo_metadata::PackageId, usize> = HashMap::new();

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
                let manifest_repopath = repo.convert_path(workspace_root)?;
                let (prefix, _) = manifest_repopath.split_basename();

                let unit_index = units.len();
                let workspace_member_ids: std::collections::HashSet<_> =
                    cargo_meta.workspace_members.iter().collect();
                for pkg in &cargo_meta.packages {
                    if pkg.source.is_none() && workspace_member_ids.contains(&pkg.id) {
                        pkgid_to_index.insert(pkg.id.clone(), unit_index);
                    }
                }

                let manifest = manifest_repopath.clone();
                units.push(DiscoveredUnit {
                    qnames: vec![name, "cargo".to_owned()],
                    version: Version::Semver(version),
                    prefix: prefix.to_owned(),
                    anchor_manifest: manifest_repopath,
                    rewriter_factories: vec![Box::new(move |id| {
                        Box::new(CargoRewriter::new(id, manifest))
                    })],
                    internal_deps: Vec::new(),
                });
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

                if pkgid_to_index.contains_key(&pkg.id) {
                    continue;
                }

                let manifest_repopath = repo.convert_path(&pkg.manifest_path)?;

                // If this member's manifest is inside any ReleaseUnit's
                // skip-list path, the ReleaseUnit owns it (as satellite
                // or primary manifest). Skip standalone registration so
                // the unit's atomic claim on the directory is respected.
                if is_path_inside_any(&manifest_repopath, skip_list) {
                    continue;
                }

                let (prefix, _) = manifest_repopath.split_basename();

                let unit_index = units.len();
                pkgid_to_index.insert(pkg.id.clone(), unit_index);

                let manifest = manifest_repopath.clone();
                units.push(DiscoveredUnit {
                    qnames: vec![pkg.name.to_string(), "cargo".to_owned()],
                    version: Version::Semver(pkg.version.clone()),
                    prefix: prefix.to_owned(),
                    anchor_manifest: manifest_repopath,
                    rewriter_factories: vec![Box::new(move |id| {
                        Box::new(CargoRewriter::new(id, manifest))
                    })],
                    internal_deps: Vec::new(),
                });
            }
        }

        Ok((units, pkgid_to_index))
    }

    /// Wire `internal_deps` on every workspace member from cargo's
    /// resolve graph. Mutates `units` in place using `pkgid_to_index`
    /// to find the right unit per package.
    fn fill_internal_deps(
        &self,
        repo: &Repository,
        cargo_meta: &cargo_metadata::Metadata,
        units: &mut [DiscoveredUnit],
        pkgid_to_index: &HashMap<cargo_metadata::PackageId, usize>,
    ) -> Result<()> {
        let mut cargoid_to_pkgindex = HashMap::new();
        for (index, pkg) in cargo_meta.packages[..].iter().enumerate() {
            cargoid_to_pkgindex.insert(pkg.id.clone(), index);
        }

        let resolve = cargo_meta
            .resolve
            .as_ref()
            .ok_or_else(|| anyhow!("cargo metadata did not include dependency resolution"))?;

        let mut added_pairs: std::collections::HashSet<(usize, String)> =
            std::collections::HashSet::new();

        for node in &resolve.nodes {
            let pkg = &cargo_meta.packages[cargoid_to_pkgindex[&node.id]];

            let Some(depender_idx) = pkgid_to_index.get(&node.id).copied() else {
                continue;
            };
            let maybe_versions = pkg.metadata.get("internal_dep_versions");
            let manifest_repopath = repo.convert_path(&pkg.manifest_path)?;

            let dep_map: HashMap<_, _> = pkg
                .dependencies
                .iter()
                .map(|cargo_dep| {
                    let name = cargo_dep.rename.as_ref().unwrap_or(&cargo_dep.name);
                    (name.clone(), cargo_dep.req.to_string())
                })
                .collect();

            for dep in &node.deps {
                let Some(dependee_idx) = pkgid_to_index.get(&dep.pkg).copied() else {
                    continue;
                };
                if dependee_idx == depender_idx {
                    continue;
                }

                let target_name = units[dependee_idx].qnames[0].clone();
                if !added_pairs.insert((depender_idx, target_name.clone())) {
                    continue;
                }

                let literal = dep_map
                    .get(&dep.name)
                    .cloned()
                    .unwrap_or_else(|| "*".to_owned());

                let req = maybe_versions
                    .and_then(|table| table.get(&dep.name))
                    .and_then(|nameval| nameval.as_str())
                    .map(|text| repo.parse_history_ref(text))
                    .transpose()?
                    .map(|cref| repo.resolve_history_ref(&cref, &manifest_repopath))
                    .transpose()?;

                let req = req.unwrap_or(DepRequirement::Unavailable);

                units[depender_idx].internal_deps.push(RawInternalDep {
                    target_package_name: target_name,
                    literal,
                    requirement: req,
                });
            }
        }

        Ok(())
    }
}

impl FormatHandler for CargoLoader {
    fn name(&self) -> &'static str {
        "cargo"
    }

    fn display_name(&self) -> &'static str {
        "Rust (Cargo)"
    }

    fn manifest_filename(&self) -> &'static str {
        "Cargo.toml"
    }

    fn default_version_field(&self) -> VersionFieldSpec {
        VersionFieldSpec::CargoToml
    }

    fn make_rewriter(
        &self,
        unit_id: ReleaseUnitId,
        manifest_path: RepoPathBuf,
    ) -> Box<dyn Rewriter> {
        Box::new(CargoRewriter::new(unit_id, manifest_path))
    }

    fn discover_units(
        &self,
        repo: &Repository,
        configured_skip_paths: &[RepoPathBuf],
    ) -> Result<Vec<DiscoveredUnit>> {
        let cargo_toml_paths = scan_index_for_filename(repo, "Cargo.toml", configured_skip_paths)?;
        if cargo_toml_paths.is_empty() {
            return Ok(Vec::new());
        }

        let workspace_data = self.discover_workspaces_with_metadata(repo, &cargo_toml_paths)?;

        if workspace_data.is_empty() {
            info!("no Cargo workspace roots found in repository");
            // Bazel fallback: walk every Cargo.toml that wasn't covered.
            return self.bazel_fallback_units(repo, &cargo_toml_paths, configured_skip_paths, &[]);
        }

        info!("found {} Cargo workspace root(s)", workspace_data.len());

        let mut all_units: Vec<DiscoveredUnit> = Vec::new();
        let mut all_pkgid_to_index: HashMap<cargo_metadata::PackageId, usize> = HashMap::new();

        for (workspace_root, cargo_meta) in &workspace_data {
            info!("loading Cargo workspace: {}", workspace_root.display());
            let (units, pkgid_to_index) =
                self.units_from_workspace(repo, cargo_meta, workspace_root, configured_skip_paths)?;
            let offset = all_units.len();
            all_units.extend(units);
            for (pkg_id, idx) in pkgid_to_index {
                all_pkgid_to_index.insert(pkg_id, offset + idx);
            }
        }

        for (_, cargo_meta) in &workspace_data {
            self.fill_internal_deps(repo, cargo_meta, &mut all_units, &all_pkgid_to_index)?;
        }

        // Bazel fallback for any Cargo.toml not covered by metadata.
        let metadata_covered_manifests: std::collections::HashSet<PathBuf> = workspace_data
            .iter()
            .flat_map(|(_, meta)| {
                meta.workspace_packages()
                    .into_iter()
                    .map(|p| p.manifest_path.clone().into_std_path_buf())
            })
            .collect();
        let fallback_units = self.bazel_fallback_units(
            repo,
            &cargo_toml_paths,
            configured_skip_paths,
            &metadata_covered_manifests.into_iter().collect::<Vec<_>>(),
        )?;
        all_units.extend(fallback_units);

        Ok(all_units)
    }
}

impl CargoLoader {
    /// Direct-parse every Cargo.toml not covered by `cargo metadata`.
    /// Inputs are pre-filtered against the skip-list.
    fn bazel_fallback_units(
        &self,
        repo: &Repository,
        cargo_toml_paths: &[RepoPathBuf],
        skip_list: &[RepoPathBuf],
        metadata_covered: &[PathBuf],
    ) -> Result<Vec<DiscoveredUnit>> {
        let covered_set: std::collections::HashSet<&PathBuf> = metadata_covered.iter().collect();
        let mut out = Vec::new();
        for toml_repopath in cargo_toml_paths {
            if is_path_inside_any(toml_repopath, skip_list) {
                continue;
            }
            let toml_abs = repo.resolve_workdir(toml_repopath);
            if covered_set.contains(&toml_abs) {
                continue;
            }
            if let Some(unit) = self.direct_parse_unit(repo, toml_repopath)? {
                out.push(unit);
            }
        }
        Ok(out)
    }
}

/// Rewrite Cargo.toml to include real version numbers.
#[derive(Debug)]
pub struct CargoRewriter {
    unit_id: ReleaseUnitId,
    toml_path: RepoPathBuf,
}

impl CargoRewriter {
    /// Create a new Cargo.toml rewriter.
    pub fn new(unit_id: ReleaseUnitId, toml_path: RepoPathBuf) -> Self {
        CargoRewriter { unit_id, toml_path }
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

        let unit = app.graph().lookup(self.unit_id);
        let mut internal_reqs = HashMap::new();

        for dep in &unit.internal_deps[..] {
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
                ws_pkg["version"] = toml_edit::value(unit.version.to_string());
            } else {
                let pkg = ct_root
                    .get_mut("package")
                    .and_then(|i| i.as_table_mut())
                    .ok_or_else(|| {
                        anyhow!("no [package] section in {}", self.toml_path.escaped())
                    })?;
                pkg["version"] = toml_edit::value(unit.version.to_string());
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

        // Phase J — refresh Cargo.lock so the bumped version
        // propagates to the lockfile in the same prepare run. The
        // `update_for_crate` helper runs `cargo update -p <name>
        // --workspace`, falls back to `cargo update --workspace` if
        // the per-crate target is unknown, and is a fast no-op when
        // the version didn't change. We log + swallow errors here so
        // a missing `cargo` binary or a Bazel-managed lockfile
        // doesn't block the rewrite of other ecosystems.
        let workspace_root = app.repo.resolve_workdir(&RepoPathBuf::new(b""));
        let crate_name = &unit.qualified_names()[0];
        if let Err(e) = crate::core::cargo_lock::update_for_crate(crate_name, &workspace_root) {
            tracing::warn!("Cargo.lock update for `{crate_name}` failed (continuing): {e}",);
        }

        Ok(())
    }

    /// Rewriting just the special Belaf requirement metadata.
    fn rewrite_belaf_requirements(&self, app: &AppSession, changes: &mut ChangeList) -> Result<()> {
        // Short-circuit if no deps. Note that we can only do this if,
        // as done below, we don't clear unexpected entries in the
        // internal_dep_versions block. Should we do that?

        if app.graph().lookup(self.unit_id).internal_deps.is_empty() {
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
            let unit = graph.lookup(self.unit_id);

            for dep in &unit.internal_deps {
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
