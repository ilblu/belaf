use std::{collections::HashMap, fs, io, io::Write as _};

use crate::{
    atry,
    core::{
        git::repository::{PathMatcher, RepoPathBuf, Repository},
        project::DepRequirement,
        session::{AppBuilder, AppSession},
        ui::{components::toggle_panel::TogglePanel, utils::centered_rect},
    },
};
use anyhow::Result;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers, MouseButton,
        MouseEvent, MouseEventKind,
    },
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
    PresetSelection,
    ProjectSelection,
    UpstreamConfig,
    Confirmation,
}

struct WizardState {
    step: WizardStep,
    projects: Vec<DetectedProject>,
    selected_project_idx: usize,
    upstream_url: String,
    upstream_input_active: bool,
    error_message: Option<String>,
    force: bool,
    dirty_warning: Option<String>,
    preset: Option<String>,
    preset_from_cli: bool,
    selected_preset_idx: usize,
    available_presets: Vec<String>,
    preset_toggle: TogglePanel,
    config_exists: bool,
    confirmed: bool,
}

impl WizardState {
    fn new(force: bool, preset: Option<String>) -> Self {
        use crate::core::embed::EmbeddedPresets;

        let preset_from_cli = preset.is_some();
        let mut available_presets = vec!["default".to_string()];
        available_presets.extend(EmbeddedPresets::list_presets());

        let selected_preset_idx = preset
            .as_ref()
            .and_then(|p| available_presets.iter().position(|x| x == p))
            .unwrap_or(0);

        Self {
            step: WizardStep::Welcome,
            projects: Vec::new(),
            selected_project_idx: 0,
            upstream_url: String::new(),
            upstream_input_active: false,
            error_message: None,
            force,
            dirty_warning: None,
            preset,
            preset_from_cli,
            selected_preset_idx,
            available_presets,
            preset_toggle: TogglePanel::default(),
            config_exists: false,
            confirmed: false,
        }
    }

    fn selected_preset_name(&self) -> &str {
        self.available_presets
            .get(self.selected_preset_idx)
            .map(|s| s.as_str())
            .unwrap_or("default")
    }

    fn handle_preset_toggle_click(&mut self, x: u16, y: u16) -> bool {
        self.preset_toggle.handle_click(x, y)
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

pub fn run(force: bool, upstream: Option<String>, preset: Option<String>) -> Result<i32> {
    let mut state = WizardState::new(force, preset);

    let repo = atry!(
        Repository::open_from_env();
        ["belaf is not being run from a Git working directory"]
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
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_wizard_loop(&mut terminal, &mut state);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;

    if state.confirmed {
        return execute_bootstrap_with_output(&state, &repo);
    }

    result
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
            print_terminal_summary(state);
            Ok(0)
        }
        Err(e) => {
            spinner.fail(&format!("Error: {}", e));
            Ok(1)
        }
    }
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

fn run_wizard_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut WizardState,
) -> Result<i32> {
    loop {
        terminal.draw(|frame| render(frame, state))?;

        match event::read()? {
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row,
                ..
            }) => {
                if state.step == WizardStep::PresetSelection
                    && state.handle_preset_toggle_click(column, row)
                {
                    continue;
                }
            }
            Event::Key(key) => match (key.code, key.modifiers, state.step) {
                (KeyCode::Char('c'), KeyModifiers::CONTROL, _)
                | (KeyCode::Char('q'), _, WizardStep::Welcome) => {
                    return Ok(1);
                }

                (KeyCode::Enter, _, WizardStep::Welcome) => {
                    if state.error_message.is_none() || state.force {
                        state.step = if state.preset_from_cli {
                            WizardStep::ProjectSelection
                        } else {
                            WizardStep::PresetSelection
                        };
                        state.error_message = None;
                    }
                }

                (KeyCode::Down | KeyCode::Char('j'), _, WizardStep::PresetSelection) => {
                    if !state.available_presets.is_empty() {
                        state.selected_preset_idx =
                            (state.selected_preset_idx + 1) % state.available_presets.len();
                    }
                }
                (KeyCode::Up | KeyCode::Char('k'), _, WizardStep::PresetSelection) => {
                    if !state.available_presets.is_empty() {
                        state.selected_preset_idx = if state.selected_preset_idx == 0 {
                            state.available_presets.len() - 1
                        } else {
                            state.selected_preset_idx - 1
                        };
                    }
                }
                (KeyCode::Enter, _, WizardStep::PresetSelection) => {
                    let selected = state.selected_preset_name().to_string();
                    state.preset = if selected == "default" {
                        None
                    } else {
                        Some(selected)
                    };
                    state.step = WizardStep::ProjectSelection;
                }
                (KeyCode::Char('m'), _, WizardStep::PresetSelection) => {
                    state.preset_toggle.toggle();
                }
                (KeyCode::Esc, _, WizardStep::PresetSelection) => {
                    state.step = WizardStep::Welcome;
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
                    state.step = if state.preset_from_cli {
                        WizardStep::Welcome
                    } else {
                        WizardStep::PresetSelection
                    };
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
                    state.confirmed = true;
                    return Ok(0);
                }
                (KeyCode::Char('n') | KeyCode::Esc, _, WizardStep::Confirmation) => {
                    state.step = WizardStep::UpstreamConfig;
                }

                _ => {}
            },
            _ => {}
        }
    }
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

