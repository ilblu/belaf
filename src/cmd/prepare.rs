use anyhow::Result;
use owo_colors::OwoColorize;
use tracing::info;

use crate::core::{
    bump_source::{self, BumpSourceInput, DEFAULT_TIMEOUT_SEC},
    config::syntax::BumpSourceConfig,
    group::GroupSet,
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

pub fn run(
    ci: bool,
    project_overrides: Option<Vec<String>>,
    bump_source: Option<String>,
    bump_source_cmd: Option<String>,
) -> Result<i32> {
    use crate::core::ui::utils::is_interactive_terminal;
    use anyhow::bail;

    info!(
        "preparing release with belaf version {}",
        env!("CARGO_PKG_VERSION")
    );

    if ci {
        return run_ci_mode(project_overrides, bump_source, bump_source_cmd);
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

    if matches!(bump_source.as_deref(), Some("-")) {
        bail!(
            "Error: `--bump-source -` (stdin) is only supported in --ci mode.\n\n\
             Use `--bump-source <FILE>` in interactive mode, or pass --ci."
        );
    }

    run_interactive_mode(project_overrides, bump_source, bump_source_cmd)
}

fn run_ci_mode(
    project_overrides: Option<Vec<String>>,
    cli_bump_source: Option<String>,
    cli_bump_source_cmd: Option<String>,
) -> Result<i32> {
    info!("running in CI mode (PR-based workflow)");

    let mut sess = AppSession::initialize_default()?;
    if let Err(message) = sess.pre_prepare_drift_check() {
        anyhow::bail!("{}", message);
    }
    let config_bump_sources = sess.config_bump_sources().to_vec();
    // Snapshot groups before ctx takes a mutable borrow on sess. The
    // GroupSet is cloneable and we only read from it during validation, so
    // there's no consistency risk vs. the live graph.
    let groups = sess.graph().groups().clone();

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

    // Precedence: config bump-source defaults → explicit --bump-source* CLI →
    // --project overrides. Later wins, so we apply in that order.
    apply_config_bump_sources(&mut selections, &config_bump_sources)?;
    apply_cli_bump_source(
        &mut selections,
        cli_bump_source.as_deref(),
        cli_bump_source_cmd.as_deref(),
    )?;
    if let Some(overrides) = project_overrides {
        apply_project_overrides(&mut selections, &overrides)?;
    }

    // Group atomicity: every member of a group must end up with the same
    // bump. If two `--project` overrides disagree, or a bump-source decision
    // contradicts a `--project` override on a sibling, fail loudly here
    // instead of silently emitting an inconsistent manifest.
    validate_group_consistency(&selections, &groups)?;

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

fn run_interactive_mode(
    project_overrides: Option<Vec<String>>,
    bump_source: Option<String>,
    bump_source_cmd: Option<String>,
) -> Result<i32> {
    // The interactive wizard owns its own selections state machine; we
    // pre-collect external decisions here and propagate them so the
    // wizard's "suggested bump" column reflects the same precedence as
    // CI mode. See `wizard::run_with_overrides_and_decisions`.
    let sess = AppSession::initialize_default()?;
    if let Err(message) = sess.pre_prepare_drift_check() {
        anyhow::bail!("{}", message);
    }
    let config_bump_sources = sess.config_bump_sources().to_vec();
    drop(sess);

    // Collect external decisions up-front so the wizard sees them.
    let mut decisions = Vec::new();
    decisions.extend(collect_config_decisions(&config_bump_sources)?);
    if let Some(d) = collect_cli_decisions(bump_source.as_deref(), bump_source_cmd.as_deref())? {
        decisions.extend(d);
    }
    wizard::run_with_overrides_and_decisions(project_overrides, decisions)
}

/// Apply `[[bump_source]]` config entries to the selections list. Each
/// entry's stdout is parsed as a list of decisions; matching projects get
/// their `bump_choice` overwritten.
fn apply_config_bump_sources(
    selections: &mut [ProjectSelection],
    sources: &[BumpSourceConfig],
) -> Result<()> {
    let decisions = collect_config_decisions(sources)?;
    apply_decisions(selections, &decisions)
}

fn collect_config_decisions(
    sources: &[BumpSourceConfig],
) -> Result<Vec<bump_source::BumpDecision>> {
    let mut all = Vec::new();
    for src in sources {
        let label = src
            .project
            .as_deref()
            .or(src.group.as_deref())
            .map(|s| s.to_string());
        let input = BumpSourceInput::Command {
            cmd: src.cmd.clone(),
            timeout_sec: src.timeout_sec.unwrap_or(DEFAULT_TIMEOUT_SEC),
            label,
        };
        all.extend(bump_source::collect(&input)?);
    }
    Ok(all)
}

fn apply_cli_bump_source(
    selections: &mut [ProjectSelection],
    bump_source_arg: Option<&str>,
    bump_source_cmd_arg: Option<&str>,
) -> Result<()> {
    let Some(decisions) = collect_cli_decisions(bump_source_arg, bump_source_cmd_arg)? else {
        return Ok(());
    };
    apply_decisions(selections, &decisions)
}

fn collect_cli_decisions(
    bump_source_arg: Option<&str>,
    bump_source_cmd_arg: Option<&str>,
) -> Result<Option<Vec<bump_source::BumpDecision>>> {
    let mut all = Vec::new();
    if let Some(arg) = bump_source_arg {
        let input = if arg == "-" {
            BumpSourceInput::Stdin
        } else {
            BumpSourceInput::File(arg.into())
        };
        all.extend(bump_source::collect(&input)?);
    }
    if let Some(cmd) = bump_source_cmd_arg {
        let input = BumpSourceInput::Command {
            cmd: cmd.to_string(),
            timeout_sec: DEFAULT_TIMEOUT_SEC,
            label: Some("--bump-source-cmd".into()),
        };
        all.extend(bump_source::collect(&input)?);
    }
    if all.is_empty() && bump_source_arg.is_none() && bump_source_cmd_arg.is_none() {
        Ok(None)
    } else {
        Ok(Some(all))
    }
}

/// Enforce that every group's members share one resolved bump. The
/// resolved bump is `bump_choice.resolve(suggested_bump)` — i.e. the same
/// string the manifest will end up carrying. Members in `Auto` mode whose
/// inferred bump differs from a sibling's explicit choice are also a
/// conflict (otherwise the conv-commits inference would silently win).
pub(super) fn validate_group_consistency(
    selections: &[ProjectSelection],
    groups: &GroupSet,
) -> Result<()> {
    if groups.is_empty() {
        return Ok(());
    }
    use std::collections::HashMap;
    // Map: group_id -> Vec<(member_name, resolved_bump)>
    let mut by_group: HashMap<&str, Vec<(&str, &'static str)>> = HashMap::new();
    for sel in selections {
        let Some(g) = groups.group_of(sel.candidate.ident) else {
            continue;
        };
        let resolved = sel.bump_choice.resolve(sel.candidate.suggested_bump);
        by_group
            .entry(g.id.as_str())
            .or_default()
            .push((sel.candidate.name.as_str(), resolved));
    }
    for (group_id, members) in &by_group {
        // "no bump" is the silent default for projects with no commits
        // since the last release. Filter those out so a group of 5 where
        // 3 members are quiescent still validates against the 2 active
        // members' shared bump.
        let active: Vec<_> = members
            .iter()
            .filter(|(_, bump)| *bump != "no bump")
            .copied()
            .collect();
        if active.len() < 2 {
            continue;
        }
        let first_bump = active[0].1;
        if active.iter().any(|(_, b)| *b != first_bump) {
            let detail = active
                .iter()
                .map(|(name, bump)| format!("`{name}` bump={bump}"))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(anyhow::anyhow!(
                "group `{group_id}`: members disagree on bump ({detail}) — \
                 all members of a group must share one bump. Reconcile via \
                 `--project name:bump` for each member, or fix the \
                 [[bump_source]] output."
            ));
        }
    }
    Ok(())
}

fn apply_decisions(
    selections: &mut [ProjectSelection],
    decisions: &[bump_source::BumpDecision],
) -> Result<()> {
    if decisions.is_empty() {
        return Ok(());
    }
    let names: Vec<String> = selections
        .iter()
        .map(|s| s.candidate.name.clone())
        .collect();
    for d in decisions {
        let Some(sel) = selections
            .iter_mut()
            .find(|s| s.candidate.name == d.project)
        else {
            return Err(anyhow::anyhow!(
                "bump-source decision for `{}` does not match any project. \
                 Available: {}",
                d.project,
                names.join(", ")
            ));
        };
        sel.bump_choice = match d.bump.as_str() {
            "major" => BumpChoice::Major,
            "minor" => BumpChoice::Minor,
            "patch" => BumpChoice::Patch,
            other => {
                return Err(anyhow::anyhow!(
                    "bump-source decision for `{}` has invalid `bump` value `{}`",
                    d.project,
                    other
                ));
            }
        };
        let reason = d.reason.as_deref().unwrap_or("");
        let source = d.source.as_deref().unwrap_or("(unspecified)");
        info!(
            "bump-source: {} -> {} (source: {}{})",
            d.project,
            d.bump,
            source,
            if reason.is_empty() {
                String::new()
            } else {
                format!(", reason: {reason}")
            }
        );
    }
    Ok(())
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
