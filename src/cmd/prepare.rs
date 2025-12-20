use anyhow::{Context, Result};
use owo_colors::OwoColorize;
use tracing::{info, warn};

use crate::{
    atry,
    core::{
        bump,
        ecosystem::types::EcosystemType,
        graph::GraphQueryBuilder,
        session::AppSession,
        workflow::{create_release_branch, ReleasePipeline, SelectedProject},
    },
};

#[path = "prepare/wizard.rs"]
mod wizard;

fn print_no_changes_message() {
    println!();
    println!(
        "{} No projects with unreleased changes found.",
        "ℹ".cyan().bold()
    );
    println!();
    println!(
        "  {} All projects are up-to-date with their latest release tags.",
        "→".dimmed()
    );
    println!(
        "  {} Make commits with conventional format (feat:, fix:, etc.) to trigger a release.",
        "→".dimmed()
    );
    println!();
}

pub fn run(
    bump: Option<String>,
    no_tui: bool,
    ci: bool,
    project: Option<Vec<String>>,
) -> Result<i32> {
    info!(
        "preparing release with belaf version {}",
        env!("CARGO_PKG_VERSION")
    );

    if ci {
        return run_ci_mode();
    }

    if let Some(ref projects) = project {
        return run_per_project_mode(projects);
    }

    let use_auto_mode = no_tui || bump.as_deref() == Some("auto");

    if use_auto_mode {
        return run_auto_mode(bump);
    }

    if bump.is_none() || bump.as_deref() == Some("manual") {
        return run_tui_wizard();
    }

    run_simple_bump_mode(bump)
}

fn run_ci_mode() -> Result<i32> {
    info!("running in CI mode (PR-based workflow)");

    let mut sess = atry!(
        AppSession::initialize_default();
        ["could not initialize app and project graph"]
    );

    if let Some(dirty) = atry!(
        sess.repo.check_if_dirty(&[]);
        ["failed to check repository for modified files"]
    ) {
        return Err(anyhow::anyhow!(
            "CI mode requires a clean working directory. Found uncommitted changes: {}",
            dirty.escaped()
        ));
    }

    let (base_branch, release_branch) = create_release_branch(&mut sess)?;

    let projects = discover_and_prepare_projects(&mut sess)?;

    if projects.is_empty() {
        print_no_changes_message();
        return Ok(0);
    }

    let pipeline = ReleasePipeline::new(&mut sess, base_branch, release_branch)?;
    pipeline.execute(projects)?;

    Ok(0)
}

fn discover_and_prepare_projects(sess: &mut AppSession) -> Result<Vec<SelectedProject>> {
    let q = GraphQueryBuilder::default();
    let idents = sess.graph().query(q).context("could not select projects")?;

    if idents.is_empty() {
        info!("no projects found in repository");
        return Ok(Vec::new());
    }

    let histories = atry!(
        sess.analyze_histories();
        ["failed to analyze project histories"]
    );

    let mut prepared_projects: Vec<SelectedProject> = Vec::new();

    for ident in &idents {
        let proj = sess.graph().lookup(*ident);
        let history = histories.lookup(*ident);
        let n_commits = history.n_commits();

        if n_commits == 0 {
            info!(
                "{}: no changes since last release, skipping",
                proj.user_facing_name
            );
            continue;
        }

        let commit_messages: Vec<String> = history
            .commits()
            .into_iter()
            .filter_map(|cid| sess.repo.get_commit_summary(*cid).ok())
            .collect();

        let analysis = atry!(
            bump::analyze_commit_messages(&commit_messages);
            ["failed to analyze commit messages for {}", proj.user_facing_name]
        );

        info!("{}: {}", proj.user_facing_name, analysis.summary());

        let bump_scheme_text = analysis.recommendation.as_str();

        if bump_scheme_text == "no bump" {
            info!(
                "{}: no version bump needed based on conventional commits",
                proj.user_facing_name
            );
            continue;
        }

        let bump_scheme = proj
            .version
            .parse_bump_scheme(bump_scheme_text)
            .with_context(|| {
                format!(
                    "invalid bump scheme \"{}\" for project {}",
                    bump_scheme_text, proj.user_facing_name
                )
            })?;

        let old_version = proj.version.to_string();
        let prefix = proj.prefix().escaped();

        let qnames = proj.qualified_names();
        let ecosystem = qnames
            .get(1)
            .and_then(|s| EcosystemType::from_qname(s))
            .unwrap_or(EcosystemType::Cargo);

        let proj_mut = sess.graph_mut().lookup_mut(*ident);

        atry!(
            bump_scheme.apply(&mut proj_mut.version);
            ["failed to apply version bump to {}", proj_mut.user_facing_name]
        );

        let new_version = proj_mut.version.to_string();

        info!(
            "{}: {} -> {} ({})",
            proj_mut.user_facing_name, old_version, new_version, bump_scheme_text
        );

        prepared_projects.push(SelectedProject {
            name: proj_mut.user_facing_name.clone(),
            prefix,
            old_version,
            new_version,
            bump_type: bump_scheme_text.to_string(),
            commit_messages,
            ecosystem,
            cached_changelog: None,
        });
    }

    Ok(prepared_projects)
}