fn render(frame: &mut Frame, state: &mut WizardState) {
    let area = frame.area();

    match state.step {
        WizardStep::Welcome => render_welcome(frame, area, state),
        WizardStep::PresetSelection => render_preset_selection(frame, area, state),
        WizardStep::ProjectSelection => render_project_selection(frame, area, state),
        WizardStep::UpstreamConfig => render_upstream_config(frame, area, state),
        WizardStep::Confirmation => render_confirmation(frame, area, state),
    }
}

fn render_welcome(frame: &mut Frame, area: Rect, state: &WizardState) {
    let is_reconfigure = state.config_exists;

    let border_color = if is_reconfigure {
        Color::Red
    } else {
        Color::Cyan
    };

    let logo_color = if is_reconfigure {
        Color::Red
    } else {
        Color::Cyan
    };

    let project_count = state.projects.len();
    let project_text = if project_count == 1 {
        "1 project".to_string()
    } else {
        format!("{} projects", project_count)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(if is_reconfigure {
            Span::styled(
                " ⚠ Reconfigure ",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(
                " Welcome ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
        });

    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    if is_reconfigure {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(8),
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(3),
            ])
            .margin(1)
            .split(inner_area);

        let logo = vec![
            Line::from(Span::styled(
                "   ██████╗██╗     ██╗██╗  ██╗██████╗ ",
                Style::default().fg(logo_color),
            )),
            Line::from(Span::styled(
                "  ██╔════╝██║     ██║██║ ██╔╝██╔══██╗",
                Style::default().fg(logo_color),
            )),
            Line::from(Span::styled(
                "  ██║     ██║     ██║█████╔╝ ██║  ██║",
                Style::default().fg(logo_color),
            )),
            Line::from(Span::styled(
                "  ██║     ██║     ██║██╔═██╗ ██║  ██║",
                Style::default().fg(logo_color),
            )),
            Line::from(Span::styled(
                "  ╚██████╗███████╗██║██║  ██╗██████╔╝",
                Style::default().fg(logo_color),
            )),
            Line::from(Span::styled(
                "   ╚═════╝╚══════╝╚═╝╚═╝  ╚═╝╚═════╝ ",
                Style::default().fg(logo_color),
            )),
            Line::from(Span::styled(
                "         Release Management",
                Style::default().fg(Color::Gray),
            )),
        ];
        let logo_para = Paragraph::new(logo).alignment(ratatui::layout::Alignment::Center);
        frame.render_widget(logo_para, chunks[0]);

        let warning_header = vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                "⚠️  RECONFIGURE MODE  ⚠️",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )]),
        ];
        let warning_para =
            Paragraph::new(warning_header).alignment(ratatui::layout::Alignment::Center);
        frame.render_widget(warning_para, chunks[1]);

        let warning_text = vec![
            Line::from(Span::styled(
                "A configuration already exists in this repo.",
                Style::default().fg(Color::Yellow),
            )),
            Line::from(Span::styled(
                "Continuing will overwrite your settings.",
                Style::default().fg(Color::Yellow),
            )),
        ];
        let warning_text_para =
            Paragraph::new(warning_text).alignment(ratatui::layout::Alignment::Center);
        frame.render_widget(warning_text_para, chunks[2]);

        let info_lines = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("📦 ", Style::default()),
                Span::styled("Detected: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    project_text,
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
        ];
        let info_para = Paragraph::new(info_lines).alignment(ratatui::layout::Alignment::Center);
        frame.render_widget(info_para, chunks[3]);

        let action_text = vec![Line::from(vec![
            Span::styled("Press ", Style::default().fg(Color::Gray)),
            Span::styled(
                "ENTER",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" to reconfigure  •  ", Style::default().fg(Color::Gray)),
            Span::styled(
                "Q",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" to quit", Style::default().fg(Color::Gray)),
        ])];
        let action_para = Paragraph::new(action_text).alignment(ratatui::layout::Alignment::Center);
        frame.render_widget(action_para, chunks[4]);
    } else {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(8),
                Constraint::Min(0),
                Constraint::Length(3),
            ])
            .margin(1)
            .split(inner_area);

        let logo = vec![
            Line::from(Span::styled(
                "   ██████╗██╗     ██╗██╗  ██╗██████╗ ",
                Style::default().fg(logo_color),
            )),
            Line::from(Span::styled(
                "  ██╔════╝██║     ██║██║ ██╔╝██╔══██╗",
                Style::default().fg(logo_color),
            )),
            Line::from(Span::styled(
                "  ██║     ██║     ██║█████╔╝ ██║  ██║",
                Style::default().fg(logo_color),
            )),
            Line::from(Span::styled(
                "  ██║     ██║     ██║██╔═██╗ ██║  ██║",
                Style::default().fg(logo_color),
            )),
            Line::from(Span::styled(
                "  ╚██████╗███████╗██║██║  ██╗██████╔╝",
                Style::default().fg(logo_color),
            )),
            Line::from(Span::styled(
                "   ╚═════╝╚══════╝╚═╝╚═╝  ╚═╝╚═════╝ ",
                Style::default().fg(logo_color),
            )),
            Line::from(Span::styled(
                "         Release Management",
                Style::default().fg(Color::Gray),
            )),
        ];
        let logo_para = Paragraph::new(logo).alignment(ratatui::layout::Alignment::Center);
        frame.render_widget(logo_para, chunks[0]);

        let mut info_lines = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("📦 ", Style::default()),
                Span::styled("Detected: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    project_text,
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(""),
        ];

        if let Some(ref warning) = state.dirty_warning {
            info_lines.push(Line::from(vec![
                Span::styled("⚠️  ", Style::default()),
                Span::styled(warning.clone(), Style::default().fg(Color::Yellow)),
            ]));
            info_lines.push(Line::from(""));
        }

        if let Some(ref error) = state.error_message {
            info_lines.push(Line::from(vec![
                Span::styled("❌ ", Style::default()),
                Span::styled(error.clone(), Style::default().fg(Color::Red)),
            ]));
            info_lines.push(Line::from(""));
        }

        info_lines.push(Line::from(""));
        info_lines.push(Line::from(Span::styled(
            "This wizard will guide you through:",
            Style::default().fg(Color::White),
        )));
        info_lines.push(Line::from(vec![
            Span::styled("  → ", Style::default().fg(Color::Cyan)),
            Span::styled(
                "Changelog preset selection",
                Style::default().fg(Color::Gray),
            ),
        ]));
        info_lines.push(Line::from(vec![
            Span::styled("  → ", Style::default().fg(Color::Cyan)),
            Span::styled("Project configuration", Style::default().fg(Color::Gray)),
        ]));
        info_lines.push(Line::from(vec![
            Span::styled("  → ", Style::default().fg(Color::Cyan)),
            Span::styled("Repository setup", Style::default().fg(Color::Gray)),
        ]));

        let info_para = Paragraph::new(info_lines).alignment(ratatui::layout::Alignment::Center);
        frame.render_widget(info_para, chunks[1]);

        let action_text = vec![Line::from(vec![
            Span::styled("Press ", Style::default().fg(Color::Gray)),
            Span::styled(
                "ENTER",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" to start  •  ", Style::default().fg(Color::Gray)),
            Span::styled(
                "Q",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" to quit", Style::default().fg(Color::Gray)),
        ])];
        let action_para = Paragraph::new(action_text).alignment(ratatui::layout::Alignment::Center);
        frame.render_widget(action_para, chunks[2]);
    }
}

