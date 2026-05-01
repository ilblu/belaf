//! Maven (Java/Kotlin/Scala) projects.
//!
//! TODO(belaf-3.0/wave1b): split this 1521-LOC file into
//! `maven/{pom_parser,property_resolver,pom_rewriter,util}.rs`. The
//! Ecosystem trait + ReleaseUnitGraphBuilder rename in Wave 1a/1b
//! shipped without needing this physical split, but the file should
//! be modularized as part of a later cleanup PR before BOM-support
//! and `settings.xml` work in 3.x crosses through it.
//!
//! Loads `pom.xml` files into the project graph, then rewrites them on
//! release. Supports the subset of Maven that real CI-friendly projects
//! actually need (plan §12):
//!
//! - Single-module POMs.
//! - `<parent>` references — rewritten in the child when the parent's
//!   version bumps. Cycles between POMs via `<parent>` are detected via
//!   Tarjan-SCC at finalize time and reported with all members.
//! - `<dependencyManagement>` — version bumps in inter-project deps
//!   propagate.
//! - Multi-module aggregators (`<modules>`): each child is loaded as a
//!   separate project, the aggregator itself is a project too.
//! - CI-friendly property resolution: `${revision}`, `${sha1}`,
//!   `${changelist}`, `${project.version}`. Walks `<parent>`-inherited
//!   `<properties>`. **No** `-D` system properties, **no** environment
//!   variables, **no** profiles or `settings.xml` (out of scope).
//!
//! An unsupported property name in a `<version>` field is a hard error
//! that names the supported set so the user can pick one of those
//! instead.
//!
//! Read **and** write go through `quick_xml::Reader`/`Writer` so we keep
//! whitespace and comments byte-stable across rewrite — line-based was
//! considered (csproj uses it) but POM `<version>` elements can have
//! internal whitespace and comments between tags, so the streaming-XML
//! path is safer.

use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{Cursor, Read, Write},
    path::PathBuf,
};

use anyhow::{anyhow, Context as _};
use petgraph::{algo::tarjan_scc, graph::DiGraph};
use quick_xml::{
    events::{BytesText, Event},
    Reader, Writer,
};
use tracing::info;

use crate::{
    atry,
    core::{
        config::syntax::ProjectConfiguration,
        ecosystem::registry::Ecosystem,
        errors::Result,
        git::repository::{ChangeList, RepoPath, RepoPathBuf, Repository},
        graph::ReleaseUnitGraphBuilder,
        resolved_release_unit::{DepRequirement, DependencyTarget, ReleaseUnitId},
        rewriters::Rewriter,
        session::{AppBuilder, AppSession},
        version::Version,
    },
};

/// Names of properties that are allowed to appear in a `<version>` element.
/// These are the Maven "CI friendly" set
/// (<https://maven.apache.org/guides/mini/guide-maven-ci-friendly.html>) plus
/// `project.version` for inter-module self-reference.
const SUPPORTED_PROPERTIES: &[&str] = &["revision", "sha1", "changelist", "project.version"];

/// Loader collecting POM files during the index scan. Resolution (parent
/// inheritance, property resolution, multi-module discovery) happens in
/// `into_projects` where we have the full set.
#[derive(Debug, Default)]
pub struct MavenLoader {
    pom_paths: Vec<RepoPathBuf>,
}

/// One parsed POM, before resolution. Coordinates may still contain
/// `${...}` placeholders at this point.
#[derive(Debug, Clone)]
struct ParsedPom {
    repo_path: RepoPathBuf,
    fs_path: PathBuf,
    group_id: Option<String>,
    artifact_id: String,
    version: Option<String>,
    parent: Option<ParentRef>,
    properties: HashMap<String, String>,
    modules: Vec<String>,
    dependencies: Vec<DepRef>,
    /// True if this POM uses `<packaging>pom</packaging>` (typical for
    /// aggregators, but not required — we treat aggregator-ness purely by
    /// `<modules>` presence).
    is_pom_packaging: bool,
}

#[derive(Debug, Clone)]
struct ParentRef {
    group_id: String,
    artifact_id: String,
    version: String,
}

#[derive(Debug, Clone)]
struct DepRef {
    group_id: String,
    artifact_id: String,
    /// `<version>` is optional in `<dependencies>` when inherited from
    /// `<dependencyManagement>` higher up. We only track inter-project
    /// deps that have a version we can resolve.
    version: Option<String>,
}

impl MavenLoader {
    /// Collect a `pom.xml` path. Resolution happens in [`into_projects`].
    pub fn record_path(&mut self, dirname: &RepoPath, basename: &RepoPath) {
        if basename.as_ref() != b"pom.xml" {
            return;
        }
        let mut p = dirname.to_owned();
        p.push(basename);
        self.pom_paths.push(p);
    }