fn run_simple_bump_mode(bump: Option<String>) -> Result<i32> {
    let bump_scheme_text = bump.as_deref().unwrap_or("patch");
    info!("version bump scheme: {}", bump_scheme_text);

    let mut sess = atry!(
        AppSession::initialize_default();
        ["could not initialize app and project graph"]
    );

    if let Some(dirty) = atry!(
        sess.repo.check_if_dirty(&[]);
        ["failed to check repository for modified files"]
    ) {
        warn!(
            "preparing release with uncommitted changes in the repository (e.g.: `{}`)",
            dirty.escaped()
        );
    }

    let q = GraphQueryBuilder::default();
    let idents = sess.graph().query(q).context("could not select projects")?;

    if idents.is_empty() {
        info!("no projects found in repository");
        return Ok(0);
    }

    let histories = atry!(
        sess.analyze_histories();
        ["failed to analyze project histories"]
    );

    let mut n_prepared = 0;
    let mut n_skipped = 0;

    for ident in &idents {
        let proj = sess.graph().lookup(*ident);
        let history = histories.lookup(*ident);
        let n_commits = history.n_commits();

        if n_commits == 0 {
            info!(
                "{}: no changes since last release, skipping",
                proj.user_facing_name
            );
            n_skipped += 1;
            continue;
        }

        let bump_scheme = proj
            .version
            .parse_bump_scheme(bump_scheme_text)
            .with_context(|| {
                format!(
                    "invalid bump scheme \"{}\" for project {}",
                    bump_scheme_text, proj.user_facing_name
                )
            })?;

        let proj_mut = sess.graph_mut().lookup_mut(*ident);
        let old_version = proj_mut.version.clone();

        atry!(
            bump_scheme.apply(&mut proj_mut.version);
            ["failed to apply version bump to {}", proj_mut.user_facing_name]
        );

        info!(
            "{}: {} -> {} ({} commit{})",
            proj_mut.user_facing_name,
            old_version,
            proj_mut.version,
            n_commits,
            if n_commits == 1 { "" } else { "s" }
        );

        n_prepared += 1;
    }

    if n_prepared == 0 {
        print_no_changes_message();
        return Ok(0);
    }

    info!("updating project files with new versions...");

    let changes = atry!(
        sess.rewrite();
        ["failed to update project files"]
    );

    if changes.paths().count() > 0 {
        info!("modified files:");
        for path in changes.paths() {
            info!("  {}", path.escaped());
        }
    }

    info!(
        "prepared {} project{} for release ({} skipped)",
        n_prepared,
        if n_prepared == 1 { "" } else { "s" },
        n_skipped
    );
    info!("review changes and commit when ready");

    Ok(0)
}

fn run_auto_mode(bump: Option<String>) -> Result<i32> {
    info!("running in auto mode (using conventional commits analysis)");

    let mut sess = atry!(
        AppSession::initialize_default();
        ["could not initialize app and project graph"]
    );

    if let Some(dirty) = atry!(
        sess.repo.check_if_dirty(&[]);
        ["failed to check repository for modified files"]
    ) {
        warn!(
            "preparing release with uncommitted changes in the repository (e.g.: `{}`)",
            dirty.escaped()
        );
    }

    let q = GraphQueryBuilder::default();
    let idents = sess.graph().query(q).context("could not select projects")?;

    if idents.is_empty() {
        info!("no projects found in repository");
        return Ok(0);
    }

    let histories = atry!(
        sess.analyze_histories();
        ["failed to analyze project histories"]
    );

    let mut n_prepared = 0;
    let mut n_skipped = 0;

    for ident in &idents {
        let proj = sess.graph().lookup(*ident);
        let history = histories.lookup(*ident);
        let n_commits = history.n_commits();

        if n_commits == 0 {
            info!(
                "{}: no changes since last release, skipping",
                proj.user_facing_name
            );
            n_skipped += 1;
            continue;
        }

        let commit_messages: Vec<String> = history
            .commits()
            .into_iter()
            .filter_map(|cid| sess.repo.get_commit_summary(*cid).ok())
            .collect();

        let analysis = atry!(
            bump::analyze_commit_messages(&commit_messages);
            ["failed to analyze commit messages for {}", proj.user_facing_name]
        );

        info!("{}: {}", proj.user_facing_name, analysis.summary());

        let bump_scheme_text = if let Some(ref explicit_bump) = bump {
            if explicit_bump == "auto" {
                analysis.recommendation.as_str()
            } else {
                explicit_bump.as_str()
            }
        } else {
            analysis.recommendation.as_str()
        };

        if bump_scheme_text == "no bump" {
            info!(
                "{}: no version bump needed based on conventional commits",
                proj.user_facing_name
            );
            n_skipped += 1;
            continue;
        }

        let bump_scheme = proj
            .version
            .parse_bump_scheme(bump_scheme_text)
            .with_context(|| {
                format!(
                    "invalid bump scheme \"{}\" for project {}",
                    bump_scheme_text, proj.user_facing_name
                )
            })?;

        let proj_mut = sess.graph_mut().lookup_mut(*ident);
        let old_version = proj_mut.version.clone();

        atry!(
            bump_scheme.apply(&mut proj_mut.version);
            ["failed to apply version bump to {}", proj_mut.user_facing_name]
        );

        info!(
            "{}: {} -> {} ({} commit{})",
            proj_mut.user_facing_name,
            old_version,
            proj_mut.version,
            n_commits,
            if n_commits == 1 { "" } else { "s" }
        );

        n_prepared += 1;
    }

    if n_prepared == 0 {
        print_no_changes_message();
        return Ok(0);
    }

    info!("updating project files with new versions...");

    let changes = atry!(
        sess.rewrite();
        ["failed to update project files"]
    );

    if changes.paths().count() > 0 {
        info!("modified files:");
        for path in changes.paths() {
            info!("  {}", path.escaped());
        }
    }

    info!(
        "prepared {} project{} for release ({} skipped)",
        n_prepared,
        if n_prepared == 1 { "" } else { "s" },
        n_skipped
    );
    info!("review changes and commit when ready");

    Ok(0)
}