fn render_preset_selection(frame: &mut Frame, area: Rect, state: &mut WizardState) {
    use crate::core::embed::{EmbeddedConfig, EmbeddedPresets};
    use ratatui::widgets::{Padding, Wrap};

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " Step 1: Changelog Preset ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    let outer_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(inner_area);

    let header_text = vec![Line::from(vec![
        Span::styled("📋 ", Style::default()),
        Span::styled(
            "Choose a changelog format that fits your project",
            Style::default().fg(Color::White),
        ),
    ])];
    let header = Paragraph::new(header_text).alignment(ratatui::layout::Alignment::Center);
    frame.render_widget(header, outer_chunks[0]);

    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(outer_chunks[1]);

    let items: Vec<ListItem> = state
        .available_presets
        .iter()
        .enumerate()
        .map(|(idx, name)| {
            let (icon, description) = match name.as_str() {
                "default" => ("📦", "Conventional Commits grouped by type"),
                "keepachangelog" => ("📝", "Keep a Changelog specification"),
                "flat" => ("📄", "Simple flat list - What's Changed"),
                "minimal" => ("✨", "Minimal - just version and date"),
                _ => ("📋", "Custom preset"),
            };
            let is_selected = idx == state.selected_preset_idx;
            let lines = vec![
                Line::from(vec![
                    Span::styled(
                        format!(" {} ", icon),
                        if is_selected {
                            Style::default().fg(Color::Cyan)
                        } else {
                            Style::default()
                        },
                    ),
                    Span::styled(
                        name.clone(),
                        if is_selected {
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::White)
                        },
                    ),
                ]),
                Line::from(Span::styled(
                    format!("    {}", description),
                    Style::default().fg(Color::Gray),
                )),
            ];
            let style = if is_selected {
                Style::default().bg(Color::Rgb(40, 40, 50))
            } else {
                Style::default()
            };
            ListItem::new(lines).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Gray))
            .title(Span::styled(" Presets ", Style::default().fg(Color::White))),
    );

    frame.render_widget(list, main_chunks[0]);

    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(main_chunks[1]);

    let preset_name = state.selected_preset_name().to_string();
    state
        .preset_toggle
        .render(frame, right_chunks[0], &preset_name);

    let config_content = if preset_name == "default" {
        EmbeddedConfig::get_config_string().unwrap_or_else(|_| "Config not available".to_string())
    } else {
        EmbeddedPresets::get_preset_string(&preset_name)
            .unwrap_or_else(|_| "Preset not available".to_string())
    };

    if state.preset_toggle.is_right() {
        let source_text = config_content
            .lines()
            .take(50)
            .collect::<Vec<_>>()
            .join("\n");

        let paragraph = Paragraph::new(source_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Gray))
                    .title(Span::styled(
                        " TOML Source ",
                        Style::default().fg(Color::Magenta),
                    ))
                    .padding(Padding::horizontal(1)),
            )
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(Color::Rgb(180, 180, 180)));
        frame.render_widget(paragraph, right_chunks[1]);
    } else {
        let example_changelog = generate_preset_example(&preset_name);
        let markdown_text = crate::core::ui::markdown::render_markdown(&example_changelog);

        let paragraph = Paragraph::new(markdown_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Gray))
                    .title(Span::styled(
                        " Changelog Preview ",
                        Style::default().fg(Color::Cyan),
                    ))
                    .padding(Padding::horizontal(1)),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(paragraph, right_chunks[1]);
    }

    let hints = Line::from(vec![
        Span::styled("↑↓", Style::default().fg(Color::Cyan)),
        Span::styled(" select  ", Style::default().fg(Color::Gray)),
        Span::styled("m", Style::default().fg(Color::Cyan)),
        Span::styled(" toggle view  ", Style::default().fg(Color::Gray)),
        Span::styled("Enter", Style::default().fg(Color::Green)),
        Span::styled(" continue  ", Style::default().fg(Color::Gray)),
        Span::styled("q", Style::default().fg(Color::Red)),
        Span::styled(" quit", Style::default().fg(Color::Gray)),
    ]);
    let hints_para = Paragraph::new(hints).alignment(ratatui::layout::Alignment::Center);
    frame.render_widget(hints_para, outer_chunks[2]);
}

