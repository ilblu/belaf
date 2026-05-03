// Copyright 2020 Peter Williams <peter@newton.cx> and collaborators
// Licensed under the MIT License.

//! NPM (JavaScripty) projects.
//!
//! In order to operate on these, we need to rewrite `package.json` files. As
//! far as I can tell, there isn't a Rust library to load and store JSON in a
//! format-preserving way, so we might generate large diffs. Would be good to
//! fix that.

use anyhow::{anyhow, Context};
use clap::Parser;
use std::{
    collections::HashMap,
    env,
    fs::{File, OpenOptions},
    io::Write,
};
use tracing::warn;

use crate::utils::file_io::check_file_size;
use crate::{
    atry,
    core::{
        ecosystem::format_handler::{
            DiscoveredUnit, FormatHandler, RawInternalDep, WorkspaceDiscoverer,
        },
        errors::Result,
        git::repository::{ChangeList, RepoPath, RepoPathBuf, Repository},
        graph::GraphQueryBuilder,
        release_unit::VersionFieldSpec,
        resolved_release_unit::{DepRequirement, ReleaseUnitId},
        rewriters::Rewriter,
        session::AppSession,
        version::Version,
    },
};

const DEPENDENCY_KEYS: &[&str] = &["dependencies", "devDependencies", "optionalDependencies"];

/// Stateless npm `FormatHandler`. The struct is only a trait-object
/// handle; per-scan state lives in local variables in `discover_units`.
#[derive(Debug, Default)]
pub struct NpmLoader;

#[derive(Debug)]
struct PackageLoadData {
    package_name: String,
    json_path: RepoPathBuf,
    pkg_data: serde_json::Map<String, serde_json::Value>,
}

impl NpmLoader {
    /// Parse one `package.json`, returning a `(DiscoveredUnit,
    /// PackageLoadData)` pair if the file describes a real package.
    /// Returns `None` for Lerna-style root manifests that only carry
    /// `dependencies` (no `bin`/`main`/`version`/etc.).
    fn parse_one(
        &self,
        repo: &Repository,
        repopath: &RepoPathBuf,
    ) -> Result<Option<(DiscoveredUnit, PackageLoadData)>> {
        let path = repo.resolve_workdir(repopath);
        let f = atry!(
            File::open(&path);
            ["failed to open repository file `{}`", path.display()]
        );
        atry!(
            check_file_size(&f, &path);
            ["file size check failed for `{}`", path.display()]
        );
        let pkg_data: serde_json::Map<String, serde_json::Value> = atry!(
            serde_json::from_reader(f);
            ["failed to parse file `{}` as JSON", path.display()]
        );

        const CONTENT_KEYS: &[&str] = &["bin", "browser", "files", "main", "types", "version"];
        let has_content = CONTENT_KEYS.iter().any(|k| pkg_data.contains_key(*k));
        if !has_content {
            return Ok(None);
        }

        let name = pkg_data
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                anyhow!(
                    "NPM file `{}` does not have a string-typed `name` field",
                    path.display()
                )
            })?
            .to_owned();

        let version_str = pkg_data
            .get("version")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                anyhow!(
                    "NPM file `{}` does not have a string-typed `version` field",
                    path.display()
                )
            })?;
        let version = atry!(
            semver::Version::parse(version_str);
            ["cannot parse `version` field \"{}\" in `{}` as a semver version",
             version_str, path.display()]
        );

        let (dirname, _) = repopath.split_basename();
        let json_path = repopath.clone();
        let unit = DiscoveredUnit {
            qnames: vec![name.clone(), "npm".to_owned()],
            version: Version::Semver(version),
            prefix: dirname.to_owned(),
            anchor_manifest: repopath.clone(),
            rewriter_factories: vec![Box::new(move |id| {
                Box::new(PackageJsonRewriter::new(id, json_path))
            })],
            internal_deps: Vec::new(),
        };
        let load = PackageLoadData {
            package_name: name,
            json_path: repopath.clone(),
            pkg_data,
        };
        Ok(Some((unit, load)))
    }
}