    /// Drain into the [`AppBuilder`]. Order of operations:
    ///
    /// 1. Parse every collected `pom.xml` (no resolution yet).
    /// 2. Build the parent-graph and run Tarjan-SCC for cycle detection.
    /// 3. Resolve coordinates: walk parent chain for missing groupId /
    ///    version inheritance, then run property substitution against the
    ///    accumulated `<properties>` map.
    /// 4. Register one project per POM under user-facing name
    ///    `groupId:artifactId`.
    /// 5. Wire inter-project dependencies into the graph.
    pub fn into_projects(
        self,
        app: &mut AppBuilder,
        pconfig: &HashMap<String, ProjectConfiguration>,
    ) -> Result<()> {
        if self.pom_paths.is_empty() {
            return Ok(());
        }

        info!("loading {} pom.xml file(s)", self.pom_paths.len());

        // Phase 1: parse.
        let mut parsed: Vec<ParsedPom> = Vec::with_capacity(self.pom_paths.len());
        for repo_path in &self.pom_paths {
            let fs_path = app.repo.resolve_workdir(repo_path);
            let pom = atry!(
                ParsedPom::from_file(repo_path, &fs_path);
                ["failed to parse Maven POM `{}`", repo_path.escaped()]
            );
            parsed.push(pom);
        }

        // Phase 2: build coords -> index map (using whatever coords we
        // have at this stage — inheritance still pending).
        let mut coord_to_idx: HashMap<(String, String), usize> = HashMap::new();
        for (idx, pom) in parsed.iter().enumerate() {
            if let Some(gid) = &pom.group_id {
                coord_to_idx.insert((gid.clone(), pom.artifact_id.clone()), idx);
            }
        }

        // Phase 2b: parent-cycle detection via Tarjan-SCC. If any SCC has
        // size > 1, that's a cycle in the inheritance graph.
        detect_parent_cycles(&parsed, &coord_to_idx)?;

        // Phase 3: inheritance + property resolution.
        let mut resolved: Vec<ResolvedPom> = Vec::with_capacity(parsed.len());
        for idx in 0..parsed.len() {
            let r = atry!(
                resolve_pom(idx, &parsed, &coord_to_idx);
                ["failed to resolve Maven coordinates for `{}`", parsed[idx].repo_path.escaped()]
            );
            resolved.push(r);
        }

        // Re-key the coord map now that every POM has a definite groupId.
        let mut resolved_coord_to_idx: HashMap<(String, String), usize> = HashMap::new();
        for (idx, r) in resolved.iter().enumerate() {
            resolved_coord_to_idx.insert((r.group_id.clone(), r.artifact_id.clone()), idx);
        }

        // Phase 4: register projects in the graph.
        let mut idx_to_pid: HashMap<usize, ReleaseUnitId> = HashMap::new();
        for (idx, r) in resolved.iter().enumerate() {
            let user_name = format!("{}:{}", r.group_id, r.artifact_id);
            let qnames = vec![user_name.clone(), "maven".to_owned()];

            if let Some(pid) = app.graph.try_add_project(qnames, pconfig) {
                let proj = app.graph.lookup_mut(pid);

                let version = atry!(
                    semver::Version::parse(&r.version)
                        .map_err(|e| anyhow!("not semver: {e}"));
                    ["Maven version `{}` for `{}` is not parseable as semver",
                     r.version, user_name]
                    (note "belaf 2.0 supports semver-shaped Maven versions only (e.g. 1.2.3, 1.0.0-SNAPSHOT). \
                     Pure-numeric chains like `1.0` need a third component (`1.0.0`).")
                );
                proj.version = Some(Version::Semver(version));

                let (prefix, _) = parsed[idx].repo_path.split_basename();
                proj.prefix = Some(prefix.to_owned());

                proj.rewriters.push(Box::new(MavenRewriter::new(
                    pid,
                    parsed[idx].repo_path.clone(),
                )));

                idx_to_pid.insert(idx, pid);
            }
        }

        // Phase 5: inter-project deps. Both `<dependencies>` and `<parent>`
        // edges produce a graph dep so toposort + bump propagation work.
        for (idx, _) in resolved.iter().enumerate() {
            let Some(&depender_pid) = idx_to_pid.get(&idx) else {
                continue;
            };
            let pom = &parsed[idx];

            // <parent>: child depends on parent.
            if let Some(p) = &pom.parent {
                if let Some(&parent_idx) =
                    resolved_coord_to_idx.get(&(p.group_id.clone(), p.artifact_id.clone()))
                {
                    if let Some(&parent_pid) = idx_to_pid.get(&parent_idx) {
                        if parent_pid != depender_pid {
                            app.graph.add_dependency(
                                depender_pid,
                                DependencyTarget::Ident(parent_pid),
                                p.version.clone(),
                                DepRequirement::Manual(p.version.clone()),
                            );
                        }
                    }
                }
            }

            // <dependencies>: only those with explicit versions and an
            // intra-repo target produce graph edges.
            for dep in &pom.dependencies {
                let Some(dep_version) = &dep.version else {
                    continue;
                };
                let key = (dep.group_id.clone(), dep.artifact_id.clone());
                let Some(&dep_idx) = resolved_coord_to_idx.get(&key) else {
                    continue;
                };
                let Some(&dep_pid) = idx_to_pid.get(&dep_idx) else {
                    continue;
                };
                if dep_pid == depender_pid {
                    continue;
                }
                app.graph.add_dependency(
                    depender_pid,
                    DependencyTarget::Ident(dep_pid),
                    dep_version.clone(),
                    DepRequirement::Manual(dep_version.clone()),
                );
            }
        }

        Ok(())
    }
}

impl Ecosystem for MavenLoader {
    fn name(&self) -> &'static str {
        "maven"
    }
    fn display_name(&self) -> &'static str {
        "Maven"
    }
    fn version_file(&self) -> &'static str {
        "pom.xml"
    }
    fn tag_format_default(&self) -> &'static str {
        "{groupId}/{artifactId}@v{version}"
    }
    fn tag_template_vars(&self) -> &'static [&'static str] {
        &["name", "version", "ecosystem", "groupId", "artifactId"]
    }

    fn process_index_item(
        &mut self,
        _repo: &Repository,
        _graph: &mut ReleaseUnitGraphBuilder,
        _repopath: &RepoPath,
        dirname: &RepoPath,
        basename: &RepoPath,
        _pconfig: &HashMap<String, ProjectConfiguration>,
    ) -> Result<()> {
        self.record_path(dirname, basename);
        Ok(())
    }

    fn finalize(
        self: Box<Self>,
        app: &mut AppBuilder,
        pconfig: &HashMap<String, ProjectConfiguration>,
    ) -> Result<()> {
        (*self).into_projects(app, pconfig)
    }
}

// ---------------------------------------------------------------------------
// POM parsing
// ---------------------------------------------------------------------------

impl ParsedPom {
    fn from_file(repo_path: &RepoPath, fs_path: &std::path::Path) -> Result<Self> {
        let mut content = String::new();
        atry!(
            File::open(fs_path).and_then(|mut f| f.read_to_string(&mut content));
            ["failed to read Maven POM `{}`", fs_path.display()]
        );
        Self::from_str(repo_path, fs_path, &content)
    }

