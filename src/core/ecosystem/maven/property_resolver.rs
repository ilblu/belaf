//! Inheritance + property substitution for parsed POMs.
//!
//! Walks the `<parent>` chain to inherit `groupId`, `version`, and
//! `<properties>`, then substitutes the Maven CI-friendly property
//! set (`${revision}`, `${sha1}`, `${changelist}`, `${project.version}`)
//! into version fields. Unsupported property names are a hard error.

use std::collections::{HashMap, HashSet};

use anyhow::anyhow;

use crate::core::errors::Result;

use super::pom_parser::ParsedPom;

/// Names of properties that are allowed to appear in a `<version>` element.
/// These are the Maven "CI friendly" set
/// (<https://maven.apache.org/guides/mini/guide-maven-ci-friendly.html>) plus
/// `project.version` for inter-module self-reference.
pub(super) const SUPPORTED_PROPERTIES: &[&str] =
    &["revision", "sha1", "changelist", "project.version"];

#[derive(Debug, Clone)]
pub(super) struct ResolvedPom {
    pub(super) group_id: String,
    pub(super) artifact_id: String,
    pub(super) version: String,
}

pub(super) fn resolve_pom(
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

pub(super) fn resolve_property(
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
            "project.version" => pom.version.clone().unwrap_or_default(),
            other => props.get(other).cloned().ok_or_else(|| {
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
