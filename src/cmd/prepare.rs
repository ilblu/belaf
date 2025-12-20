use anyhow::{Context, Result};
use owo_colors::OwoColorize;
use tracing::info;

use crate::{
    atry,
    core::{
        bump,
        ecosystem::types::EcosystemType,
        graph::GraphQueryBuilder,
        session::AppSession,
        version::VersionBumpScheme,
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

pub fn run(ci: bool, project_overrides: Option<Vec<String>>) -> Result<i32> {
    info!(
        "preparing release with belaf version {}",
        env!("CARGO_PKG_VERSION")
    );

    if ci {
        run_ci_mode(project_overrides)
    } else {
        run_interactive_mode(project_overrides)
    }
}

fn run_ci_mode(project_overrides: Option<Vec<String>>) -> Result<i32> {
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

    let mut projects = discover_and_prepare_projects(&mut sess)?;

    if let Some(overrides) = project_overrides {
        apply_project_overrides(&mut sess, &mut projects, &overrides)?;
    }

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

fn run_interactive_mode(project_overrides: Option<Vec<String>>) -> Result<i32> {
    wizard::run_with_overrides(project_overrides)
}

fn apply_project_overrides(
    sess: &mut AppSession,
    projects: &mut [SelectedProject],
    overrides: &[String],
) -> Result<()> {
    let project_names: Vec<String> = projects.iter().map(|p| p.name.clone()).collect();

    for override_str in overrides {
        let parts: Vec<&str> = override_str.splitn(2, ':').collect();
        if parts.len() != 2 {
            return Err(anyhow::anyhow!(
                "Invalid project override format '{}'. Expected 'project:bump' (e.g., 'gate:major')",
                override_str
            ));
        }

        let project_name = parts[0];
        let bump_type = parts[1];

        if !project_names.iter().any(|n| n == project_name) {
            return Err(anyhow::anyhow!(
                "Unknown project '{}'. Available: {}",
                project_name,
                project_names.join(", ")
            ));
        }

        let valid_bumps = ["major", "minor", "patch"];
        if !valid_bumps.contains(&bump_type) {
            return Err(anyhow::anyhow!(
                "Invalid bump type '{}' for project '{}'. Valid: major, minor, patch",
                bump_type,
                project_name
            ));
        }

        if let Some(project) = projects.iter_mut().find(|p| p.name == project_name) {
            let old_ver: semver::Version = project.old_version.parse().map_err(|e| {
                anyhow::anyhow!("Invalid version '{}' for project '{}': {}", project.old_version, project_name, e)
            })?;
            let new_version = match bump_type {
                "major" => semver::Version::new(old_ver.major + 1, 0, 0),
                "minor" => semver::Version::new(old_ver.major, old_ver.minor + 1, 0),
                "patch" => semver::Version::new(old_ver.major, old_ver.minor, old_ver.patch + 1),
                _ => unreachable!(),
            };
            info!(
                "override: {} {} -> {} ({})",
                project_name, project.old_version, new_version, bump_type
            );
            project.new_version = new_version.to_string();
            project.bump_type = bump_type.to_string();

            let ident = sess
                .graph()
                .lookup_ident(&project.name)
                .with_context(|| format!("project '{}' not found in graph", project_name))?;
            let proj_mut = sess.graph_mut().lookup_mut(ident);
            let force_scheme = VersionBumpScheme::Force(new_version.to_string());
            force_scheme
                .apply(&mut proj_mut.version)
                .with_context(|| format!("failed to set version for project '{}'", project_name))?;
        }
    }

    Ok(())
}