fn generate_preset_example(preset_name: &str) -> String {
    match preset_name {
        "default" => r#"## [1.2.0] - 2025-01-15

### ✨ Features

- **cli:** Add `--verbose` flag for detailed output
- **api:** Implement rate limiting for requests

### 🐛 Bug Fixes

- **auth:** Resolve token refresh race condition
- **parser:** Handle unicode characters in paths

### ⚡ Performance

- Optimize database queries

### 📚 Documentation

- Update installation guide
"#
        .to_string(),
        "keepachangelog" => r#"## [1.2.0] - 2025-01-15

### Added

- Add `--verbose` flag for detailed output
- Implement rate limiting for requests

### Fixed

- Resolve token refresh race condition
- Handle unicode characters in paths

### Changed

- Optimize database queries
- Update installation guide
"#
        .to_string(),
        "flat" => r#"## What's Changed

* Add `--verbose` flag for detailed output by @nyxb
* Implement rate limiting for requests by @nyxb
* Resolve token refresh race condition by @nyxb
* Handle unicode characters in paths by @nyxb
* Optimize database queries by @nyxb
* Update installation guide by @nyxb

**Full Changelog**: https://github.com/user/repo/compare/v1.1.0...v1.2.0
"#
        .to_string(),
        "minimal" => r#"## 1.2.0 (2025-01-15)

- Add `--verbose` flag for detailed output
- Implement rate limiting for requests
- Resolve token refresh race condition
- Handle unicode characters in paths
- Optimize database queries
- Update installation guide
"#
        .to_string(),
        _ => "Preview not available for this preset.".to_string(),
    }
}

