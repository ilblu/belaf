//! Modular init-wizard orchestrator.
//!
//! Phase I refactor (BELAF_MASTER_PLAN.md). The wizard is a stack of
//! [`Step`] trait objects driven by [`run_wizard_loop`]: render the
//! top-of-stack, dispatch input, react to the [`StepResult`]
//! returned. Concrete steps live in their own modules.

pub mod confirmation;
pub mod detector_review;
pub mod preset;
pub mod project;
pub mod state;
pub mod step;
pub mod tag_format;
pub mod upstream;
pub mod welcome;

use std::{collections::HashMap, fs, io, io::Write as _};

use anyhow::Result;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, MouseButton, MouseEvent,
        MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use crate::{
    atry,
    core::{
        git::repository::{PathMatcher, RepoPathBuf, Repository},
        project::DepRequirement,
        release_unit::detector,
        session::{AppBuilder, AppSession},
    },
};

use super::auto_detect;

use self::{
    state::{DetectedProject, WizardState},
    step::{MouseClick, Step, StepResult, WizardOutcome},
    welcome::WelcomeStep,
};

use super::{BootstrapConfiguration, BootstrapProjectInfo};

pub fn run(force: bool, upstream: Option<String>, preset: Option<String>) -> Result<i32> {
    let mut state = WizardState::new(force, preset);

    let repo = atry!(
        Repository::open_from_env();
        ["belaf is not being run from a Git working directory"]
        (note "run `belaf init` inside the Git work tree you wish to bootstrap")
    );

    let mut config_path = repo.resolve_config_dir();
    config_path.push("config.toml");
    state.config_exists = config_path.exists();

    let belaf_config_matcher = PathMatcher::new_include(RepoPathBuf::new(b"belaf"));
    if let Some(dirty) = atry!(
        repo.check_if_dirty(&[belaf_config_matcher]);
        ["failed to check the repository for modified files"]
    ) {
        state.dirty_warning = Some(format!(
            "Warning: uncommitted changes detected (e.g.: {})",
            dirty.escaped()
        ));
        if !force {
            state.error_message =
                Some("Repository has uncommitted changes. Use --force to override.".to_string());
        }
    }

    if let Some(url) = upstream {
        state.upstream_url = url;
    } else if let Ok(url) = repo.upstream_url() {
        state.upstream_url = url;
    }

    state.detection = detector::detect_all(&repo);

    let sess = AppBuilder::new()?.with_progress(true).initialize()?;

    for ident in sess.graph().toposorted() {
        let proj = sess.graph().lookup(ident);
        let prefix = proj.prefix();
        let prefix_str = if prefix.is_empty() {
            "root".to_string()
        } else {
            prefix.escaped()
        };

        state.projects.push(DetectedProject {
            name: proj.user_facing_name.clone(),
            version: proj.version.to_string(),
            prefix: prefix_str,
            selected: true,
        });
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let outcome = run_wizard_loop(&mut terminal, &mut state);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;

    match outcome? {
        WizardOutcome::Confirmed => execute_bootstrap_with_output(&state, &repo),
        WizardOutcome::Cancelled => Ok(1),
        WizardOutcome::SuggestedAlternative(msg) => {
            println!();
            println!("{}", msg);
            Ok(0)
        }
    }
}

fn run_wizard_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut WizardState,
) -> Result<WizardOutcome> {
    let mut stack: Vec<Box<dyn Step>> = vec![Box::new(WelcomeStep::new())];

    loop {
        {
            let top = stack
                .last_mut()
                .expect("wizard stack must never be empty during the loop");
            terminal.draw(|frame| {
                let area = frame.area();
                top.render(frame, area, state);
            })?;
        }

        let evt = event::read()?;

        // Mouse-click hit-test goes to the active step first.
        if let Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            ..
        }) = evt
        {
            let click = MouseClick { column, row };
            let top = stack
                .last_mut()
                .expect("wizard stack must never be empty during the loop");
            if let Some(result) = top.handle_click(&click, state) {
                if let Some(outcome) = apply(result, &mut stack) {
                    return Ok(outcome);
                }
                continue;
            }
        }

        let result = {
            let top = stack
                .last_mut()
                .expect("wizard stack must never be empty during the loop");
            top.handle_event(&evt, state)
        };

        if let Some(outcome) = apply(result, &mut stack) {
            return Ok(outcome);
        }
    }
}

/// Apply a [`StepResult`] to the wizard stack; return `Some(outcome)`
/// if the loop should terminate, `None` to keep looping.
fn apply(result: StepResult, stack: &mut Vec<Box<dyn Step>>) -> Option<WizardOutcome> {
    match result {
        StepResult::Continue => None,
        StepResult::Next(step) => {
            stack.push(step);
            None
        }
        StepResult::Back => {
            stack.pop();
            if stack.is_empty() {
                Some(WizardOutcome::Cancelled)
            } else {
                None
            }
        }
        StepResult::Exit(outcome) => Some(outcome),
    }
}

fn execute_bootstrap_with_output(state: &WizardState, repo: &Repository) -> Result<i32> {
    println!();
    let mut spinner = spinoff::Spinner::new(
        spinoff::spinners::Dots,
        "Initializing belaf...",
        spinoff::Color::Yellow,
    );

    match execute_bootstrap(state, repo) {
        Ok(_) => {
            spinner.success("Initialization complete!");
            let mut cfg_path = repo.resolve_config_dir();
            cfg_path.push("config.toml");
            if state.detector_accepted {
                let result = auto_detect::run(repo);
                if let Err(e) = auto_detect::append_to_config(&cfg_path, &result.toml_snippet) {
                    eprintln!(
                        "warning: detected bundles but failed to append to {}: {}",
                        cfg_path.display(),
                        e
                    );
                }
            }
            if let Some(snippet) = build_tag_format_snippet(state) {
                if let Err(e) = auto_detect::append_to_config(&cfg_path, &snippet) {
                    eprintln!(
                        "warning: failed to append tag_format override to {}: {}",
                        cfg_path.display(),
                        e
                    );
                }
            }
            print_terminal_summary(state);
            Ok(0)
        }
        Err(e) => {
            spinner.fail(&format!("Error: {}", e));
            Ok(1)
        }
    }
}

