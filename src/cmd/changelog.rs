use anyhow::{Context, Result};
use owo_colors::OwoColorize;
use tracing::info;

use crate::core::{
    bump::{self, BumpConfig},
    changelog::{ChangelogConfig, Commit, GitConfig},
    graph::GraphQueryBuilder,
    session::AppSession,
    wire::known::Ecosystem,
    workflow::{
        extract_github_remote, generate_and_write_project_changelog, load_github_token,
        ChangelogGenerationParams,
    },
};

pub fn run(
    preview: bool,
    stdout: bool,
    project_filter: Option<String>,
    output_path: Option<String>,
    unreleased: bool,
    ci: bool,
) -> Result<i32> {
    if !ci {
        info!(
            "generating changelog with belaf version {}",
            env!("CARGO_PKG_VERSION")
        );
    }

    let sess = AppSession::initialize_default()?;

    let q = GraphQueryBuilder::default();
    let idents = sess.graph().query(q)?;

    if idents.is_empty() {
        if !ci {
            println!("{} No projects found in repository.", "ℹ".cyan().bold());
        }
        return Ok(0);
    }

    let histories = sess
        .analyze_histories()
        .context("failed to analyze project histories")?;

    let git_config = GitConfig::from_user_config(&sess.changelog_config);
    let changelog_config = ChangelogConfig::from_user_config(&sess.changelog_config);
    let bump_config = BumpConfig::from_user_config(&sess.bump_config);

    let github_remote = extract_github_remote(&sess.repo);
    let github_token = load_github_token();

    let mut processed_count = 0;
    // Track per-project outcome for the `--ci` JSON status. Populated
    // alongside the existing `processed_count` so we don't change the
    // happy-path control flow.
    let mut ci_files_written: Vec<String> = Vec::new();
    let mut ci_projects: Vec<String> = Vec::new();

    for ident in &idents {
        let unit = sess.graph().lookup(*ident);

        if let Some(ref filter) = project_filter {
            if unit.user_facing_name != *filter {
                continue;
            }
        }

        let history = histories.lookup(*ident);
        let n_commits = history.n_commits();

        if n_commits == 0 {
            if !ci {
                info!(
                    "{}: no changes since last release, skipping",
                    unit.user_facing_name
                );
            }
            continue;
        }

        let commits: Vec<Commit> = history
            .commits()
            .into_iter()
            .filter_map(|cid| sess.repo.get_commit_details(*cid).ok())
            .collect();

        if commits.is_empty() {
            continue;
        }

        let current_version = unit.version.to_string();
        let analysis = bump::analyze_commits(&commits)
            .with_context(|| format!("failed to analyze commits for {}", unit.user_facing_name))?;

        let suggested_bump = analysis
            .recommendation
            .apply_config(&bump_config, Some(&current_version));

        let new_version = if unreleased || suggested_bump.as_str() == "no bump" {
            None
        } else {
            let mut version_clone = unit.version.clone();
            let bump_scheme = version_clone
                .parse_bump_scheme(suggested_bump.as_str())
                .with_context(|| {
                    format!("invalid bump scheme for project {}", unit.user_facing_name)
                })?;
            bump_scheme.apply(&mut version_clone).with_context(|| {
                format!("failed to apply version bump to {}", unit.user_facing_name)
            })?;
            Some(version_clone.to_string())
        };

        let qnames = unit.qualified_names();
        let ecosystem = qnames
            .get(1)
            .map(|s| Ecosystem::classify(s))
            .unwrap_or_else(|| Ecosystem::classify("cargo"));

        let prefix = unit.prefix().escaped();
        let write_to_file = !preview && !stdout;

        let params = ChangelogGenerationParams {
            repo: &sess.repo,
            project_name: &unit.user_facing_name,
            prefix: &prefix,
            version: new_version.as_deref(),
            commits: &commits,
            git_config: &git_config,
            changelog_config: &changelog_config,
            bump_config: &bump_config,
            write_to_file,
            custom_output_path: output_path.as_deref(),
            github_owner: github_remote.as_ref().map(|r| r.owner.as_str()),
            github_repo: github_remote.as_ref().map(|r| r.repo.as_str()),
            github_token: github_token.clone(),
        };
        let result = generate_and_write_project_changelog(&params)?;

        if !result.has_user_changes {
            if !ci {
                info!(
                    "{}: no user-facing changes, skipping",
                    unit.user_facing_name
                );
            }
            continue;
        }

        if preview {
            print_changelog_preview(
                &unit.user_facing_name,
                &current_version,
                new_version.as_deref(),
                &ecosystem,
                &result.content,
            );
        } else if stdout {
            print!("{}", result.content);
        } else if !ci {
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
                unit.user_facing_name.bold(),
                ecosystem.display_name().dimmed(),
                version_info,
                path_display.dimmed()
            );
        }

        processed_count += 1;
        ci_projects.push(unit.user_facing_name.clone());
        if let Some(p) = result.path.as_ref() {
            ci_files_written.push(p.escaped().to_string());
        }
    }

    if !ci {
        if processed_count == 0 {
            if project_filter.is_some() {
                println!(
                    "{} ReleaseUnit '{}' not found or has no changes.",
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
    } else {
        emit_changelog_ci_status(
            processed_count,
            preview,
            stdout,
            &ci_projects,
            &ci_files_written,
        );
    }

    Ok(0)
}

#[derive(serde::Serialize)]
struct ChangelogCiStatus<'a> {
    /// Stable label. `generated` when at least one project produced a
    /// changelog, `nothing_to_do` otherwise.
    status: &'static str,
    /// `disk` (default), `preview` (--preview), `stdout` (--stdout).
    /// Tells the agent where the actual changelog content went.
    mode: &'static str,
    projects: &'a [String],
    files_written: &'a [String],
}

/// Emit the final `--ci` JSON status. If `--stdout` is set, the
/// changelog content is what's on stdout so the JSON status goes to
/// stderr; otherwise stdout is free for the JSON.
fn emit_changelog_ci_status(
    processed_count: usize,
    preview: bool,
    stdout: bool,
    projects: &[String],
    files_written: &[String],
) {
    let status = if processed_count == 0 {
        "nothing_to_do"
    } else {
        "generated"
    };
    let mode = if stdout {
        "stdout"
    } else if preview {
        "preview"
    } else {
        "disk"
    };
    let payload = ChangelogCiStatus {
        status,
        mode,
        projects,
        files_written,
    };
    let s = match serde_json::to_string_pretty(&payload) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: failed to serialise --ci status: {e}");
            return;
        }
    };
    if stdout {
        eprintln!("{s}");
    } else {
        println!("{s}");
    }
}

fn print_changelog_preview(
    project_name: &str,
    current_version: &str,
    new_version: Option<&str>,
    ecosystem: &Ecosystem,
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
