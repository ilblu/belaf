use std::{collections::HashMap, fs, io, io::Write as _};

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame, Terminal,
};
use tracing::info;

use crate::{
    atry,
    core::release::{
        config::ConfigurationFile,
        project::DepRequirement,
        repository::{PathMatcher, RepoPathBuf, Repository},
        session::{AppBuilder, AppSession},
    },
};

use super::{BootstrapConfiguration, BootstrapProjectInfo};

#[derive(Clone, Debug)]
struct DetectedProject {
    name: String,
    version: String,
    prefix: String,
    selected: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WizardStep {
    Welcome,
    ProjectSelection,
    UpstreamConfig,
    Confirmation,
    Processing,
    Complete,
}

struct WizardState {
    step: WizardStep,
    projects: Vec<DetectedProject>,
    selected_project_idx: usize,
    upstream_url: String,
    upstream_input_active: bool,
    error_message: Option<String>,
    success_message: Option<String>,
    force: bool,
    dirty_warning: Option<String>,
}

impl WizardState {
    fn new(force: bool) -> Self {
        Self {
            step: WizardStep::Welcome,
            projects: Vec::new(),
            selected_project_idx: 0,
            upstream_url: String::new(),
            upstream_input_active: false,
            error_message: None,
            success_message: None,
            force,
            dirty_warning: None,
        }
    }

    fn selected_projects(&self) -> Vec<&DetectedProject> {
        self.projects.iter().filter(|p| p.selected).collect()
    }

    fn toggle_current_project(&mut self) {
        if let Some(proj) = self.projects.get_mut(self.selected_project_idx) {
            proj.selected = !proj.selected;
        }
    }

    fn select_all(&mut self) {
        for proj in &mut self.projects {
            proj.selected = true;
        }
    }

    fn deselect_all(&mut self) {
        for proj in &mut self.projects {
            proj.selected = false;
        }
    }
}

pub fn run(force: bool, upstream: Option<String>) -> Result<i32> {
    let mut state = WizardState::new(force);

    let repo = atry!(
        Repository::open_from_env();
        ["belaf is not being run from a Git working directory"]
    );

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

    let sess = atry!(
        AppBuilder::new()?.with_progress(true).initialize();
        ["could not initialize app and project graph"]
    );

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
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_wizard_loop(&mut terminal, &mut state, &repo);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

    result
}

fn run_wizard_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut WizardState,
    repo: &Repository,
) -> Result<i32> {
    loop {
        terminal.draw(|frame| render(frame, state))?;

        if let Event::Key(key) = event::read()? {
            match (key.code, key.modifiers, state.step) {
                (KeyCode::Char('c'), KeyModifiers::CONTROL, _)
                | (KeyCode::Char('q'), _, WizardStep::Welcome) => {
                    return Ok(1);
                }

                (KeyCode::Enter, _, WizardStep::Welcome) => {
                    if state.error_message.is_none() || state.force {
                        state.step = WizardStep::ProjectSelection;
                        state.error_message = None;
                    }
                }

                (KeyCode::Down | KeyCode::Char('j'), _, WizardStep::ProjectSelection) => {
                    if !state.projects.is_empty() {
                        state.selected_project_idx =
                            (state.selected_project_idx + 1) % state.projects.len();
                    }
                }
                (KeyCode::Up | KeyCode::Char('k'), _, WizardStep::ProjectSelection) => {
                    if !state.projects.is_empty() {
                        state.selected_project_idx = if state.selected_project_idx == 0 {
                            state.projects.len() - 1
                        } else {
                            state.selected_project_idx - 1
                        };
                    }
                }
                (KeyCode::Char(' '), _, WizardStep::ProjectSelection) => {
                    state.toggle_current_project();
                }
                (KeyCode::Char('a'), _, WizardStep::ProjectSelection) => {
                    state.select_all();
                }
                (KeyCode::Char('n'), _, WizardStep::ProjectSelection) => {
                    state.deselect_all();
                }
                (KeyCode::Enter, _, WizardStep::ProjectSelection) => {
                    if state.selected_projects().is_empty() {
                        state.error_message =
                            Some("Please select at least one project".to_string());
                    } else {
                        state.error_message = None;
                        state.step = WizardStep::UpstreamConfig;
                    }
                }
                (KeyCode::Esc, _, WizardStep::ProjectSelection) => {
                    state.step = WizardStep::Welcome;
                }

                (KeyCode::Char(c), _, WizardStep::UpstreamConfig)
                    if state.upstream_input_active =>
                {
                    state.upstream_url.push(c);
                }
                (KeyCode::Backspace, _, WizardStep::UpstreamConfig)
                    if state.upstream_input_active =>
                {
                    state.upstream_url.pop();
                }
                (KeyCode::Enter, _, WizardStep::UpstreamConfig) => {
                    if state.upstream_url.is_empty() {
                        state.error_message = Some("Upstream URL is required".to_string());
                    } else {
                        state.error_message = None;
                        state.step = WizardStep::Confirmation;
                    }
                }
                (KeyCode::Tab, _, WizardStep::UpstreamConfig) => {
                    state.upstream_input_active = !state.upstream_input_active;
                }
                (KeyCode::Esc, _, WizardStep::UpstreamConfig) => {
                    state.step = WizardStep::ProjectSelection;
                }

                (KeyCode::Enter | KeyCode::Char('y'), _, WizardStep::Confirmation) => {
                    state.step = WizardStep::Processing;
                    terminal.draw(|frame| render(frame, state))?;

                    match execute_bootstrap(state, repo) {
                        Ok(msg) => {
                            state.success_message = Some(msg);
                            state.step = WizardStep::Complete;
                        }
                        Err(e) => {
                            state.error_message = Some(format!("Error: {}", e));
                            state.step = WizardStep::Confirmation;
                        }
                    }
                }
                (KeyCode::Char('n') | KeyCode::Esc, _, WizardStep::Confirmation) => {
                    state.step = WizardStep::UpstreamConfig;
                }

                (KeyCode::Enter | KeyCode::Char('q'), _, WizardStep::Complete) => {
                    return Ok(0);
                }

                _ => {}
            }
        }
    }
}

