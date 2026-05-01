//! Streaming `pom.xml` parser. Produces a [`ParsedPom`] of the
//! coordinates, parent reference, properties, modules, and inter-project
//! dependency references — without inheritance or property substitution
//! (those happen in [`super::property_resolver`]).
//!
//! Plus parent-cycle detection via Tarjan-SCC on the parent-graph
//! built from the parsed POMs.

use std::{collections::HashMap, fs::File, io::Read, path::PathBuf};

use anyhow::anyhow;
use petgraph::{algo::tarjan_scc, graph::DiGraph};
use quick_xml::{events::Event, Reader};

use crate::{
    atry,
    core::{
        errors::Result,
        git::repository::{RepoPath, RepoPathBuf},
    },
};

/// One parsed POM, before resolution. Coordinates may still contain
/// `${...}` placeholders at this point.
#[derive(Debug, Clone)]
pub(super) struct ParsedPom {
    pub(super) repo_path: RepoPathBuf,
    pub(super) fs_path: PathBuf,
    pub(super) group_id: Option<String>,
    pub(super) artifact_id: String,
    pub(super) version: Option<String>,
    pub(super) parent: Option<ParentRef>,
    pub(super) properties: HashMap<String, String>,
    pub(super) modules: Vec<String>,
    pub(super) dependencies: Vec<DepRef>,
    /// True if this POM uses `<packaging>pom</packaging>` (typical for
    /// aggregators, but not required — we treat aggregator-ness purely by
    /// `<modules>` presence).
    pub(super) is_pom_packaging: bool,
}

#[derive(Debug, Clone)]
pub(super) struct ParentRef {
    pub(super) group_id: String,
    pub(super) artifact_id: String,
    pub(super) version: String,
}

#[derive(Debug, Clone)]
pub(super) struct DepRef {
    pub(super) group_id: String,
    pub(super) artifact_id: String,
    /// `<version>` is optional in `<dependencies>` when inherited from
    /// `<dependencyManagement>` higher up. We only track inter-project
    /// deps that have a version we can resolve.
    pub(super) version: Option<String>,
}

impl ParsedPom {
    pub(super) fn from_file(repo_path: &RepoPath, fs_path: &std::path::Path) -> Result<Self> {
        let mut content = String::new();
        atry!(
            File::open(fs_path).and_then(|mut f| f.read_to_string(&mut content));
            ["failed to read Maven POM `{}`", fs_path.display()]
        );
        Self::from_str(repo_path, fs_path, &content)
    }

    pub(super) fn from_str(
        repo_path: &RepoPath,
        fs_path: &std::path::Path,
        content: &str,
    ) -> Result<Self> {
        let mut reader = Reader::from_str(content);
        reader.config_mut().trim_text(false);

        let mut buf = Vec::new();
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

pub(super) fn local_name(qname: &[u8]) -> String {
    let s = std::str::from_utf8(qname).unwrap_or_default();
    match s.rsplit_once(':') {
        Some((_, local)) => local.to_string(),
        None => s.to_string(),
    }
}

pub(super) fn path(stack: &[String]) -> Option<String> {
    if stack.is_empty() {
        None
    } else {
        Some(stack.join("/"))
    }
}

pub(super) fn detect_parent_cycles(
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
