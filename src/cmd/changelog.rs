use anyhow::{Context, Result};
use owo_colors::OwoColorize;
use tracing::info;

use crate::core::{
    bump::{self, BumpConfig},
    changelog::{ChangelogConfig, Commit, GitConfig},
    ecosystem::types::EcosystemType,
    graph::GraphQueryBuilder,
    session::AppSession,
    workflow::{generate_and_write_project_changelog, ChangelogGenerationParams},
};

pub fn run(
    preview: bool,
    stdout: bool,
    project_filter: Option<String>,
    output_path: Option<String>,
    unreleased: bool,
) -> Result<i32> {
    info!(
        "generating changelog with belaf version {}",
        env!("CARGO_PKG_VERSION")
    );

    let sess =
        AppSession::initialize_default().context("could not initialize app and project graph")?;

    let q = GraphQueryBuilder::default();
    let idents = sess.graph().query(q).context("could not query projects")?;

    if idents.is_empty() {
        println!("{} No projects found in repository.", "ℹ".cyan().bold());
        return Ok(0);
    }

    let histories = sess
        .analyze_histories()
        .context("failed to analyze project histories")?;

    let git_config = GitConfig::from_user_config(&sess.changelog_config);
    let changelog_config = ChangelogConfig::from_user_config(&sess.changelog_config);
    let bump_config = BumpConfig::from_user_config(&sess.bump_config);

    let mut processed_count = 0;

    for ident in &idents {
        let proj = sess.graph().lookup(*ident);

        if let Some(ref filter) = project_filter {
            if proj.user_facing_name != *filter {
                continue;
            }
        }

        let history = histories.lookup(*ident);
        let n_commits = history.n_commits();

        if n_commits == 0 {
            info!(
                "{}: no changes since last release, skipping",
                proj.user_facing_name
            );
            continue;
        }

        let commits: Vec<Commit<'static>> = history
            .commits()
            .into_iter()
            .filter_map(|cid| {
                sess.repo.get_commit_summary(*cid).ok().map(|msg| Commit {
                    id: cid.to_string(),
                    message: msg,
                    ..Default::default()
                })
            })
            .collect();

        if commits.is_empty() {
            continue;
        }

        let current_version = proj.version.to_string();
        let analysis = bump::analyze_commits(&commits)
            .with_context(|| format!("failed to analyze commits for {}", proj.user_facing_name))?;

        let suggested_bump = analysis
            .recommendation
            .apply_config(&bump_config, Some(&current_version));

        let new_version = if unreleased || suggested_bump.as_str() == "no bump" {
            None
        } else {
            let mut version_clone = proj.version.clone();
            let bump_scheme = version_clone
                .parse_bump_scheme(suggested_bump.as_str())
                .with_context(|| {
                    format!("invalid bump scheme for project {}", proj.user_facing_name)
                })?;
            bump_scheme.apply(&mut version_clone).with_context(|| {
                format!("failed to apply version bump to {}", proj.user_facing_name)
            })?;
            Some(version_clone.to_string())
        };

        let qnames = proj.qualified_names();
        let ecosystem = qnames
            .get(1)
            .and_then(|s| EcosystemType::from_qname(s))
            .unwrap_or(EcosystemType::Cargo);

        let prefix = proj.prefix().escaped();
        let write_to_file = !preview && !stdout;

        let params = ChangelogGenerationParams {
            repo: &sess.repo,
            project_name: &proj.user_facing_name,
            prefix: &prefix,
            version: new_version.as_deref(),
            commits: &commits,
            git_config: &git_config,
            changelog_config: &changelog_config,
            bump_config: &bump_config,
            write_to_file,
            custom_output_path: output_path.as_deref(),
        };
        let result = generate_and_write_project_changelog(&params)?;

        if !result.has_user_changes {
            info!(
                "{}: no user-facing changes, skipping",
                proj.user_facing_name
            );
            continue;
        }

        if preview {
            print_changelog_preview(
                &proj.user_facing_name,
                &current_version,
                new_version.as_deref(),
                ecosystem,
                &result.content,
            );
        } else if stdout {
            print!("{}", result.content);
        } else {
            let version_info = match new_version.as_deref() {
                Some(nv) => format!("{} → {}", current_version.dimmed(), nv.green()),
                None => format!("{} [unreleased]", current_version.dimmed()),
            };

            let path_display = result
                .path
                .as_ref()
                .map(|p| p.escaped())
                .unwrap_or_else(|| "CHANGELOG.md".to_string());

            println!(
                "  {} {} ({}) {} → {}",
                "✓".green(),
                proj.user_facing_name.bold(),
                ecosystem.display_name().dimmed(),
                version_info,
                path_display.dimmed()
            );
        }

        processed_count += 1;
    }

    if processed_count == 0 {
        if project_filter.is_some() {
            println!(
                "{} Project '{}' not found or has no changes.",
                "ℹ".cyan().bold(),
                project_filter.unwrap()
            );
        } else {
            println!(
                "{} No projects with unreleased changes found.",
                "ℹ".cyan().bold()
            );
        }
    } else if !preview && !stdout {
        println!();
        println!(
            "{} Generated changelog for {} project{}.",
            "✓".green().bold(),
            processed_count,
            if processed_count == 1 { "" } else { "s" }
        );
    }

    Ok(0)
}

fn print_changelog_preview(
    project_name: &str,
    current_version: &str,
    new_version: Option<&str>,
    ecosystem: EcosystemType,
    content: &str,
) {
    println!();
    if let Some(nv) = new_version {
        println!(
            "{} {} {} → {} ({})",
            "─".repeat(3).dimmed(),
            project_name.bold().cyan(),
            current_version.dimmed(),
            nv.green(),
            ecosystem.display_name().dimmed()
        );
    } else {
        println!(
            "{} {} {} [unreleased] ({})",
            "─".repeat(3).dimmed(),
            project_name.bold().cyan(),
            current_version.dimmed(),
            ecosystem.display_name().dimmed()
        );
    }
    println!();
    print!("{content}");
}
