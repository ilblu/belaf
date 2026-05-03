//! Streaming POM rewriter. Rewrites the top-level `<version>`, plus
//! any `<parent><version>` and `<dependencies>` /
//! `<dependencyManagement>` member versions whose `groupId:artifactId`
//! resolves to a sibling project that's also being bumped. Preserves
//! every other byte (comments, whitespace, namespaces, unrelated tags).

use std::{
    fs::File,
    io::{Cursor, Read, Write},
};

use anyhow::{anyhow, Context as _};
use quick_xml::{
    events::{BytesText, Event},
    Reader, Writer,
};

use crate::{
    atry,
    core::{
        errors::Result,
        git::repository::{ChangeList, RepoPathBuf},
        resolved_release_unit::ReleaseUnitId,
        rewriters::Rewriter,
        session::AppSession,
    },
};

use super::pom_parser::{local_name, path};

#[derive(Debug)]
pub struct MavenRewriter {
    unit_id: ReleaseUnitId,
    pom_path: RepoPathBuf,
}

impl MavenRewriter {
    pub fn new(unit_id: ReleaseUnitId, pom_path: RepoPathBuf) -> Self {
        Self { unit_id, pom_path }
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

        let unit = app.graph().lookup(self.unit_id);
        let new_version = unit.version.to_string();

        let graph = app.graph();
        let coord_lookup = |group_id: &str, artifact_id: &str| -> Option<String> {
            let user_name = format!("{group_id}:{artifact_id}");
            let pid = graph.lookup_ident(&user_name)?;
            if pid == self.unit_id {
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

pub(super) fn rewrite_pom<F>(
    content: &str,
    top_level_version: &str,
    coord_lookup: &F,
) -> Result<String>
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
    let mut buffered: Vec<Event<'static>> = Vec::new();
    let mut buffered_scope: Option<String> = None;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref());
                stack.push(name.clone());
                let p = path(&stack);

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
                        .write_event(other.clone())
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

/// Apply the inter-project version-rewrite logic to one buffered
/// `<parent>` or `<dependency>` block. Returns the events to emit —
/// either the buffer as-is (when the block doesn't reference a sibling
/// project, e.g. `org.junit.jupiter:junit-jupiter-api`) or with the
/// `<version>` text replaced.
fn rewrite_buffered_block<F>(
    buffered: &[Event<'static>],
    coord_lookup: &F,
) -> Result<Vec<Event<'static>>>
where
    F: Fn(&str, &str) -> Option<String>,
{
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
        return Ok(buffered.to_vec());
    };

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
                } else {
                    out.push(Event::Text(t.clone()).into_owned());
                }
            }
            other => out.push(other.clone()),
        }
    }

    Ok(out)
}