fn render_project_selection(frame: &mut Frame, area: Rect, state: &WizardState) {
    let selected_count = state.projects.iter().filter(|p| p.selected).count();
    let total_count = state.projects.len();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " Step 2: Project Selection ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(inner_area);

    let header_lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("📦 ", Style::default()),
            Span::styled(
                "Select which projects to include in release management",
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("   Selected: ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("{}", selected_count),
                Style::default()
                    .fg(if selected_count > 0 {
                        Color::Green
                    } else {
                        Color::Yellow
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" / {}", total_count),
                Style::default().fg(Color::Gray),
            ),
        ]),
    ];
    let header = Paragraph::new(header_lines).alignment(ratatui::layout::Alignment::Center);
    frame.render_widget(header, chunks[0]);

    let items: Vec<ListItem> = state
        .projects
        .iter()
        .enumerate()
        .map(|(idx, proj)| {
            let is_current = idx == state.selected_project_idx;
            let checkbox = if proj.selected { "✅" } else { "⬜" };
            let lines = vec![Line::from(vec![
                Span::styled(format!(" {} ", checkbox), Style::default()),
                Span::styled(
                    proj.name.clone(),
                    if is_current {
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else if proj.selected {
                        Style::default().fg(Color::Green)
                    } else {
                        Style::default().fg(Color::White)
                    },
                ),
                Span::styled(
                    format!(" @ {}", proj.version),
                    Style::default().fg(Color::Gray),
                ),
                Span::styled(
                    format!("  ({})", proj.prefix),
                    Style::default().fg(Color::Rgb(100, 100, 100)),
                ),
            ])];
            let style = if is_current {
                Style::default().bg(Color::Rgb(40, 40, 50))
            } else {
                Style::default()
            };
            ListItem::new(lines).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Gray))
            .title(Span::styled(
                " Projects ",
                Style::default().fg(Color::White),
            )),
    );

    frame.render_widget(list, chunks[1]);

    let hints = Line::from(vec![
        Span::styled("↑↓", Style::default().fg(Color::Cyan)),
        Span::styled(" navigate  ", Style::default().fg(Color::Gray)),
        Span::styled("Space", Style::default().fg(Color::Cyan)),
        Span::styled(" toggle  ", Style::default().fg(Color::Gray)),
        Span::styled("a", Style::default().fg(Color::Green)),
        Span::styled(" all  ", Style::default().fg(Color::Gray)),
        Span::styled("n", Style::default().fg(Color::Yellow)),
        Span::styled(" none  ", Style::default().fg(Color::Gray)),
        Span::styled("Enter", Style::default().fg(Color::Green)),
        Span::styled(" continue  ", Style::default().fg(Color::Gray)),
        Span::styled("q", Style::default().fg(Color::Red)),
        Span::styled(" quit", Style::default().fg(Color::Gray)),
    ]);
    let hints_para = Paragraph::new(hints).alignment(ratatui::layout::Alignment::Center);
    frame.render_widget(hints_para, chunks[2]);

    if let Some(ref error) = state.error_message {
        let popup_area = centered_rect(60, 20, area);
        frame.render_widget(Clear, popup_area);
        let popup = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(error.clone(), Style::default().fg(Color::Red))),
        ])
        .alignment(ratatui::layout::Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red))
                .title(Span::styled(
                    " ⚠ Error ",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                )),
        );
        frame.render_widget(popup, popup_area);
    }
}

