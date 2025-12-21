use anyhow::{Context, Result};
use owo_colors::OwoColorize;
use tracing::info;

use crate::core::{
    session::AppSession,
    workflow::{BumpChoice, PrepareContext, ProjectSelection},
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
    use crate::core::ui::utils::is_interactive_terminal;
    use anyhow::bail;

    info!(
        "preparing release with belaf version {}",
        env!("CARGO_PKG_VERSION")
    );

    if ci {
        return run_ci_mode(project_overrides);
    }

    if !is_interactive_terminal() {
        bail!(
            "Error: No interactive terminal detected.\n\n\
            prepare requires either:\n\
            • Interactive terminal (TTY) for the release wizard\n\
            • --ci flag for full automation (bump, changelog, push, PR)\n\n\
            Hint: belaf prepare --ci"
        );
    }

    run_interactive_mode(project_overrides)
}

fn run_ci_mode(project_overrides: Option<Vec<String>>) -> Result<i32> {
    info!("running in CI mode (PR-based workflow)");

    let mut sess =
        AppSession::initialize_default().context("could not initialize app and project graph")?;

    let mut ctx = PrepareContext::initialize(&mut sess, false)?;
    ctx.discover_projects()?;

    if !ctx.has_candidates() {
        ctx.cleanup();
        print_no_changes_message();
        return Ok(0);
    }

    let mut selections: Vec<ProjectSelection> = ctx
        .candidates
        .iter()
        .cloned()
        .map(|candidate| ProjectSelection {
            candidate,
            bump_choice: BumpChoice::Auto,
            cached_changelog: None,
        })
        .collect();

    if let Some(overrides) = project_overrides {
        apply_project_overrides(&mut selections, &overrides)?;
    }

    let has_actionable_bumps = selections.iter().any(|s| {
        let bump_text = s.bump_choice.resolve(s.candidate.suggested_bump);
        bump_text != "no bump"
    });

    if !has_actionable_bumps {
        ctx.cleanup();
        print_no_changes_message();
        return Ok(0);
    }

    ctx.finalize(selections)?;

    Ok(0)
}

fn run_interactive_mode(project_overrides: Option<Vec<String>>) -> Result<i32> {
    wizard::run_with_overrides(project_overrides)
}

fn apply_project_overrides(
    selections: &mut [ProjectSelection],
    overrides: &[String],
) -> Result<()> {
    let project_names: Vec<String> = selections
        .iter()
        .map(|s| s.candidate.name.clone())
        .collect();

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

        if let Some(selection) = selections
            .iter_mut()
            .find(|s| s.candidate.name == project_name)
        {
            selection.bump_choice = match bump_type {
                "major" => BumpChoice::Major,
                "minor" => BumpChoice::Minor,
                "patch" => BumpChoice::Patch,
                _ => unreachable!(),
            };
            info!(
                "override: {} -> {} (was: {})",
                project_name,
                bump_type,
                selection.candidate.suggested_bump.as_str()
            );
        }
    }

    Ok(())
}