    fn from_str(repo_path: &RepoPath, fs_path: &std::path::Path, content: &str) -> Result<Self> {
        let mut reader = Reader::from_str(content);
        reader.config_mut().trim_text(false);

        let mut buf = Vec::new();
        // Track tag stack so we can disambiguate top-level vs. nested
        // `<version>` (e.g. plugin / dependency child versions).
        let mut stack: Vec<String> = Vec::new();

        let mut pom = ParsedPom {
            repo_path: repo_path.to_owned(),
            fs_path: fs_path.to_path_buf(),
            group_id: None,
            artifact_id: String::new(),
            version: None,
            parent: None,
            properties: HashMap::new(),
            modules: Vec::new(),
            dependencies: Vec::new(),
            is_pom_packaging: false,
        };

        // Scratch state for nested blocks (parent / dependency item /
        // properties / modules / dependencyManagement.)
        let mut current_parent: Option<PartialParent> = None;
        let mut current_dep: Option<PartialDep> = None;
        let mut text_buf = String::new();

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) => {
                    let name = local_name(e.name().as_ref());
                    stack.push(name.clone());

                    match path(&stack).as_deref() {
                        Some("project/parent") => current_parent = Some(PartialParent::default()),
                        Some(p)
                            if p == "project/dependencies/dependency"
                                || p == "project/dependencyManagement/dependencies/dependency" =>
                        {
                            current_dep = Some(PartialDep::default());
                        }
                        _ => {}
                    }
                    text_buf.clear();
                }
                Ok(Event::Text(t)) => {
                    let txt = atry!(
                        t.decode().map(|s| s.into_owned());
                        ["failed to decode text node in POM `{}`", fs_path.display()]
                    );
                    text_buf.push_str(&txt);
                }
                Ok(Event::CData(t)) => {
                    text_buf.push_str(&String::from_utf8_lossy(t.as_ref()));
                }
                Ok(Event::End(_)) => {
                    let trimmed = text_buf.trim().to_string();
                    let p = path(&stack);

                    match p.as_deref() {
                        Some("project/groupId") => pom.group_id = Some(trimmed.clone()),
                        Some("project/artifactId") => pom.artifact_id = trimmed.clone(),
                        Some("project/version") => pom.version = Some(trimmed.clone()),
                        Some("project/packaging") if trimmed == "pom" => {
                            pom.is_pom_packaging = true;
                        }
                        Some("project/parent/groupId") => {
                            if let Some(pp) = current_parent.as_mut() {
                                pp.group_id = Some(trimmed.clone());
                            }
                        }
                        Some("project/parent/artifactId") => {
                            if let Some(pp) = current_parent.as_mut() {
                                pp.artifact_id = Some(trimmed.clone());
                            }
                        }
                        Some("project/parent/version") => {
                            if let Some(pp) = current_parent.as_mut() {
                                pp.version = Some(trimmed.clone());
                            }
                        }
                        Some("project/parent/relativePath") => {
                            // Parsed but discarded — see ParentRef docstring.
                        }
                        Some("project/parent") => {
                            if let Some(pp) = current_parent.take() {
                                pom.parent = pp.into_parent_ref();
                            }
                        }
                        Some("project/modules/module") => {
                            pom.modules.push(trimmed.clone());
                        }
                        Some(s)
                            if s == "project/dependencies/dependency/groupId"
                                || s
                                    == "project/dependencyManagement/dependencies/dependency/groupId" =>
                        {
                            if let Some(d) = current_dep.as_mut() {
                                d.group_id = Some(trimmed.clone());
                            }
                        }
                        Some(s)
                            if s == "project/dependencies/dependency/artifactId"
                                || s == "project/dependencyManagement/dependencies/dependency/artifactId" =>
                        {
                            if let Some(d) = current_dep.as_mut() {
                                d.artifact_id = Some(trimmed.clone());
                            }
                        }
                        Some(s)
                            if s == "project/dependencies/dependency/version"
                                || s == "project/dependencyManagement/dependencies/dependency/version" =>
                        {
                            if let Some(d) = current_dep.as_mut() {
                                d.version = Some(trimmed.clone());
                            }
                        }
                        Some(s)
                            if s == "project/dependencies/dependency"
                                || s == "project/dependencyManagement/dependencies/dependency" =>
                        {
                            if let Some(d) = current_dep.take() {
                                if let Some(dr) = d.into_dep_ref() {
                                    pom.dependencies.push(dr);
                                }
                            }
                        }
                        Some(s)
                            if s.starts_with("project/properties/") && stack.len() == 3 =>
                        {
                            // 3-level path means we are at a direct child of
                            // <properties>. The leaf name is the property
                            // key.
                            if let Some(name) = stack.last() {
                                pom.properties.insert(name.clone(), trimmed.clone());
                            }
                        }
                        _ => {}
                    }

                    stack.pop();
                    text_buf.clear();
                }
                Ok(Event::Eof) => break,
                Ok(_) => {}
                Err(e) => {
                    return Err(anyhow!(
                        "POM `{}` is not well-formed XML: {}",
                        fs_path.display(),
                        e
                    ));
                }
            }
            buf.clear();
        }

        if pom.artifact_id.is_empty() {
            return Err(anyhow!(
                "Maven POM `{}` has no <artifactId> at the project level",
                fs_path.display()
            ));
        }

        Ok(pom)
    }
}

#[derive(Debug, Default)]
struct PartialParent {
    group_id: Option<String>,
    artifact_id: Option<String>,
    version: Option<String>,
}

impl PartialParent {
    fn into_parent_ref(self) -> Option<ParentRef> {
        Some(ParentRef {
            group_id: self.group_id?,
            artifact_id: self.artifact_id?,
            version: self.version?,
        })
    }
}

#[derive(Debug, Default)]
struct PartialDep {
    group_id: Option<String>,
    artifact_id: Option<String>,
    version: Option<String>,
}