fn run_per_project_mode(projects: &[String]) -> Result<i32> {
    use std::collections::HashMap;

    info!("running in per-project mode");

    const VALID_BUMP_TYPES: &[&str] = &["major", "minor", "patch"];

    let mut bump_specs: HashMap<String, String> = HashMap::new();
    for spec in projects {
        let parts: Vec<&str> = spec.split(':').collect();
        if parts.len() != 2 {
            return Err(anyhow::anyhow!(
                "invalid project spec '{}': expected format 'project:bump' (e.g., gate:major)",
                spec
            ));
        }
        let bump_type = parts[1];
        if !VALID_BUMP_TYPES.contains(&bump_type) {
            return Err(anyhow::anyhow!(
                "invalid bump type '{}' in spec '{}': expected one of {:?}",
                bump_type,
                spec,
                VALID_BUMP_TYPES
            ));
        }
        bump_specs.insert(parts[0].to_string(), bump_type.to_string());
    }

    let mut sess = atry!(
        AppSession::initialize_default();
        ["could not initialize app and project graph"]
    );

    if let Some(dirty) = atry!(
        sess.repo.check_if_dirty(&[]);
        ["failed to check repository for modified files"]
    ) {
        warn!(
            "preparing release with uncommitted changes in the repository (e.g.: `{}`)",
            dirty.escaped()
        );
    }

    let q = GraphQueryBuilder::default();
    let idents = sess.graph().query(q).context("could not select projects")?;

    if idents.is_empty() {
        info!("no projects found in repository");
        return Ok(0);
    }

    let histories = atry!(
        sess.analyze_histories();
        ["failed to analyze project histories"]
    );

    let mut n_prepared = 0;
    let mut n_skipped = 0;

    for ident in &idents {
        let proj = sess.graph().lookup(*ident);
        let history = histories.lookup(*ident);
        let n_commits = history.n_commits();

        let bump_scheme_text = match bump_specs.get(&proj.user_facing_name) {
            Some(bump) => bump.as_str(),
            None => {
                if n_commits == 0 {
                    info!(
                        "{}: no changes and no explicit bump, skipping",
                        proj.user_facing_name
                    );
                } else {
                    info!(
                        "{}: no explicit bump specified, skipping ({} commit{})",
                        proj.user_facing_name,
                        n_commits,
                        if n_commits == 1 { "" } else { "s" }
                    );
                }
                n_skipped += 1;
                continue;
            }
        };

        let bump_scheme = proj
            .version
            .parse_bump_scheme(bump_scheme_text)
            .with_context(|| {
                format!(
                    "invalid bump scheme \"{}\" for project {}",
                    bump_scheme_text, proj.user_facing_name
                )
            })?;

        let proj_mut = sess.graph_mut().lookup_mut(*ident);
        let old_version = proj_mut.version.clone();

        atry!(
            bump_scheme.apply(&mut proj_mut.version);
            ["failed to apply version bump to {}", proj_mut.user_facing_name]
        );

        info!(
            "{}: {} -> {} ({})",
            proj_mut.user_facing_name, old_version, proj_mut.version, bump_scheme_text
        );

        n_prepared += 1;
    }

    if n_prepared == 0 {
        info!("no projects matched the specified bumps");
        return Ok(0);
    }

    info!("updating project files with new versions...");

    let changes = atry!(
        sess.rewrite();
        ["failed to update project files"]
    );

    if changes.paths().count() > 0 {
        info!("modified files:");
        for path in changes.paths() {
            info!("  {}", path.escaped());
        }
    }

    info!(
        "prepared {} project{} for release ({} skipped)",
        n_prepared,
        if n_prepared == 1 { "" } else { "s" },
        n_skipped
    );
    info!("review changes and commit when ready");

    Ok(0)
}

fn run_tui_wizard() -> Result<i32> {
    wizard::run()
}