fn execute_bootstrap(state: &WizardState, repo: &Repository) -> Result<String> {
    let mut cfg = ConfigurationFile::default();
    cfg.repo.upstream_urls = vec![state.upstream_url.clone()];
    let cfg_text = cfg.into_toml()?;

    let mut cfg_path = repo.resolve_config_dir();
    fs::create_dir_all(&cfg_path)?;

    cfg_path.push("config.toml");

    if cfg_path.exists() {
        info!("config file already exists, skipping");
    } else {
        let mut f = fs::File::create(&cfg_path)?;
        f.write_all(cfg_text.as_bytes())?;
    }

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

    Ok(format!(
        "Successfully initialized {} project(s)!\n\nNext steps:\n1. Review the changes\n2. Add belaf/ to your repository\n3. Commit the changes\n4. Try `belaf release status`",
        selected_names.len()
    ))
}

fn render(frame: &mut Frame, state: &WizardState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(3),
        ])
        .split(frame.area());

    render_header(frame, chunks[0], state);

    match state.step {
        WizardStep::Welcome => render_welcome(frame, chunks[1], state),
        WizardStep::ProjectSelection => render_project_selection(frame, chunks[1], state),
        WizardStep::UpstreamConfig => render_upstream_config(frame, chunks[1], state),
        WizardStep::Confirmation => render_confirmation(frame, chunks[1], state),
        WizardStep::Processing => render_processing(frame, chunks[1]),
        WizardStep::Complete => render_complete(frame, chunks[1], state),
    }

    render_footer(frame, chunks[2], state);
}

fn render_header(frame: &mut Frame, area: Rect, state: &WizardState) {
    let step_text = match state.step {
        WizardStep::Welcome => "Welcome",
        WizardStep::ProjectSelection => "Step 1/3: Project Selection",
        WizardStep::UpstreamConfig => "Step 2/3: Upstream Configuration",
        WizardStep::Confirmation => "Step 3/3: Confirmation",
        WizardStep::Processing => "Processing...",
        WizardStep::Complete => "Complete!",
    };

    let block = Block::default()
        .title(format!(" Clikd Release Init - {} ", step_text))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    frame.render_widget(block, area);
}

fn render_welcome(frame: &mut Frame, area: Rect, state: &WizardState) {
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "Welcome to Clikd Release Management!",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("This wizard will help you initialize release management for your repository."),
        Line::from(""),
        Line::from(format!(
            "Detected {} project(s) in your repository.",
            state.projects.len()
        )),
        Line::from(""),
    ];

    if let Some(ref warning) = state.dirty_warning {
        lines.push(Line::from(Span::styled(
            warning.clone(),
            Style::default().fg(Color::Yellow),
        )));
        lines.push(Line::from(""));
    }

    if let Some(ref error) = state.error_message {
        lines.push(Line::from(Span::styled(
            error.clone(),
            Style::default().fg(Color::Red),
        )));
        lines.push(Line::from(""));
    }

    lines.push(Line::from("Press ENTER to continue or 'q' to quit."));

    let para =
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" Welcome "));

    frame.render_widget(para, area);
}

