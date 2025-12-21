use std::{collections::HashMap, fs, io, io::Write as _};

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
use crate::{
    atry,
    core::{
        git::repository::{PathMatcher, RepoPathBuf, Repository},
        project::DepRequirement,
        session::{AppBuilder, AppSession},
        ui::{
            components::toggle_panel::TogglePanel,
            utils::centered_rect,
        },
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
    PresetSelection,
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
    preset: Option<String>,
    preset_from_cli: bool,
    selected_preset_idx: usize,
    available_presets: Vec<String>,
    preset_toggle: TogglePanel,
    config_exists: bool,
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
            success_message: None,
            force,
            dirty_warning: None,
            preset,
            preset_from_cli,
            selected_preset_idx,
            available_presets,
            preset_toggle: TogglePanel::default(),
            config_exists: false,
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

    let result = run_wizard_loop(&mut terminal, &mut state, &repo);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;

    result
}

fn run_wizard_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut WizardState,
    repo: &Repository,
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
        WizardStep::PresetSelection => render_preset_selection(frame, chunks[1], state),
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
        WizardStep::PresetSelection => "Step 1/4: Changelog Preset",
        WizardStep::ProjectSelection => "Step 2/4: Project Selection",
        WizardStep::UpstreamConfig => "Step 3/4: Upstream Configuration",
        WizardStep::Confirmation => "Step 4/4: Confirmation",
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
                Style::default()
                    .fg(Color::Red)
                    .add_modifier(Modifier::BOLD),
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
                Style::default().fg(Color::DarkGray),
            )),
        ];
        let logo_para = Paragraph::new(logo).alignment(ratatui::layout::Alignment::Center);
        frame.render_widget(logo_para, chunks[0]);

        let warning_header = vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                "⚠️  RECONFIGURE MODE  ⚠️",
                Style::default()
                    .fg(Color::Red)
                    .add_modifier(Modifier::BOLD),
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
                Span::styled("Detected: ", Style::default().fg(Color::DarkGray)),
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
            Span::styled("Press ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "ENTER",
                Style::default()
                    .fg(Color::Red)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" to reconfigure  •  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "Q",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" to quit", Style::default().fg(Color::DarkGray)),
        ])];
        let action_para =
            Paragraph::new(action_text).alignment(ratatui::layout::Alignment::Center);
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
                Style::default().fg(Color::DarkGray),
            )),
        ];
        let logo_para = Paragraph::new(logo).alignment(ratatui::layout::Alignment::Center);
        frame.render_widget(logo_para, chunks[0]);

        let mut info_lines = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("📦 ", Style::default()),
                Span::styled("Detected: ", Style::default().fg(Color::DarkGray)),
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
                Style::default().fg(Color::DarkGray),
            ),
        ]));
        info_lines.push(Line::from(vec![
            Span::styled("  → ", Style::default().fg(Color::Cyan)),
            Span::styled(
                "Project configuration",
                Style::default().fg(Color::DarkGray),
            ),
        ]));
        info_lines.push(Line::from(vec![
            Span::styled("  → ", Style::default().fg(Color::Cyan)),
            Span::styled("Repository setup", Style::default().fg(Color::DarkGray)),
        ]));

        let info_para = Paragraph::new(info_lines).alignment(ratatui::layout::Alignment::Center);
        frame.render_widget(info_para, chunks[1]);

        let action_text = vec![Line::from(vec![
            Span::styled("Press ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "ENTER",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" to start  •  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "Q",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" to quit", Style::default().fg(Color::DarkGray)),
        ])];
        let action_para =
            Paragraph::new(action_text).alignment(ratatui::layout::Alignment::Center);
        frame.render_widget(action_para, chunks[2]);
    }
}

fn render_preset_selection(frame: &mut Frame, area: Rect, state: &mut WizardState) {
    use crate::core::embed::{EmbeddedConfig, EmbeddedPresets};
    use ratatui::widgets::{Padding, Wrap};

    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(area);

    let items: Vec<ListItem> = state
        .available_presets
        .iter()
        .enumerate()
        .map(|(idx, name)| {
            let description = match name.as_str() {
                "default" => "Standard Conventional Commits",
                "keepachangelog" => "Keep a Changelog spec",
                "flat" => "What's Changed - flat list",
                "minimal" => "Version + Date only",
                _ => "Custom preset",
            };
            let text = format!("{}\n  {}", name, description);
            let style = if idx == state.selected_preset_idx {
                Style::default()
                    .bg(Color::DarkGray)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(text).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Select Changelog Preset "),
    );

    frame.render_widget(list, main_chunks[0]);

    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(main_chunks[1]);

    let preset_name = state.selected_preset_name().to_string();
    state.preset_toggle.render(frame, right_chunks[0], &preset_name);

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
                    .title(" TOML Source ")
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
                    .title(" Changelog Preview ")
                    .padding(Padding::horizontal(1)),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(paragraph, right_chunks[1]);
    }
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
        WizardStep::PresetSelection => "↑/↓: Navigate  m: Toggle View  ENTER: Select  ESC: Back",
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