impl PartialDep {
    fn into_dep_ref(self) -> Option<DepRef> {
        Some(DepRef {
            group_id: self.group_id?,
            artifact_id: self.artifact_id?,
            version: self.version,
        })
    }
}

fn local_name(qname: &[u8]) -> String {
    // Strip namespace prefix (`maven:project` → `project`). POMs typically
    // don't have prefixes but we defensively handle them.
    let s = std::str::from_utf8(qname).unwrap_or_default();
    match s.rsplit_once(':') {
        Some((_, local)) => local.to_string(),
        None => s.to_string(),
    }
}

fn path(stack: &[String]) -> Option<String> {
    if stack.is_empty() {
        None
    } else {
        Some(stack.join("/"))
    }
}

// ---------------------------------------------------------------------------
// Parent-cycle detection
// ---------------------------------------------------------------------------

fn detect_parent_cycles(
    pomstack: &[ParsedPom],
    coord_to_idx: &HashMap<(String, String), usize>,
) -> Result<()> {
    let mut g: DiGraph<usize, ()> = DiGraph::new();
    let nodes: Vec<_> = (0..pomstack.len()).map(|i| g.add_node(i)).collect();

    for (i, pom) in pomstack.iter().enumerate() {
        if let Some(parent) = &pom.parent {
            if let Some(&j) =
                coord_to_idx.get(&(parent.group_id.clone(), parent.artifact_id.clone()))
            {
                g.add_edge(nodes[i], nodes[j], ());
            }
        }
    }

    for scc in tarjan_scc(&g) {
        if scc.len() > 1 {
            let names: Vec<String> = scc
                .iter()
                .map(|nx| {
                    let i = g[*nx];
                    let p = &pomstack[i];
                    let gid = p.group_id.as_deref().unwrap_or("?");
                    format!("{gid}:{}", p.artifact_id)
                })
                .collect();
            return Err(anyhow!(
                "Maven <parent> cycle detected among {} POMs: {}",
                names.len(),
                names.join(" → ")
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Inheritance + property resolution
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct ResolvedPom {
    group_id: String,
    artifact_id: String,
    version: String,
}

fn resolve_pom(
    idx: usize,
    pomstack: &[ParsedPom],
    coord_to_idx: &HashMap<(String, String), usize>,
) -> Result<ResolvedPom> {
    // Walk parent chain to accumulate inherited groupId / version /
    // properties. The bottom-most POM's value wins.
    let mut chain: Vec<usize> = vec![idx];
    let mut cursor = idx;
    let mut seen = HashSet::new();
    seen.insert(cursor);
    while let Some(parent) = &pomstack[cursor].parent {
        let key = (parent.group_id.clone(), parent.artifact_id.clone());
        let Some(&p_idx) = coord_to_idx.get(&key) else {
            break;
        };
        if !seen.insert(p_idx) {
            // Defensive: the SCC pass should have caught this.
            break;
        }
        chain.push(p_idx);
        cursor = p_idx;
    }

    // Accumulate properties bottom-up so children override parents.
    let mut props: HashMap<String, String> = HashMap::new();
    for &i in chain.iter().rev() {
        for (k, v) in &pomstack[i].properties {
            props.insert(k.clone(), v.clone());
        }
    }

    let pom = &pomstack[idx];
    let group_id = pom
        .group_id
        .clone()
        .or_else(|| pom.parent.as_ref().map(|p| p.group_id.clone()))
        .ok_or_else(|| {
            anyhow!(
                "Maven POM `{}` has no <groupId> and no <parent> to inherit from",
                pom.fs_path.display()
            )
        })?;
    let raw_version = pom
        .version
        .clone()
        .or_else(|| pom.parent.as_ref().map(|p| p.version.clone()))
        .ok_or_else(|| {
            anyhow!(
                "Maven POM `{}` has no <version> and no <parent> to inherit from",
                pom.fs_path.display()
            )
        })?;

    let version = resolve_property(&raw_version, &props, &group_id, &pom.artifact_id, pom)?;

    Ok(ResolvedPom {
        group_id,
        artifact_id: pom.artifact_id.clone(),
        version,
    })
}

fn resolve_property(
    raw: &str,
    props: &HashMap<String, String>,
    project_group_id: &str,
    project_artifact_id: &str,
    pom: &ParsedPom,
) -> Result<String> {
    // Single-pass `${name}` substitution. Recursive expansion is allowed
    // (a property whose value is itself `${other}`), capped at a small
    // depth to avoid pathological loops in malformed POMs.
    let mut current = raw.to_string();
    for _depth in 0..8 {
        let Some(start) = current.find("${") else {
            return Ok(current);
        };
        let Some(end_rel) = current[start..].find('}') else {
            return Ok(current);
        };
        let end = start + end_rel;
        let name = &current[start + 2..end];

        if !SUPPORTED_PROPERTIES.contains(&name) {
            return Err(anyhow!(
                "Maven POM `{}`: unsupported property `${{{}}}` in version field. \
                 Supported properties: {}. \
                 belaf does not resolve user-defined `<properties>` keys in version fields, \
                 `-D` system properties, environment variables, or `<settings.xml>` profiles.",
                pom.fs_path.display(),
                name,
                SUPPORTED_PROPERTIES.join(", ")
            ));
        }

        let value = match name {
            "project.version" => {
                // Self-reference: only meaningful in inter-module refs;
                // in a top-level <version> it would be a cycle. Resolve to
                // the raw POM's version (without recursing).
                pom.version.clone().unwrap_or_default()
            }
            other => props
                .get(other)
                .cloned()
                .ok_or_else(|| {
                    anyhow!(
                        "Maven POM `{}`: property `${{{}}}` is recognised but has no <properties> entry",
                        pom.fs_path.display(),
                        other
                    )
                })?,
        };

        let _ = project_group_id;
        let _ = project_artifact_id;

        let mut next = String::with_capacity(current.len());
        next.push_str(&current[..start]);
        next.push_str(&value);
        next.push_str(&current[end + 1..]);
        current = next;
    }
    Err(anyhow!(
        "Maven POM `{}`: property substitution did not converge after 8 passes — \
         likely a self-referential `<properties>` definition",
        pom.fs_path.display()
    ))
}

// ---------------------------------------------------------------------------
// Rewriter
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct MavenRewriter {
    proj_id: ReleaseUnitId,
    pom_path: RepoPathBuf,
}

impl MavenRewriter {
    pub fn new(proj_id: ReleaseUnitId, pom_path: RepoPathBuf) -> Self {
        Self { proj_id, pom_path }
    }
}

impl Rewriter for MavenRewriter {
    fn rewrite(&self, app: &AppSession, changes: &mut ChangeList) -> Result<()> {
        let fs_path = app.repo.resolve_workdir(&self.pom_path);

        let mut content = String::new();
        atry!(
            File::open(&fs_path).and_then(|mut f| f.read_to_string(&mut content));
            ["failed to open POM `{}`", fs_path.display()]
        );

        let proj = app.graph().lookup(self.proj_id);
        let new_version = proj.version.to_string();

        // Build the coord-to-version lookup. For each inter-project dep
        // we look up the target's bumped version via the graph, keyed
        // by Maven `groupId:artifactId` (which is also the sibling
        // project's user-facing name).
        let graph = app.graph();
        let coord_lookup = |group_id: &str, artifact_id: &str| -> Option<String> {
            let user_name = format!("{group_id}:{artifact_id}");
            let pid = graph.lookup_ident(&user_name)?;
            // Skip self — the top-level <version> path handles it.
            if pid == self.proj_id {
                return None;
            }
            Some(graph.lookup(pid).version.to_string())
        };

        let new_content = atry!(
            rewrite_pom(&content, &new_version, &coord_lookup);
            ["failed to rewrite POM `{}`", fs_path.display()]
        );

        let mut f = atry!(
            File::create(&fs_path);
            ["failed to write POM `{}`", fs_path.display()]
        );
        atry!(
            f.write_all(new_content.as_bytes());
            ["failed to write POM body to `{}`", fs_path.display()]
        );
        changes.add_path(&self.pom_path);

        Ok(())
    }
}

/// Rewrite a POM's top-level `<version>`, plus any `<parent><version>` and
/// `<dependencies>` / `<dependencyManagement>` member versions whose
/// `groupId:artifactId` resolves to a sibling project that's also being
/// bumped. Preserves every other byte (comments, whitespace, namespaces,
/// unrelated tags).
///
/// The per-coord lookup decides whether a given `<dependency>` or
/// `<parent>` block should be rewritten — `None` from the lookup means
/// "leave alone" (e.g. external deps, third-party libs, the project's
/// own self-reference). The caller wires `coord_lookup` against the
/// graph in `MavenRewriter::rewrite`.
fn rewrite_pom<F>(content: &str, top_level_version: &str, coord_lookup: &F) -> Result<String>
where
    F: Fn(&str, &str) -> Option<String>,
{
    let mut reader = Reader::from_str(content);
    reader.config_mut().trim_text(false);

    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buf = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut in_top_version = false;
    let mut wrote_replacement = false;
    // Buffered events for the currently-open `<parent>` or
    // `<dependency>` block. Empty when not in such a block.
    let mut buffered: Vec<Event<'static>> = Vec::new();
    // Path of the buffered scope so we know which End event closes it.
    let mut buffered_scope: Option<String> = None;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref());
                stack.push(name.clone());
                let p = path(&stack);

                // Enter a buffered scope on `<parent>` or `<dependency>`.
                if buffered_scope.is_none()
                    && matches!(
                        p.as_deref(),
                        Some("project/parent")
                            | Some("project/dependencies/dependency")
                            | Some("project/dependencyManagement/dependencies/dependency",)
                    )
                {
                    buffered_scope = p.clone();
                    buffered.push(Event::Start(e.clone()).into_owned());
                    buf.clear();
                    continue;
                }

                if buffered_scope.is_some() {
                    buffered.push(Event::Start(e.clone()).into_owned());
                } else if p.as_deref() == Some("project/version") {
                    in_top_version = true;
                    wrote_replacement = false;
                    writer
                        .write_event(Event::Start(e.clone()))
                        .map_err(|err| anyhow!("xml write: {err}"))?;
                } else {
                    writer
                        .write_event(Event::Start(e.clone()))
                        .map_err(|err| anyhow!("xml write: {err}"))?;
                }
            }
            Ok(Event::End(e)) => {
                let p = path(&stack);

                if let Some(scope) = &buffered_scope {
                    if p.as_deref() == Some(scope.as_str()) {
                        // Closing the buffered scope. Apply rewrite and
                        // flush all buffered events.
                        buffered.push(Event::End(e.clone()).into_owned());
                        let rewritten = rewrite_buffered_block(&buffered, coord_lookup)?;
                        for ev in rewritten {
                            writer
                                .write_event(ev)
                                .map_err(|err| anyhow!("xml write: {err}"))?;
                        }
                        buffered.clear();
                        buffered_scope = None;
                    } else {
                        buffered.push(Event::End(e.clone()).into_owned());
                    }
                } else if p.as_deref() == Some("project/version") {
                    if !wrote_replacement {
                        writer
                            .write_event(Event::Text(BytesText::new(top_level_version)))
                            .map_err(|err| anyhow!("xml write: {err}"))?;
                    }
                    in_top_version = false;
                    writer
                        .write_event(Event::End(e.clone()))
                        .map_err(|err| anyhow!("xml write: {err}"))?;
                } else {
                    writer
                        .write_event(Event::End(e.clone()))
                        .map_err(|err| anyhow!("xml write: {err}"))?;
                }
                stack.pop();
            }
            Ok(Event::Text(t)) => {
                if buffered_scope.is_some() {
                    buffered.push(Event::Text(t.clone()).into_owned());
                } else if in_top_version {
                    let original = t.decode().unwrap_or_default();
                    if original.trim().is_empty() {
                        writer
                            .write_event(Event::Text(t.clone()))
                            .map_err(|err| anyhow!("xml write: {err}"))?;
                    } else if !wrote_replacement {
                        writer
                            .write_event(Event::Text(BytesText::new(top_level_version)))
                            .map_err(|err| anyhow!("xml write: {err}"))?;
                        wrote_replacement = true;
                    } else {
                        // Drop subsequent text inside <version> — see
                        // pre-buffer note about malformed multi-text POMs.
                    }
                } else {
                    writer
                        .write_event(Event::Text(t.clone()))
                        .map_err(|err| anyhow!("xml write: {err}"))?;
                }
            }
            Ok(Event::Eof) => break,
            Ok(other) => {
                if buffered_scope.is_some() {
                    buffered.push(other.into_owned());
                } else {
                    writer
                        .write_event(other_event_owned(&other))
                        .map_err(|err| anyhow!("xml write: {err}"))?;
                }
            }
            Err(e) => return Err(anyhow!("xml read error: {e}")),
        }
        buf.clear();
    }

    let inner = writer.into_inner().into_inner();
    let s = String::from_utf8(inner).context("rewritten POM is not valid UTF-8")?;
    Ok(s)
}

fn other_event_owned<'a>(e: &Event<'a>) -> Event<'a> {
    e.clone()
}