/// Phase I.3 — when the user picked a tag-format override on the
/// single-project tag-format step, build the per-project override
/// snippet that gets appended to `belaf/config.toml`.
fn build_tag_format_snippet(state: &WizardState) -> Option<String> {
    let format = state.tag_format_override.as_ref()?;
    let project = state.selected_projects().into_iter().next()?;
    Some(format!(
        "\n[projects.\"{name}\"]\ntag_format = \"{format}\"\n",
        name = project.name,
    ))
}

fn execute_bootstrap(state: &WizardState, repo: &Repository) -> Result<String> {
    use crate::core::embed::{EmbeddedConfig, EmbeddedPresets};

    let base_config = match state.preset.as_deref() {
        Some(preset_name) => EmbeddedPresets::get_preset_string(preset_name)?,
        None => EmbeddedConfig::get_config_string()?,
    };

    let cfg_text = base_config.replace(
        "upstream_urls = []",
        &format!("upstream_urls = [\"{}\"]", state.upstream_url),
    );

    let mut cfg_path = repo.resolve_config_dir();
    fs::create_dir_all(&cfg_path)?;

    cfg_path.push("config.toml");

    let mut f = fs::File::create(&cfg_path)?;
    f.write_all(cfg_text.as_bytes())?;

    let mut sess = AppSession::initialize_default()?;

    let mut bs_cfg = BootstrapConfiguration::default();
    let mut versions = HashMap::new();
    let selected_names: Vec<String> = state
        .selected_projects()
        .iter()
        .map(|p| p.name.clone())
        .collect();

    let topo_ids: Vec<_> = sess.graph().toposorted().collect();
    for ident in topo_ids {
        let proj = sess.graph_mut().lookup_mut(ident);
        if !selected_names.contains(&proj.user_facing_name) {
            continue;
        }

        bs_cfg.project.push(BootstrapProjectInfo {
            qnames: proj.qualified_names().to_owned(),
            version: proj.version.to_string(),
            release_commit: None,
        });

        versions.insert(proj.ident(), proj.version.clone());

        for dep in &mut proj.internal_deps[..] {
            dep.belaf_requirement = DepRequirement::Manual(
                versions
                    .get(&dep.ident)
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
            );
        }
    }

    let bs_text = toml::to_string_pretty(&bs_cfg)?;

    let mut bs_path = repo.resolve_config_dir();
    bs_path.push("bootstrap.toml");

    if !bs_path.exists() {
        let mut f = fs::File::create(&bs_path)?;
        f.write_all(bs_text.as_bytes())?;
    }

    sess.rewrite()?;

    let topo_ids: Vec<_> = sess.graph().toposorted().collect();
    for ident in topo_ids {
        let proj = sess.graph_mut().lookup_mut(ident);
        for dep in &mut proj.internal_deps[..] {
            dep.belaf_requirement = DepRequirement::Manual(dep.literal.clone());
        }
    }

    sess.rewrite_belaf_requirements()?;

    repo.create_baseline_tag()?;

    let action = if state.config_exists {
        "reconfigured"
    } else {
        "initialized"
    };

    Ok(format!(
        "Successfully {} {} project(s)!\n\nNext steps:\n1. Review the changes\n2. Commit the changes\n3. Try `belaf status`",
        action,
        selected_names.len()
    ))
}

fn hyperlink(text: &str, path: &std::path::Path) -> String {
    format!(
        "\x1b]8;;file://{}\x1b\\{}\x1b]8;;\x1b\\",
        path.display(),
        text
    )
}

fn print_terminal_summary(state: &WizardState) {
    use owo_colors::OwoColorize;

    let config_path = std::env::current_dir()
        .map(|p| p.join("belaf/config.toml"))
        .ok();

    println!();
    if state.config_exists {
        println!(
            "{} {}",
            "✅".green(),
            "Repository reconfigured successfully!".green().bold()
        );
    } else {
        println!(
            "{} {}",
            "✅".green(),
            "Repository initialized successfully!".green().bold()
        );
    }
    println!();
    println!("{}", "Created:".white().bold());
    if let Some(ref path) = config_path {
        println!(
            "  {} {}",
            "•".cyan(),
            hyperlink(&"belaf/config.toml".yellow().to_string(), path)
        );
    } else {
        println!("  {} {}", "•".cyan(), "belaf/config.toml".yellow());
    }
    println!();
    println!("{}", "Next steps:".white().bold());
    println!(
        "  {}. Run {} to see project versions",
        "1".cyan(),
        "belaf status".cyan()
    );
    println!(
        "  {}. Run {} when ready to release",
        "2".cyan(),
        "belaf prepare".cyan()
    );
    if let Some(ref path) = config_path {
        println!(
            "  {}. Edit {} to customize",
            "3".cyan(),
            hyperlink(&"belaf/config.toml".yellow().to_string(), path)
        );
    } else {
        println!(
            "  {}. Edit {} to customize",
            "3".cyan(),
            "belaf/config.toml".yellow()
        );
    }
    println!();
}