impl FormatHandler for NpmLoader {
    fn name(&self) -> &'static str {
        "npm"
    }

    fn display_name(&self) -> &'static str {
        "Node.js (npm)"
    }

    fn is_manifest_file(&self, path: &RepoPath) -> bool {
        let (_, basename) = path.split_basename();
        basename.as_ref() == b"package.json"
    }

    fn parse_version(&self, content: &str) -> Result<String> {
        let pkg: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(content).map_err(|e| anyhow!("parse package.json: {e}"))?;
        let v = pkg
            .get("version")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("no version in package.json"))?;
        Ok(v.to_string())
    }

    fn default_version_field(&self) -> VersionFieldSpec {
        VersionFieldSpec::NpmPackageJson
    }

    fn tag_format_default(&self) -> &'static str {
        "{name}@v{version}"
    }

    fn make_rewriter(
        &self,
        unit_id: ReleaseUnitId,
        manifest_path: RepoPathBuf,
    ) -> Box<dyn Rewriter> {
        Box::new(PackageJsonRewriter::new(unit_id, manifest_path))
    }

    fn discover_single(
        &self,
        repo: &Repository,
        manifest_path: &RepoPath,
    ) -> Result<DiscoveredUnit> {
        let path_buf = manifest_path.to_owned();
        let (unit, _load) = self.parse_one(repo, &path_buf)?.ok_or_else(|| {
            anyhow!(
                "package.json `{}` lacks the content keys belaf considers releasable",
                manifest_path.escaped()
            )
        })?;
        Ok(unit)
    }
}

/// Workspace walker for npm: claims any `package.json` carrying a
/// `workspaces` field; enumerates members per the glob array.
#[derive(Debug, Default)]
pub struct NpmWorkspaceDiscoverer;

impl WorkspaceDiscoverer for NpmWorkspaceDiscoverer {
    fn name(&self) -> &'static str {
        "npm"
    }

    fn claims(&self, repo: &Repository, manifest_path: &RepoPath) -> bool {
        let (_, basename) = manifest_path.split_basename();
        if basename.as_ref() != b"package.json" {
            return false;
        }
        let abs = repo.resolve_workdir(manifest_path);
        let Ok(content) = std::fs::read_to_string(&abs) else {
            return false;
        };
        let Ok(pkg) = serde_json::from_str::<serde_json::Value>(&content) else {
            return false;
        };
        pkg.get("workspaces").is_some()
    }

    fn discover(&self, repo: &Repository, root_path: &RepoPath) -> Result<Vec<DiscoveredUnit>> {
        // Read root manifest, expand `workspaces` globs, parse each
        // member's package.json, wire internal deps. Top-level
        // package.json itself is treated as a unit only if it has a
        // `version` + content key (parse_one handles the filter).
        let root_abs = repo.resolve_workdir(root_path);
        let root_content = match std::fs::read_to_string(&root_abs) {
            Ok(c) => c,
            Err(_) => return Ok(Vec::new()),
        };
        let root_json: serde_json::Value = match serde_json::from_str(&root_content) {
            Ok(v) => v,
            Err(_) => return Ok(Vec::new()),
        };
        let workspace_globs = match root_json.get("workspaces") {
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect::<Vec<_>>(),
            // npm-pro shape: `{ "packages": [...] }`
            Some(serde_json::Value::Object(obj)) => obj
                .get("packages")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .map(|s| s.to_string())
                        .collect()
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        };

        let (root_dir, _) = root_path.split_basename();
        let root_dir_owned = root_dir.to_owned();
        let mut member_paths: Vec<RepoPathBuf> = Vec::new();

        // Try the root itself if it has a `name` + content keys.
        let mut tried_root_self = false;

        for glob in &workspace_globs {
            // Simple glob expansion — npm workspaces always uses
            // `path/*` or `path/**`. Scan the index for any
            // package.json under root_dir/{glob_prefix}.
            let prefix = glob.trim_end_matches("/**").trim_end_matches("/*");
            let prefix_path = if prefix.is_empty() || prefix == "." {
                root_dir_owned.clone()
            } else {
                let mut p = root_dir_owned.clone();
                p.push(prefix.as_bytes());
                p
            };
            repo.scan_paths(|p| {
                let (parent, basename) = p.split_basename();
                if basename.as_ref() != b"package.json" {
                    return Ok(());
                }
                if !crate::core::ecosystem::format_handler::is_path_inside_any(
                    parent,
                    std::slice::from_ref(&prefix_path),
                ) {
                    return Ok(());
                }
                if p == root_path {
                    tried_root_self = true;
                }
                member_paths.push(p.to_owned());
                Ok(())
            })?;
        }

        // Always include root itself in case it's a publishable pkg.
        if !tried_root_self {
            member_paths.push(root_path.to_owned());
        }

        let mut units: Vec<DiscoveredUnit> = Vec::new();
        let mut loads: Vec<PackageLoadData> = Vec::new();
        let mut name_to_index: HashMap<String, usize> = HashMap::new();

        for p in &member_paths {
            if let Some((unit, load)) = self.dummy_loader().parse_one(repo, p)? {
                let idx = units.len();
                name_to_index.insert(load.package_name.clone(), idx);
                units.push(unit);
                loads.push(load);
            }
        }

        let strict_validation = false;
        for (idx, load) in loads.iter().enumerate() {
            let maybe_internal_specs = load
                .pkg_data
                .get("internalDepVersions")
                .and_then(|v| v.as_object());
            for dep_key in DEPENDENCY_KEYS {
                let Some(dep_map) = load.pkg_data.get(*dep_key).and_then(|v| v.as_object()) else {
                    continue;
                };
                for (dep_name, dep_spec) in dep_map {
                    if !name_to_index.contains_key(dep_name) {
                        continue;
                    }
                    let req = if let Some(belaf_spec) = maybe_internal_specs
                        .and_then(|d| d.get(dep_name))
                        .and_then(|v| v.as_str())
                    {
                        match repo
                            .parse_history_ref(belaf_spec)
                            .and_then(|cref| repo.resolve_history_ref(&cref, &load.json_path))
                        {
                            Ok(r) => r,
                            Err(e) => {
                                if strict_validation {
                                    return Err(anyhow!(
                                        "invalid internalDepVersions.{} for {}: {}",
                                        dep_name,
                                        load.package_name,
                                        e
                                    ));
                                }
                                warn!(
                                    "invalid internalDepVersions.{} for {}: {}",
                                    dep_name, load.package_name, e
                                );
                                DepRequirement::Unavailable
                            }
                        }
                    } else {
                        DepRequirement::Unavailable
                    };
                    units[idx].internal_deps.push(RawInternalDep {
                        target_package_name: dep_name.clone(),
                        literal: dep_spec.as_str().unwrap_or("UNDEFINED").to_owned(),
                        requirement: req,
                    });
                }
            }
        }

        Ok(units)
    }
}