fn render_project_selection(frame: &mut Frame, area: Rect, state: &WizardState) {
    let items: Vec<ListItem> = state
        .projects
        .iter()
        .enumerate()
        .map(|(idx, proj)| {
            let checkbox = if proj.selected { "[✓]" } else { "[ ]" };
            let text = format!(
                "{} {} @ {} ({})",
                checkbox, proj.name, proj.version, proj.prefix
            );
            let style = if idx == state.selected_project_idx {
                Style::default().bg(Color::DarkGray).fg(Color::White)
            } else if proj.selected {
                Style::default().fg(Color::Green)
            } else {
                Style::default()
            };
            ListItem::new(text).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Select Projects (Space=toggle, a=all, n=none) "),
    );

    frame.render_widget(list, area);

    if let Some(ref error) = state.error_message {
        let popup_area = centered_rect(60, 20, area);
        frame.render_widget(Clear, popup_area);
        let popup = Paragraph::new(error.clone())
            .style(Style::default().fg(Color::Red))
            .block(Block::default().borders(Borders::ALL).title(" Error "));
        frame.render_widget(popup, popup_area);
    }
}

fn render_upstream_config(frame: &mut Frame, area: Rect, state: &WizardState) {
    let lines = vec![
        Line::from(""),
        Line::from("Enter the upstream Git URL for your repository:"),
        Line::from(""),
        Line::from(Span::styled(
            if state.upstream_url.is_empty() {
                "(empty)"
            } else {
                &state.upstream_url
            },
            Style::default().fg(if state.upstream_input_active {
                Color::Yellow
            } else {
                Color::White
            }),
        )),
        Line::from(""),
        Line::from("Press TAB to edit, ENTER to continue."),
    ];

    let para = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Upstream Configuration "),
    );

    frame.render_widget(para, area);
}

fn render_confirmation(frame: &mut Frame, area: Rect, state: &WizardState) {
    let selected = state.selected_projects();
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "Configuration Summary:",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(format!("Upstream URL: {}", state.upstream_url)),
        Line::from(format!("Selected Projects: {}", selected.len())),
        Line::from(""),
    ];

    for proj in selected.iter().take(10) {
        lines.push(Line::from(format!("  • {} @ {}", proj.name, proj.version)));
    }

    if selected.len() > 10 {
        lines.push(Line::from(format!(
            "  ... and {} more",
            selected.len() - 10
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from("This will:"));
    lines.push(Line::from("  • Create belaf/config.toml"));
    lines.push(Line::from("  • Create belaf/bootstrap.toml"));
    lines.push(Line::from("  • Update project version files"));
    lines.push(Line::from("  • Create baseline Git tags"));
    lines.push(Line::from(""));
    lines.push(Line::from(
        "Press 'y' or ENTER to confirm, 'n' or ESC to go back.",
    ));

    let para = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Confirmation "),
    );

    frame.render_widget(para, area);
}

fn render_processing(frame: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(""),
        Line::from(""),
        Line::from(Span::styled(
            "Processing...",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("Please wait while we initialize your release configuration."),
    ];

    let para =
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" Processing "));

    frame.render_widget(para, area);
}

fn render_complete(frame: &mut Frame, area: Rect, state: &WizardState) {
    let message = state
        .success_message
        .as_deref()
        .unwrap_or("Initialization complete!");

    let lines: Vec<Line> = message.lines().map(|l| Line::from(l.to_string())).collect();

    let para = Paragraph::new(lines)
        .style(Style::default().fg(Color::Green))
        .block(Block::default().borders(Borders::ALL).title(" Success "));

    frame.render_widget(para, area);
}

fn render_footer(frame: &mut Frame, area: Rect, state: &WizardState) {
    let help_text = match state.step {
        WizardStep::Welcome => "ENTER: Continue  q: Quit",
        WizardStep::ProjectSelection => {
            "↑/↓: Navigate  SPACE: Toggle  a: All  n: None  ENTER: Next  ESC: Back"
        }
        WizardStep::UpstreamConfig => "TAB: Edit  ENTER: Next  ESC: Back",
        WizardStep::Confirmation => "y/ENTER: Confirm  n/ESC: Back",
        WizardStep::Processing => "Please wait...",
        WizardStep::Complete => "ENTER/q: Exit",
    };

    let para = Paragraph::new(help_text)
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::ALL));

    frame.render_widget(para, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
