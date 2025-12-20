use anyhow::Result;
use serde::Serialize;
use std::io::Write;

use crate::{
    atry,
    core::{graph::GraphQueryBuilder, session::AppSession},
};

const HTML_TEMPLATE: &str = include_str!("templates/graph.html");

#[derive(Serialize)]
struct GraphData {
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
}

#[derive(Serialize)]
struct GraphNode {
    id: String,
    label: String,
    #[serde(rename = "type")]
    node_type: String,
    version: String,
    deps_count: usize,
}

#[derive(Serialize)]
struct GraphEdge {
    source: String,
    target: String,
}

pub fn open_browser(output_path: Option<&str>) -> Result<i32> {
    let sess = atry!(
        AppSession::initialize_default();
        ["could not initialize app and project graph"]
    );

    let q = GraphQueryBuilder::default();
    let idents = sess
        .graph()
        .query(q)
        .map_err(|e| anyhow::anyhow!("could not select projects: {}", e))?;

    if idents.is_empty() {
        println!("No projects found in repository");
        return Ok(0);
    }

    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    for &ident in &idents {
        let proj = sess.graph().lookup(ident);
        let has_deps = !proj.internal_deps.is_empty();

        nodes.push(GraphNode {
            id: proj.user_facing_name.clone(),
            label: proj.user_facing_name.clone(),
            node_type: if has_deps {
                "app".to_string()
            } else {
                "package".to_string()
            },
            version: proj.version.to_string(),
            deps_count: proj.internal_deps.len(),
        });

        for dep in &proj.internal_deps {
            let dep_proj = sess.graph().lookup(dep.ident);
            edges.push(GraphEdge {
                source: proj.user_facing_name.clone(),
                target: dep_proj.user_facing_name.clone(),
            });
        }
    }

    let graph_data = GraphData { nodes, edges };
    let json_data = serde_json::to_string(&graph_data)?;

    let html_content = HTML_TEMPLATE.replace("/*GRAPH_DATA_PLACEHOLDER*/", &json_data);

    let output_file = if let Some(path) = output_path {
        std::path::PathBuf::from(path)
    } else {
        let mut temp = std::env::temp_dir();
        temp.push("belaf-dependency-graph.html");
        temp
    };

    let mut file = std::fs::File::create(&output_file)?;
    file.write_all(html_content.as_bytes())?;

    if output_path.is_some() {
        println!("Graph saved to: {}", output_file.display());
    } else {
        println!("Opening dependency graph in browser...");
        open::that(&output_file)?;
    }

    Ok(0)
}