impl NpmWorkspaceDiscoverer {
    /// Borrow a stateless NpmLoader handle to reuse `parse_one`.
    fn dummy_loader(&self) -> NpmLoader {
        NpmLoader
    }
}

/// Rewrite `package.json` to include real version numbers.
#[derive(Debug)]
pub struct PackageJsonRewriter {
    unit_id: ReleaseUnitId,
    json_path: RepoPathBuf,
}

impl PackageJsonRewriter {
    /// Create a new `package.json` rewriter.
    pub fn new(unit_id: ReleaseUnitId, json_path: RepoPathBuf) -> Self {
        PackageJsonRewriter { unit_id, json_path }
    }
}

impl Rewriter for PackageJsonRewriter {
    fn rewrite(&self, app: &AppSession, changes: &mut ChangeList) -> Result<()> {
        let path = app.repo.resolve_workdir(&self.json_path);

        // Parse the JSON.
        let mut pkg_data: serde_json::Map<String, serde_json::Value> = {
            let f = atry!(
                File::open(&path);
                ["failed to open file `{}`", path.display()]
            );
            atry!(
                serde_json::from_reader(f);
                ["failed to parse file `{}` as JSON", path.display()]
            )
        };

        // Helper table for applying internal deps. Note that we use the 0'th
        // qname, not the user-facing name, since that is what is used in
        // NPM-land.

        let unit = app.graph().lookup(self.unit_id);
        let mut internal_reqs = HashMap::new();

        for dep in &unit.internal_deps[..] {
            let req_text = match dep.belaf_requirement {
                DepRequirement::Manual(ref t) => t.clone(),

                DepRequirement::Commit(_) => {
                    if let Some(ref v) = dep.resolved_version {
                        // The user can configure a custom resolution protocol
                        // to prepend to the version. This capability basically
                        // exists to let us prepend a `workspace:` when using
                        // Yarn workspaces, which helps ensure that we always
                        // resolve internal deps internally.

                        let (protocol, sep) = app
                            .npm_config
                            .internal_dep_protocol
                            .as_ref()
                            .map(|p| (p.as_ref(), ":"))
                            .unwrap_or(("", ""));

                        // Hack: For versions before 1.0, semver treats minor
                        // versions as incompatible: ^0.1 is not compatible with
                        // 0.2. This busts our paradigm. We can work around by
                        // using explicit greater-than expressions, but
                        // unfortunately in Yarn workspace expressions it seems
                        // that we can't add an upper "<1" constraint too.
                        let v = v.to_string();
                        if v.starts_with("0.") {
                            format!("{protocol}{sep}>={v}")
                        } else {
                            format!("{protocol}{sep}^{v}")
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

        // Update everything.

        pkg_data["version"] = serde_json::Value::String(unit.version.to_string());

        for dep_key in DEPENDENCY_KEYS {
            if let Some(dep_map) = pkg_data.get_mut(*dep_key).and_then(|v| v.as_object_mut()) {
                for (dep_name, dep_spec) in dep_map.iter_mut() {
                    if let Some(text) = internal_reqs.get(dep_name) {
                        *dep_spec = serde_json::Value::String(text.clone());
                    }
                }
            }
        }

        // Write it out again.

        {
            let mut f = File::create(&path)?;
            atry!(
                serde_json::to_writer_pretty(&mut f, &pkg_data);
                ["failed to overwrite JSON file `{}`", path.display()]
            );
            atry!(
                writeln!(f, "");
                ["failed to overwrite JSON file `{}`", path.display()]
            );
            changes.add_path(&self.json_path);
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

        // Parse the JSON.

        let path = app.repo.resolve_workdir(&self.json_path);

        let mut pkg_data: serde_json::Map<String, serde_json::Value> = {
            let f = atry!(
                File::open(&path);
                ["failed to open file `{}`", path.display()]
            );
            atry!(
                serde_json::from_reader(f);
                ["failed to parse file `{}` as JSON", path.display()]
            )
        };

        // Mutate.

        let reqs = match pkg_data
            .get_mut("internalDepVersions")
            .and_then(|v| v.as_object_mut())
        {
            Some(t) => t,

            None => {
                pkg_data.insert(
                    "internalDepVersions".to_owned(),
                    serde_json::Value::Object(serde_json::Map::new()),
                );
                pkg_data["internalDepVersions"]
                    .as_object_mut()
                    .expect("BUG: internalDepVersions should be an object after insertion")
            }
        };

        let graph = app.graph();
        let unit = graph.lookup(self.unit_id);

        for dep in &unit.internal_deps {
            let target = &graph.lookup(dep.ident).qualified_names()[0];

            let spec = match &dep.belaf_requirement {
                DepRequirement::Commit(cid) => cid.to_string(),
                DepRequirement::Manual(t) => format!("manual:{t}"),
                DepRequirement::Unavailable => continue,
            };

            reqs.insert(target.to_owned(), serde_json::Value::String(spec));
        }

        // Write it out again.

        {
            let f = File::create(&path)?;
            atry!(
                serde_json::to_writer_pretty(f, &pkg_data);
                ["failed to overwrite JSON file `{}`", path.display()]
            );
            changes.add_path(&self.json_path);
        }

        Ok(())
    }
}

/// Npm-specific CLI utilities.
#[derive(Debug, Eq, PartialEq, Parser)]
pub enum NpmCommands {
    /// Install $NPM_TOKEN in the user's .npmrc or .yarnrc.yml
    InstallToken(InstallTokenCommand),

    /// Write incorrect internal version requirements so that Lerna can
    /// understand them.
    LernaWorkaround(LernaWorkaroundCommand),
}

#[derive(Debug, Eq, PartialEq, Parser)]
pub struct NpmCommand {
    #[command(subcommand)]
    command: NpmCommands,
}

impl NpmCommand {
    pub fn execute(self) -> Result<i32> {
        match self.command {
            NpmCommands::InstallToken(o) => o.execute(),
            NpmCommands::LernaWorkaround(o) => o.execute(),
        }
    }
}

/// `belaf npm install-token`
#[derive(Debug, Eq, PartialEq, Parser)]
pub struct InstallTokenCommand {
    #[arg(long)]
    yarn: bool,

    #[arg(long = "registry", help = "The registry base URL.")]
    registry: Option<String>,
}

impl InstallTokenCommand {
    fn execute(self) -> Result<i32> {
        let token = atry!(
            env::var("NPM_TOKEN");
            ["missing or non-textual environment variable NPM_TOKEN"]
            (note "set NPM_TOKEN in your CI environment to publish to npm")
        );

        let mut p =
            dirs::home_dir().ok_or_else(|| anyhow!("cannot determine user's home directory"))?;

        if self.yarn {
            let registry = self
                .registry
                .unwrap_or_else(|| "https://registry.yarnpkg.com/".to_owned());

            p.push(".yarnrc.yml");

            let mut file = atry!(
                OpenOptions::new().create(true).append(true).open(&p);
                ["failed to open file `{}` for appending", p.display()]
            );

            atry!(
                writeln!(file, "npmRegistries:");
                ["failed to write token data to file `{}`", p.display()]
            );
            atry!(
                writeln!(file, "  \"{}\":", registry);
                ["failed to write token data to file `{}`", p.display()]
            );
            atry!(
                writeln!(file, "    npmAuthToken: {}", token);
                ["failed to write token data to file `{}`", p.display()]
            );
        } else {
            let registry = self
                .registry
                .unwrap_or_else(|| "//registry.npmjs.org/".to_owned());

            p.push(".npmrc");

            let mut file = atry!(
                OpenOptions::new().create(true).append(true).open(&p);
                ["failed to open file `{}` for appending", p.display()]
            );

            atry!(
                writeln!(file, "{}:_authToken={}", registry, token);
                ["failed to write token data to file `{}`", p.display()]
            );
        }

        Ok(0)
    }
}

/// `belaf npm lerna-workaround`
#[derive(Debug, Eq, PartialEq, Parser)]
pub struct LernaWorkaroundCommand {}

impl LernaWorkaroundCommand {
    fn execute(self) -> Result<i32> {
        let mut sess = AppSession::initialize_default()?;

        let mut q = GraphQueryBuilder::default();
        q.only_ecosystem("npm");
        let idents = sess
            .graph()
            .query(q)
            .context("could not select projects for `npm lerna-workaround`")?;

        sess.fake_internal_deps();

        let mut changes = ChangeList::default();

        for ident in &idents {
            let unit = sess.graph().lookup(*ident);

            for rw in &unit.rewriters {
                atry!(
                    rw.rewrite(&sess, &mut changes);
                    ["failed to rewrite metadata for `{}`", unit.user_facing_name]
                );
            }
        }

        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_parse_package_json_name() {
        let json = r#"{"name": "@scope/package", "version": "1.0.0"}"#;
        let parsed: serde_json::Value =
            serde_json::from_str(json).expect("BUG: test JSON should parse");

        let name = parsed.get("name").and_then(|v| v.as_str());
        assert_eq!(name, Some("@scope/package"));
    }

    #[test]
    fn test_parse_package_json_version() {
        let json = r#"{"name": "my-package", "version": "2.1.3"}"#;
        let parsed: serde_json::Value =
            serde_json::from_str(json).expect("BUG: test JSON should parse");

        let version = parsed.get("version").and_then(|v| v.as_str());
        assert_eq!(version, Some("2.1.3"));
    }

    #[test]
    fn test_parse_package_json_dependencies() {
        let json = r#"{
            "name": "test",
            "dependencies": {
                "react": "^18.0.0",
                "lodash": "~4.17.0"
            }
        }"#;
        let parsed: serde_json::Value =
            serde_json::from_str(json).expect("BUG: test JSON should parse");

        let deps = parsed.get("dependencies").and_then(|v| v.as_object());
        assert!(deps.is_some());
        assert_eq!(
            deps.expect("BUG: deps should be Some after assertion")
                .len(),
            2
        );
    }

    #[test]
    fn test_parse_package_json_with_workspace() {
        let json = r#"{
            "name": "root",
            "workspaces": ["packages/*"]
        }"#;
        let parsed: serde_json::Value =
            serde_json::from_str(json).expect("BUG: test JSON should parse");

        let workspaces = parsed.get("workspaces");
        assert!(workspaces.is_some());
    }
}