fn render_upstream_config(frame: &mut Frame, area: Rect, state: &WizardState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " Step 3: Repository Configuration ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(inner_area);

    let header_lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("🔗 ", Style::default()),
            Span::styled(
                "Configure the upstream Git repository URL",
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(Span::styled(
            "   Used for changelog links and release references",
            Style::default().fg(Color::Gray),
        )),
    ];
    let header = Paragraph::new(header_lines).alignment(ratatui::layout::Alignment::Center);
    frame.render_widget(header, chunks[0]);

    let content_area = centered_rect(80, 50, chunks[1]);

    let input_border_color = if state.upstream_input_active {
        Color::Yellow
    } else {
        Color::Gray
    };

    let url_display = if state.upstream_url.is_empty() {
        Span::styled(
            "https://github.com/user/repo",
            Style::default().fg(Color::Rgb(80, 80, 80)),
        )
    } else {
        Span::styled(
            state.upstream_url.clone(),
            Style::default()
                .fg(if state.upstream_input_active {
                    Color::Yellow
                } else {
                    Color::White
                })
                .add_modifier(if state.upstream_input_active {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        )
    };

    let cursor = if state.upstream_input_active {
        Span::styled("▌", Style::default().fg(Color::Yellow))
    } else {
        Span::raw("")
    };

    let input_lines = vec![
        Line::from(""),
        Line::from(vec![Span::raw("  "), url_display, cursor]),
        Line::from(""),
    ];

    let input_block = Paragraph::new(input_lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(input_border_color))
            .title(Span::styled(
                if state.upstream_input_active {
                    " ✏️  Editing URL "
                } else {
                    " URL "
                },
                Style::default().fg(if state.upstream_input_active {
                    Color::Yellow
                } else {
                    Color::White
                }),
            )),
    );

    frame.render_widget(input_block, content_area);

    let hints = if state.upstream_input_active {
        Line::from(vec![
            Span::styled("Type", Style::default().fg(Color::Yellow)),
            Span::styled(" to enter URL  ", Style::default().fg(Color::Gray)),
            Span::styled("Backspace", Style::default().fg(Color::Yellow)),
            Span::styled(" delete  ", Style::default().fg(Color::Gray)),
            Span::styled("Tab/Esc", Style::default().fg(Color::Cyan)),
            Span::styled(" finish editing", Style::default().fg(Color::Gray)),
        ])
    } else {
        Line::from(vec![
            Span::styled("Tab", Style::default().fg(Color::Cyan)),
            Span::styled(" edit URL  ", Style::default().fg(Color::Gray)),
            Span::styled("Enter", Style::default().fg(Color::Green)),
            Span::styled(" continue  ", Style::default().fg(Color::Gray)),
            Span::styled("Backspace", Style::default().fg(Color::Yellow)),
            Span::styled(" back  ", Style::default().fg(Color::Gray)),
            Span::styled("q", Style::default().fg(Color::Red)),
            Span::styled(" quit", Style::default().fg(Color::Gray)),
        ])
    };
    let hints_para = Paragraph::new(hints).alignment(ratatui::layout::Alignment::Center);
    frame.render_widget(hints_para, chunks[2]);
}

fn render_confirmation(frame: &mut Frame, area: Rect, state: &WizardState) {
    let selected = state.selected_projects();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " Step 4: Confirmation ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(inner_area);

    let header_lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("📋 ", Style::default()),
            Span::styled(
                "Review your configuration before initializing",
                Style::default().fg(Color::White),
            ),
        ]),
    ];
    let header = Paragraph::new(header_lines).alignment(ratatui::layout::Alignment::Center);
    frame.render_widget(header, chunks[0]);

    let content_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .margin(1)
        .split(chunks[1]);

    let mut summary_lines = vec![
        Line::from(vec![
            Span::styled("🔗 ", Style::default()),
            Span::styled("Repository", Style::default().fg(Color::White)),
        ]),
        Line::from(Span::styled(
            format!("   {}", state.upstream_url),
            Style::default().fg(Color::Gray),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("📦 ", Style::default()),
            Span::styled(
                format!("Projects ({})", selected.len()),
                Style::default().fg(Color::White),
            ),
        ]),
    ];

    for proj in selected.iter().take(8) {
        summary_lines.push(Line::from(vec![
            Span::styled("   ✅ ", Style::default().fg(Color::Green)),
            Span::styled(proj.name.clone(), Style::default().fg(Color::White)),
            Span::styled(
                format!(" @ {}", proj.version),
                Style::default().fg(Color::Gray),
            ),
        ]));
    }

    if selected.len() > 8 {
        summary_lines.push(Line::from(Span::styled(
            format!("   ... and {} more", selected.len() - 8),
            Style::default().fg(Color::Gray),
        )));
    }

    let summary_block = Paragraph::new(summary_lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Gray))
            .title(Span::styled(" Summary ", Style::default().fg(Color::White))),
    );
    frame.render_widget(summary_block, content_chunks[0]);

    let action_lines = vec![
        Line::from(vec![
            Span::styled("⚡ ", Style::default()),
            Span::styled("Actions", Style::default().fg(Color::White)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("   📄 ", Style::default().fg(Color::Cyan)),
            Span::styled("Create belaf/config.toml", Style::default().fg(Color::Gray)),
        ]),
        Line::from(vec![
            Span::styled("   📄 ", Style::default().fg(Color::Cyan)),
            Span::styled(
                "Create belaf/bootstrap.toml",
                Style::default().fg(Color::Gray),
            ),
        ]),
        Line::from(vec![
            Span::styled("   ✏️  ", Style::default().fg(Color::Yellow)),
            Span::styled(
                "Update project version files",
                Style::default().fg(Color::Gray),
            ),
        ]),
        Line::from(vec![
            Span::styled("   🏷️  ", Style::default().fg(Color::Green)),
            Span::styled("Create baseline Git tags", Style::default().fg(Color::Gray)),
        ]),
    ];

    let action_block = Paragraph::new(action_lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Gray))
            .title(Span::styled(
                " Will Execute ",
                Style::default().fg(Color::White),
            )),
    );
    frame.render_widget(action_block, content_chunks[1]);

    let hints = Line::from(vec![
        Span::styled("Enter/y", Style::default().fg(Color::Green)),
        Span::styled(" confirm  ", Style::default().fg(Color::Gray)),
        Span::styled("Backspace/n", Style::default().fg(Color::Yellow)),
        Span::styled(" go back  ", Style::default().fg(Color::Gray)),
        Span::styled("q", Style::default().fg(Color::Red)),
        Span::styled(" quit", Style::default().fg(Color::Gray)),
    ]);
    let hints_para = Paragraph::new(hints).alignment(ratatui::layout::Alignment::Center);
    frame.render_widget(hints_para, chunks[2]);
}