/// Apply the inter-project version-rewrite logic to one buffered
/// `<parent>` or `<dependency>` block. Returns the events to emit —
/// either the buffer as-is (when the block doesn't reference a sibling
/// project, e.g. `org.junit.jupiter:junit-jupiter-api`) or with the
/// `<version>` text replaced.
///
/// We walk the buffer twice: first to extract `groupId` and
/// `artifactId` (and locate the version text node), then to emit with
/// optional substitution. Two passes are simpler than threading
/// "version-not-yet-known" state through one pass — the buffers are
/// always tiny (3-5 child elements).
fn rewrite_buffered_block<F>(
    buffered: &[Event<'static>],
    coord_lookup: &F,
) -> Result<Vec<Event<'static>>>
where
    F: Fn(&str, &str) -> Option<String>,
{
    // First pass: extract coords + figure out whether we'll rewrite.
    let mut group_id: Option<String> = None;
    let mut artifact_id: Option<String> = None;
    let mut local_stack: Vec<String> = Vec::new();
    let mut last_text: Option<String> = None;

    for ev in buffered {
        match ev {
            Event::Start(e) => {
                local_stack.push(local_name(e.name().as_ref()));
                last_text = None;
            }
            Event::Text(t) => {
                last_text = Some(t.decode().unwrap_or_default().into_owned());
            }
            Event::End(_) => {
                let depth = local_stack.len();
                if depth == 2 {
                    let leaf = &local_stack[depth - 1];
                    if let Some(text) = last_text.take() {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() {
                            if leaf == "groupId" {
                                group_id = Some(trimmed.to_string());
                            } else if leaf == "artifactId" {
                                artifact_id = Some(trimmed.to_string());
                            }
                        }
                    }
                }
                local_stack.pop();
            }
            _ => {}
        }
    }

    let new_version = match (group_id.as_deref(), artifact_id.as_deref()) {
        (Some(g), Some(a)) => coord_lookup(g, a),
        _ => None,
    };

    let Some(new_version) = new_version else {
        // External dep or self-reference — emit unchanged.
        return Ok(buffered.to_vec());
    };

    // Second pass: emit with `<version>` text substituted.
    let mut out = Vec::with_capacity(buffered.len());
    let mut local_stack: Vec<String> = Vec::new();
    let mut in_version = false;
    let mut wrote_replacement = false;

    for ev in buffered {
        match ev {
            Event::Start(e) => {
                local_stack.push(local_name(e.name().as_ref()));
                if local_stack.len() == 2 && local_stack[1] == "version" {
                    in_version = true;
                    wrote_replacement = false;
                }
                out.push(ev.clone());
            }
            Event::End(e) => {
                if in_version && !wrote_replacement {
                    // <version></version> with no text — inject one.
                    out.push(Event::Text(BytesText::new(&new_version)).into_owned());
                }
                if local_stack.len() == 2 && local_stack[1] == "version" {
                    in_version = false;
                }
                local_stack.pop();
                out.push(Event::End(e.clone()).into_owned());
            }
            Event::Text(t) => {
                if in_version {
                    let original = t.decode().unwrap_or_default();
                    if original.trim().is_empty() {
                        out.push(Event::Text(t.clone()).into_owned());
                    } else if !wrote_replacement {
                        out.push(Event::Text(BytesText::new(&new_version)).into_owned());
                        wrote_replacement = true;
                    }
                    // Drop subsequent text inside <version>.
                } else {
                    out.push(Event::Text(t.clone()).into_owned());
                }
            }
            other => out.push(other.clone()),
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn parse(content: &str) -> ParsedPom {
        let rp = RepoPathBuf::new(b"pom.xml");
        ParsedPom::from_str(rp.as_ref(), Path::new("pom.xml"), content).expect("parse")
    }

    #[test]
    fn parses_single_module_pom() {
        let p = parse(
            r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>lib</artifactId>
  <version>1.0.0</version>
</project>"#,
        );
        assert_eq!(p.group_id.as_deref(), Some("com.example"));
        assert_eq!(p.artifact_id, "lib");
        assert_eq!(p.version.as_deref(), Some("1.0.0"));
        assert!(p.parent.is_none());
        assert!(p.modules.is_empty());
    }

    #[test]
    fn parses_parent_block() {
        let p = parse(
            r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <parent>
    <groupId>com.example</groupId>
    <artifactId>parent</artifactId>
    <version>2.0.0</version>
    <relativePath>../pom.xml</relativePath>
  </parent>
  <artifactId>child</artifactId>
</project>"#,
        );
        let parent = p.parent.expect("parent ref");
        assert_eq!(parent.group_id, "com.example");
        assert_eq!(parent.artifact_id, "parent");
        assert_eq!(parent.version, "2.0.0");
    }

    #[test]
    fn parses_modules_aggregator() {
        let p = parse(
            r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>aggregator</artifactId>
  <version>1.0.0</version>
  <packaging>pom</packaging>
  <modules>
    <module>core</module>
    <module>util</module>
  </modules>
</project>"#,
        );
        assert!(p.is_pom_packaging);
        assert_eq!(p.modules, vec!["core", "util"]);
    }

    #[test]
    fn parses_properties() {
        let p = parse(
            r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>lib</artifactId>
  <version>${revision}</version>
  <properties>
    <revision>2.4.1</revision>
    <changelist>-SNAPSHOT</changelist>
  </properties>
</project>"#,
        );
        assert_eq!(
            p.properties.get("revision").map(String::as_str),
            Some("2.4.1")
        );
        assert_eq!(
            p.properties.get("changelist").map(String::as_str),
            Some("-SNAPSHOT")
        );
    }

    #[test]
    fn parses_dependencies_with_versions() {
        let p = parse(
            r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>app</artifactId>
  <version>1.0.0</version>
  <dependencies>
    <dependency>
      <groupId>com.example</groupId>
      <artifactId>lib</artifactId>
      <version>1.0.0</version>
    </dependency>
    <dependency>
      <groupId>org.junit.jupiter</groupId>
      <artifactId>junit-jupiter-api</artifactId>
    </dependency>
  </dependencies>
</project>"#,
        );
        assert_eq!(p.dependencies.len(), 2);
        assert_eq!(p.dependencies[0].artifact_id, "lib");
        assert_eq!(p.dependencies[0].version.as_deref(), Some("1.0.0"));
        assert!(p.dependencies[1].version.is_none());
    }

    fn resolve(content: &str) -> ResolvedPom {
        let p = parse(content);
        let mut coords = HashMap::new();
        if let Some(g) = &p.group_id {
            coords.insert((g.clone(), p.artifact_id.clone()), 0);
        }
        let stack = vec![p];
        resolve_pom(0, &stack, &coords).expect("resolve")
    }

    #[test]
    fn resolves_revision_property() {
        let r = resolve(
            r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>lib</artifactId>
  <version>${revision}</version>
  <properties><revision>2.4.1</revision></properties>
</project>"#,
        );
        assert_eq!(r.version, "2.4.1");
    }

    #[test]
    fn resolves_revision_with_changelist_concatenation() {
        let r = resolve(
            r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>lib</artifactId>
  <version>${revision}${sha1}${changelist}</version>
  <properties>
    <revision>1.2.3</revision>
    <sha1></sha1>
    <changelist>-SNAPSHOT</changelist>
  </properties>
</project>"#,
        );
        assert_eq!(r.version, "1.2.3-SNAPSHOT");
    }

    #[test]
    fn resolves_sha1_property() {
        let r = resolve(
            r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>lib</artifactId>
  <version>${sha1}</version>
  <properties><sha1>3.1.0</sha1></properties>
</project>"#,
        );
        assert_eq!(r.version, "3.1.0");
    }

    #[test]
    fn resolves_changelist_property() {
        let r = resolve(
            r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>lib</artifactId>
  <version>${changelist}</version>
  <properties><changelist>4.5.6</changelist></properties>
</project>"#,
        );
        assert_eq!(r.version, "4.5.6");
    }

    #[test]
    fn project_version_self_ref_in_top_level_version_does_not_converge() {
        // `${project.version}` is meaningful in inter-module deps where
        // it expands to the depending project's own version. Used as the
        // top-level `<version>` field it self-references and the resolver
        // can't converge — this asserts the documented failure mode.
        let p = parse(
            r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>lib</artifactId>
  <version>${project.version}</version>
</project>"#,
        );
        let mut coords = HashMap::new();
        coords.insert((p.group_id.clone().unwrap(), p.artifact_id.clone()), 0);
        let stack = vec![p];
        let err = resolve_pom(0, &stack, &coords).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("did not converge"),
            "expected non-convergence message; got: {msg}"
        );
    }

    #[test]
    fn rejects_unsupported_property() {
        let p = parse(
            r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>lib</artifactId>
  <version>${customVer}</version>
  <properties><customVer>1.0.0</customVer></properties>
</project>"#,
        );
        let mut coords = HashMap::new();
        coords.insert((p.group_id.clone().unwrap(), p.artifact_id.clone()), 0);
        let stack = vec![p];
        let err = resolve_pom(0, &stack, &coords).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("unsupported property"),
            "expected unsupported-property message, got: {msg}"
        );
        assert!(
            msg.contains("revision"),
            "error must list supported properties incl. `revision`; got: {msg}"
        );
    }

    #[test]
    fn detects_parent_cycle_via_tarjan() {
        let a = parse(
            r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>org</groupId>
  <artifactId>a</artifactId>
  <version>1.0.0</version>
  <parent>
    <groupId>org</groupId>
    <artifactId>b</artifactId>
    <version>1.0.0</version>
  </parent>
</project>"#,
        );
        let b = parse(
            r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>org</groupId>
  <artifactId>b</artifactId>
  <version>1.0.0</version>
  <parent>
    <groupId>org</groupId>
    <artifactId>a</artifactId>
    <version>1.0.0</version>
  </parent>
</project>"#,
        );
        let mut coords = HashMap::new();
        coords.insert(("org".to_string(), "a".to_string()), 0);
        coords.insert(("org".to_string(), "b".to_string()), 1);
        let stack = vec![a, b];
        let err = detect_parent_cycles(&stack, &coords).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("cycle"), "expected cycle error, got: {msg}");
        assert!(msg.contains("org:a"));
        assert!(msg.contains("org:b"));
    }

    #[test]
    fn rewrites_top_level_version_only() {
        let original = r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>lib</artifactId>
  <version>1.0.0</version>
  <dependencies>
    <dependency>
      <groupId>com.other</groupId>
      <artifactId>thing</artifactId>
      <version>9.9.9</version>
    </dependency>
  </dependencies>
</project>"#;
        let no_lookup = |_: &str, _: &str| -> Option<String> { None };
        let rewritten = rewrite_pom(original, "2.0.0", &no_lookup).expect("rewrite");
        // Top-level project version replaced.
        assert!(
            rewritten.contains("<version>2.0.0</version>"),
            "expected new top-level version 2.0.0; got:\n{rewritten}"
        );
        // External dep version untouched (lookup returned None).
        assert!(
            rewritten.contains("<version>9.9.9</version>"),
            "external dep version must not be rewritten; got:\n{rewritten}"
        );
        assert_eq!(
            rewritten.matches("<version>").count(),
            2,
            "expected exactly two <version> elements (project + dep); got:\n{rewritten}"
        );
    }

    #[test]
    fn rewrite_preserves_comments_and_whitespace() {
        let original = r#"<?xml version="1.0"?>
<!-- top-level comment -->
<project>
  <modelVersion>4.0.0</modelVersion>
  <!-- coordinates -->
  <groupId>com.example</groupId>
  <artifactId>lib</artifactId>
  <version>1.0.0</version>
</project>
"#;
        let no_lookup = |_: &str, _: &str| -> Option<String> { None };
        let rewritten = rewrite_pom(original, "1.0.1", &no_lookup).expect("rewrite");
        assert!(rewritten.contains("<!-- top-level comment -->"));
        assert!(rewritten.contains("<!-- coordinates -->"));
        assert!(rewritten.contains("<version>1.0.1</version>"));
    }

    /// Plan §12 / Gap #1. When a sibling project bumps, its <parent>
    /// reference in a child POM must update to the new version, otherwise
    /// the child's build is broken on next mvn invocation.
    #[test]
    fn rewrites_parent_version_when_lookup_resolves() {
        let child = r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <parent>
    <groupId>com.example</groupId>
    <artifactId>parent</artifactId>
    <version>1.0.0</version>
  </parent>
  <artifactId>child</artifactId>
</project>"#;

        let lookup = |g: &str, a: &str| -> Option<String> {
            if g == "com.example" && a == "parent" {
                Some("1.1.0".to_string())
            } else {
                None
            }
        };
        // Child's own version inherits from parent (no project/version
        // tag in this POM); we still need a `top_level_version` for the
        // signature, but since `project/version` is absent it never gets
        // applied.
        let rewritten = rewrite_pom(child, "1.1.0", &lookup).expect("rewrite");
        assert!(
            rewritten.contains("<version>1.1.0</version>"),
            "<parent> version should now be 1.1.0; got:\n{rewritten}"
        );
        assert!(
            !rewritten.contains("<version>1.0.0</version>"),
            "<parent> version 1.0.0 should be gone; got:\n{rewritten}"
        );
    }

    /// Inter-module dep with explicit version: when the target sibling
    /// is bumped, the dep's <version> must follow.
    #[test]
    fn rewrites_dependency_version_when_lookup_resolves() {
        let pom = r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>app</artifactId>
  <version>1.0.0</version>
  <dependencies>
    <dependency>
      <groupId>com.example</groupId>
      <artifactId>lib</artifactId>
      <version>1.0.0</version>
    </dependency>
    <dependency>
      <groupId>org.junit.jupiter</groupId>
      <artifactId>junit-jupiter-api</artifactId>
      <version>5.10.0</version>
    </dependency>
  </dependencies>
</project>"#;

        // Lookup only resolves the sibling, not the third-party JUnit dep.
        let lookup = |g: &str, a: &str| -> Option<String> {
            if g == "com.example" && a == "lib" {
                Some("2.0.0".to_string())
            } else {
                None
            }
        };
        let rewritten = rewrite_pom(pom, "1.1.0", &lookup).expect("rewrite");

        // Top-level was bumped to 1.1.0.
        assert!(
            rewritten.contains("<artifactId>app</artifactId>\n  <version>1.1.0</version>"),
            "top-level project version should be 1.1.0; got:\n{rewritten}"
        );
        // Sibling dep now 2.0.0.
        let lib_block_idx = rewritten.find("<artifactId>lib</artifactId>").unwrap();
        let after_lib = &rewritten[lib_block_idx..];
        assert!(
            after_lib.contains("<version>2.0.0</version>"),
            "sibling dep `lib` should be 2.0.0; got after lib block:\n{after_lib}"
        );
        // Third-party JUnit dep untouched.
        assert!(
            rewritten.contains("<version>5.10.0</version>"),
            "external JUnit dep version must be preserved; got:\n{rewritten}"
        );
    }

    /// dependencyManagement entries follow the same rule — a sibling
    /// dep declared there gets the same version-bump treatment.
    #[test]
    fn rewrites_dependency_management_version() {
        let pom = r#"<?xml version="1.0"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>parent</artifactId>
  <version>1.0.0</version>
  <packaging>pom</packaging>
  <dependencyManagement>
    <dependencies>
      <dependency>
        <groupId>com.example</groupId>
        <artifactId>lib</artifactId>
        <version>1.0.0</version>
      </dependency>
    </dependencies>
  </dependencyManagement>
</project>"#;

        let lookup = |g: &str, a: &str| -> Option<String> {
            if g == "com.example" && a == "lib" {
                Some("2.0.0".to_string())
            } else {
                None
            }
        };
        let rewritten = rewrite_pom(pom, "1.1.0", &lookup).expect("rewrite");
        let lib_block_idx = rewritten.find("<artifactId>lib</artifactId>").unwrap();
        let after_lib = &rewritten[lib_block_idx..];
        assert!(
            after_lib.contains("<version>2.0.0</version>"),
            "dependencyManagement entry should be 2.0.0; got:\n{after_lib}"
        );
    }
}
