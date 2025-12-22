use anyhow::{Context, Result};
use tracing::info;

use crate::{
    atry,
    cli::GraphOutputFormat,
    core::{graph::GraphQueryBuilder, session::AppSession},
};

#[path = "graph/wizard.rs"]
mod wizard;

#[path = "graph/browser.rs"]
mod browser;

pub fn run(
    format: Option<GraphOutputFormat>,
    ci: bool,
    web: bool,
    out: Option<String>,
) -> Result<i32> {
    use crate::core::ui::utils::should_use_tui;

    if web || out.is_some() {
        return browser::open_browser(out.as_deref());
    }

    if should_use_tui(ci, &format) {
        return wizard::run();
    }

    info!(
        "showing dependency graph with belaf version {}",
        env!("CARGO_PKG_VERSION")
    );

    let sess = atry!(
        AppSession::initialize_default();
        ["could not initialize app and project graph"]
    );

    let q = GraphQueryBuilder::default();
    let idents = sess.graph().query(q).context("could not select projects")?;

    if idents.is_empty() {
        println!("No projects found in repository");
        return Ok(0);
    }

    let output_format = if ci {
        GraphOutputFormat::Json
    } else {
        format.unwrap_or(GraphOutputFormat::Ascii)
    };

    match output_format {
        GraphOutputFormat::Ascii => render_ascii(&sess, &idents),
        GraphOutputFormat::Dot => render_dot(&sess, &idents),
        GraphOutputFormat::Json => render_json(&sess, &idents)?,
    }

    Ok(0)
}

fn render_ascii(sess: &AppSession, idents: &[usize]) {
    println!();
    println!("╭─────────────────────────────────────────────────────────╮");
    println!("│              Project Dependency Graph                   │");
    println!("╰─────────────────────────────────────────────────────────╯");
    println!();

    let mut has_deps = false;

    for ident in idents {
        let proj = sess.graph().lookup(*ident);
        let deps = &proj.internal_deps;

        if deps.is_empty() {
            println!("  ○ {} @ {}", proj.user_facing_name, proj.version);
        } else {
            has_deps = true;
            println!("  ● {} @ {}", proj.user_facing_name, proj.version);
            for (i, dep) in deps.iter().enumerate() {
                let dep_proj = sess.graph().lookup(dep.ident);
                let prefix = if i == deps.len() - 1 {
                    "└──"
                } else {
                    "├──"
                };
                println!(
                    "    {} → {} @ {}",
                    prefix, dep_proj.user_facing_name, dep_proj.version
                );
            }
        }
        println!();
    }

    println!("╭─────────────────────────────────────────────────────────╮");
    println!("│  Legend: ○ = no deps  ● = has deps  → = depends on     │");
    println!("╰─────────────────────────────────────────────────────────╯");

    if !has_deps {
        println!();
        println!("  No internal dependencies found between projects.");
    }

    println!();
    println!("  Release order (topological):");
    let toposorted: Vec<_> = sess.graph().toposorted().collect();
    for (i, ident) in toposorted.iter().enumerate() {
        let proj = sess.graph().lookup(*ident);
        println!("    {}. {}", i + 1, proj.user_facing_name);
    }
    println!();
}

fn render_dot(sess: &AppSession, idents: &[usize]) {
    println!("digraph dependencies {{");
    println!("    rankdir=TB;");
    println!("    node [shape=box, style=rounded];");
    println!();

    for ident in idents {
        let proj = sess.graph().lookup(*ident);
        let label = format!("{}\\n{}", proj.user_facing_name, proj.version);
        println!("    \"{}\" [label=\"{}\"];", proj.user_facing_name, label);
    }

    println!();

    for ident in idents {
        let proj = sess.graph().lookup(*ident);
        for dep in &proj.internal_deps {
            let dep_proj = sess.graph().lookup(dep.ident);
            println!(
                "    \"{}\" -> \"{}\";",
                proj.user_facing_name, dep_proj.user_facing_name
            );
        }
    }

    println!("}}");
}

fn render_json(sess: &AppSession, idents: &[usize]) -> Result<()> {
    use serde_json::json;

    let mut projects = Vec::new();

    for ident in idents {
        let proj = sess.graph().lookup(*ident);
        let deps: Vec<String> = proj
            .internal_deps
            .iter()
            .map(|d| {
                let dep_proj = sess.graph().lookup(d.ident);
                dep_proj.user_facing_name.clone()
            })
            .collect();

        projects.push(json!({
            "name": proj.user_facing_name,
            "version": proj.version.to_string(),
            "prefix": proj.prefix().escaped(),
            "dependencies": deps,
        }));
    }

    let toposorted: Vec<String> = sess
        .graph()
        .toposorted()
        .map(|id| sess.graph().lookup(id).user_facing_name.clone())
        .collect();

    let output = json!({
        "projects": projects,
        "release_order": toposorted,
    });

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}
