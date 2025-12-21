use anyhow::{Context, Result};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use owo_colors::OwoColorize;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Padding, Paragraph, Wrap},
    Frame, Terminal,
};
use std::io;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::Duration;
use tracing::info;

use std::path::Path;

use crate::core::{
    bump::{BumpConfig, BumpRecommendation},
    changelog::{ChangelogConfig, Commit, GitConfig},
    config::syntax::{BumpConfiguration, ChangelogConfiguration},
    ecosystem::types::EcosystemType,
    git::repository::RepoPathBuf,
    session::AppSession,
    ui::{
        components::toggle_panel::TogglePanel,
        markdown,
        utils::centered_rect,
    },
    workflow::{
        generate_changelog_entry, BumpChoice, PrepareContext, ProjectCandidate, ProjectSelection,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum WizardStep {
    ProjectSelection,
    ProjectConfig { project_index: usize },
    Confirmation,
}

struct ProjectItem {
    candidate: ProjectCandidate,
    selected: bool,
    chosen_bump: Option<BumpChoice>,
    cached_changelog: Option<String>,
    existing_changelog: String,
}

impl ProjectItem {
    fn from_candidate(candidate: ProjectCandidate, existing_changelog: String) -> Self {
        Self {
            candidate,
            selected: true,
            chosen_bump: None,
            cached_changelog: None,
            existing_changelog,
        }
    }

    fn name(&self) -> &str {
        &self.candidate.name
    }

    fn current_version(&self) -> &str {
        &self.candidate.current_version
    }

    fn commit_count(&self) -> usize {
        self.candidate.commit_count
    }

    fn suggested_bump(&self) -> BumpRecommendation {
        self.candidate.suggested_bump
    }

    fn commits(&self) -> &[Commit] {
        &self.candidate.commits
    }

    fn project_type(&self) -> EcosystemType {
        self.candidate.ecosystem
    }

    fn effective_bump(&self) -> BumpChoice {
        self.chosen_bump.unwrap_or(BumpChoice::Auto)
    }

    fn effective_bump_str(&self) -> &'static str {
        self.effective_bump().resolve(self.candidate.suggested_bump)
    }
}

fn calculate_next_version(current: &str, recommendation: BumpRecommendation) -> String {
    match recommendation {
        BumpRecommendation::Major => calculate_major_version(current),
        BumpRecommendation::Minor => calculate_minor_version(current),
        BumpRecommendation::Patch => calculate_patch_version(current),
        BumpRecommendation::None => current.to_string(),
    }
}

fn calculate_major_version(current: &str) -> String {
    if let Ok(mut v) = semver::Version::parse(current) {
        v.major += 1;
        v.minor = 0;
        v.patch = 0;
        v.pre = semver::Prerelease::EMPTY;
        v.build = semver::BuildMetadata::EMPTY;
        v.to_string()
    } else {
        format!("{}+1.0.0", current)
    }
}

fn calculate_minor_version(current: &str) -> String {
    if let Ok(mut v) = semver::Version::parse(current) {
        v.minor += 1;
        v.patch = 0;
        v.pre = semver::Prerelease::EMPTY;
        v.build = semver::BuildMetadata::EMPTY;
        v.to_string()
    } else {
        format!("{}+0.1.0", current)
    }
}

fn calculate_patch_version(current: &str) -> String {
    if let Ok(mut v) = semver::Version::parse(current) {
        v.patch += 1;
        v.pre = semver::Prerelease::EMPTY;
        v.build = semver::BuildMetadata::EMPTY;
        v.to_string()
    } else {
        format!("{}+0.0.1", current)
    }
}

struct WizardState {
    step: WizardStep,
    projects: Vec<ProjectItem>,
    project_list_state: ListState,
    bump_list_state: ListState,
    show_changelog: bool,
    show_help: bool,
    loading_changelog: bool,
    loading_frame: usize,
    loading_receiver: Option<Receiver<String>>,
    changelog_toggle: TogglePanel,
    changelog_config: ChangelogConfiguration,
    bump_config: BumpConfiguration,
}

impl WizardState {
    fn new(
        projects: Vec<ProjectItem>,
        changelog_config: ChangelogConfiguration,
        bump_config: BumpConfiguration,
    ) -> Self {
        let mut project_list_state = ListState::default();
        if !projects.is_empty() {
            project_list_state.select(Some(0));
        }

        let mut bump_list_state = ListState::default();
        bump_list_state.select(Some(0));

        Self {
            step: WizardStep::ProjectSelection,
            projects,
            project_list_state,
            bump_list_state,
            show_changelog: false,
            show_help: false,
            loading_changelog: false,
            loading_frame: 0,
            loading_receiver: None,
            changelog_toggle: TogglePanel::default(),
            changelog_config,
            bump_config,
        }
    }

    fn toggle_markdown_view(&mut self) {
        self.changelog_toggle.toggle();
    }

    fn handle_mouse_click(&mut self, x: u16, y: u16) -> bool {
        self.changelog_toggle.handle_click(x, y)
    }

    fn is_loading(&self) -> bool {
        self.loading_changelog
    }

    fn tick_loading(&mut self) {
        self.loading_frame = (self.loading_frame + 1) % 8;
    }

    fn loading_spinner(&self) -> &'static str {
        const FRAMES: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
        FRAMES[self.loading_frame]
    }

    fn start_background_changelog_generation(&mut self) {
        let project = match self.get_current_project() {
            Some(p) => p,
            None => return,
        };

        if project.cached_changelog.is_some() {
            self.loading_changelog = false;
            self.show_changelog = true;
            return;
        }

        let commits = project.commits().to_vec();
        let current_version = project.current_version().to_string();
        let chosen_bump = project.chosen_bump.unwrap_or(BumpChoice::Auto);
        let suggested_bump = project.suggested_bump();

        let new_version = match chosen_bump {
            BumpChoice::Auto => calculate_next_version(&current_version, suggested_bump),
            BumpChoice::Major => calculate_major_version(&current_version),
            BumpChoice::Minor => calculate_minor_version(&current_version),
            BumpChoice::Patch => calculate_patch_version(&current_version),
        };

        let (tx, rx) = mpsc::channel();
        self.loading_receiver = Some(rx);

        let git_config = GitConfig::from_user_config(&self.changelog_config);
        let changelog_config = ChangelogConfig::from_user_config(&self.changelog_config);
        let bump_config = BumpConfig::from_user_config(&self.bump_config);

        thread::spawn(move || {
            let changelog = generate_changelog_entry(
                &new_version,
                &commits,
                &git_config,
                &changelog_config,
                &bump_config,
            )
            .unwrap_or_else(|_| "Failed to generate changelog".to_string());

            let _ = tx.send(changelog);
        });
    }

    fn check_loading_complete(&mut self) -> bool {
        if let Some(ref receiver) = self.loading_receiver {
            match receiver.try_recv() {
                Ok(changelog) => {
                    if let Some(project) = self.get_current_project_mut() {
                        project.cached_changelog = Some(changelog);
                    }
                    self.loading_changelog = false;
                    self.loading_receiver = None;
                    self.show_changelog = true;
                    true
                }
                Err(TryRecvError::Empty) => false,
                Err(TryRecvError::Disconnected) => {
                    self.loading_changelog = false;
                    self.loading_receiver = None;
                    self.show_changelog = true;
                    true
                }
            }
        } else {
            false
        }
    }

    fn cancel_loading(&mut self) {
        self.loading_changelog = false;
        self.loading_receiver = None;
    }

    fn get_current_project(&self) -> Option<&ProjectItem> {
        if let WizardStep::ProjectConfig { project_index } = self.step {
            self.projects
                .iter()
                .filter(|p| p.selected)
                .nth(project_index)
        } else {
            None
        }
    }

    fn get_current_project_mut(&mut self) -> Option<&mut ProjectItem> {
        if let WizardStep::ProjectConfig { project_index } = self.step {
            self.projects
                .iter_mut()
                .filter(|p| p.selected)
                .nth(project_index)
        } else {
            None
        }
    }

    fn selected_count(&self) -> usize {
        self.projects.iter().filter(|p| p.selected).count()
    }

    fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
    }

    fn next_step(&mut self) -> bool {
        if self.loading_changelog {
            return false;
        }

        match &self.step {
            WizardStep::ProjectSelection => {
                if self.selected_count() == 0 {
                    return false;
                }
                self.step = WizardStep::ProjectConfig { project_index: 0 };
                self.show_changelog = false;
                self.bump_list_state.select(Some(0));
                true
            }
            WizardStep::ProjectConfig { project_index } => {
                if !self.show_changelog {
                    if let Some(selected) = self.bump_list_state.selected() {
                        if let Some(project) = self.get_current_project_mut() {
                            project.chosen_bump = Some(BumpChoice::all()[selected]);
                        }
                    }
                    self.loading_changelog = true;
                    self.start_background_changelog_generation();
                    true
                } else {
                    if *project_index + 1 < self.selected_count() {
                        self.step = WizardStep::ProjectConfig {
                            project_index: project_index + 1,
                        };
                        self.show_changelog = false;
                        self.bump_list_state.select(Some(0));
                    } else {
                        self.step = WizardStep::Confirmation;
                    }
                    true
                }
            }
            WizardStep::Confirmation => false,
        }
    }

    fn prev_step(&mut self) -> bool {
        match &self.step {
            WizardStep::ProjectSelection => false,
            WizardStep::ProjectConfig { project_index } => {
                if self.show_changelog {
                    self.show_changelog = false;
                    true
                } else if *project_index == 0 {
                    self.step = WizardStep::ProjectSelection;
                    true
                } else {
                    self.step = WizardStep::ProjectConfig {
                        project_index: project_index - 1,
                    };
                    self.show_changelog = true;
                    true
                }
            }
            WizardStep::Confirmation => {
                let last_idx = self.selected_count().saturating_sub(1);
                self.step = WizardStep::ProjectConfig {
                    project_index: last_idx,
                };
                self.show_changelog = true;
                true
            }
        }
    }

    fn selected_projects(&self) -> Vec<&ProjectItem> {
        self.projects.iter().filter(|p| p.selected).collect()
    }

    fn handle_key_project_selection(&mut self, key: KeyCode) -> bool {
        match key {
            KeyCode::Up => {
                if let Some(selected) = self.project_list_state.selected() {
                    if selected > 0 {
                        self.project_list_state.select(Some(selected - 1));
                    }
                }
            }
            KeyCode::Down => {
                if let Some(selected) = self.project_list_state.selected() {
                    if selected < self.projects.len() - 1 {
                        self.project_list_state.select(Some(selected + 1));
                    }
                }
            }
            KeyCode::Char(' ') => {
                if let Some(selected) = self.project_list_state.selected() {
                    self.projects[selected].selected = !self.projects[selected].selected;
                }
            }
            KeyCode::Char('a') => {
                let all_selected = self.projects.iter().all(|p| p.selected);
                for project in &mut self.projects {
                    project.selected = !all_selected;
                }
            }
            KeyCode::Enter => {
                if self.selected_projects().is_empty() {
                    return false;
                }
                return self.next_step();
            }
            _ => {}
        }
        false
    }

    fn handle_key_project_config(&mut self, key: KeyCode) -> bool {
        match key {
            KeyCode::Tab => {
                self.show_changelog = !self.show_changelog;
                if !self.show_changelog {
                    if let Some(project) = self.get_current_project() {
                        if let Some(chosen) = project.chosen_bump {
                            let idx = BumpChoice::all()
                                .iter()
                                .position(|s| *s == chosen)
                                .unwrap_or(0);
                            self.bump_list_state.select(Some(idx));
                        }
                    }
                }
                false
            }
            KeyCode::Up if !self.show_changelog => {
                if let Some(selected) = self.bump_list_state.selected() {
                    if selected > 0 {
                        self.bump_list_state.select(Some(selected - 1));
                    }
                }
                false
            }
            KeyCode::Down if !self.show_changelog => {
                if let Some(selected) = self.bump_list_state.selected() {
                    let strategies = BumpChoice::all();
                    if selected < strategies.len() - 1 {
                        self.bump_list_state.select(Some(selected + 1));
                    }
                }
                false
            }
            KeyCode::Enter => self.next_step(),
            KeyCode::Backspace | KeyCode::Esc => self.prev_step(),
            _ => false,
        }
    }

    fn handle_key_confirmation(&mut self, key: KeyCode) -> (bool, bool) {
        match key {
            KeyCode::Enter => (false, true),
            KeyCode::Backspace | KeyCode::Esc => (self.prev_step(), false),
            _ => (false, false),
        }
    }
}

pub fn run_with_overrides(project_overrides: Option<Vec<String>>) -> Result<i32> {
    info!("starting interactive TUI wizard for release preparation");

    let mut sess =
        AppSession::initialize_default().context("could not initialize app and project graph")?;

    let mut ctx = PrepareContext::initialize(&mut sess, true)?;
    ctx.discover_projects()?;

    if !ctx.has_candidates() {
        ctx.cleanup();
        print_no_changes_message();
        return Ok(0);
    }

    let projects: Vec<ProjectItem> = ctx
        .candidates
        .iter()
        .map(|candidate| {
            let prefix = &candidate.prefix;
            let changelog_rel_path = if prefix.is_empty() {
                "CHANGELOG.md".to_string()
            } else {
                format!("{}/CHANGELOG.md", prefix)
            };
            let changelog_repo_path = RepoPathBuf::new(changelog_rel_path.as_bytes());
            let changelog_path = ctx.resolve_workdir(changelog_repo_path.as_ref());
            let existing_changelog = parse_existing_changelog(&changelog_path).unwrap_or_default();
            ProjectItem::from_candidate(candidate.clone(), existing_changelog)
        })
        .collect();

    let mut projects = projects;
    if let Some(ref overrides) = project_overrides {
        apply_project_overrides_to_items(&mut projects, overrides)?;
    }

    let wizard_result = run_wizard_ui(
        projects,
        ctx.changelog_config.clone(),
        ctx.bump_config.clone(),
    )?;

    let selected_items = match wizard_result {
        Some(items) => items,
        None => {
            info!("release preparation cancelled by user");
            ctx.cleanup();
            return Ok(1);
        }
    };

    if selected_items.is_empty() {
        ctx.cleanup();
        println!();
        println!("{} No projects selected.", "ℹ".cyan().bold());
        println!();
        return Ok(0);
    }

    let selections: Vec<ProjectSelection> = selected_items
        .into_iter()
        .map(|item| ProjectSelection {
            candidate: item.candidate,
            bump_choice: item.chosen_bump.unwrap_or(BumpChoice::Auto),
            cached_changelog: item.cached_changelog,
        })
        .collect();

    let pr_url = ctx.finalize(selections)?;

    println!();
    println!("{} Release preparation complete!", "✓".green().bold());
    println!();
    println!("  {} Pull request created:", "→".cyan());
    println!("    {}", pr_url.cyan().underline());
    println!();

    Ok(0)
}

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

fn run_wizard_ui(
    projects: Vec<ProjectItem>,
    changelog_config: ChangelogConfiguration,
    bump_config: BumpConfiguration,
) -> Result<Option<Vec<ProjectItem>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut state = WizardState::new(projects, changelog_config, bump_config);
    let result = run_app(&mut terminal, &mut state);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if result? {
        let selected = state.projects.into_iter().filter(|p| p.selected).collect();
        Ok(Some(selected))
    } else {
        Ok(None)
    }
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut WizardState,
) -> Result<bool> {
    loop {
        terminal.draw(|f| ui(f, state))?;

        if state.is_loading() {
            state.check_loading_complete();

            if event::poll(Duration::from_millis(100))? {
                if let Event::Key(KeyEvent {
                    code,
                    kind: KeyEventKind::Press,
                    ..
                }) = event::read()?
                {
                    if code == KeyCode::Esc {
                        state.cancel_loading();
                    }
                }
            }

            state.tick_loading();
            continue;
        }

        match event::read()? {
            Event::Key(KeyEvent {
                code,
                kind: KeyEventKind::Press,
                ..
            }) => {
                if code == KeyCode::Char('q') || code == KeyCode::Char('c') {
                    return Ok(false);
                }

                if code == KeyCode::Char('?') || code == KeyCode::Char('h') {
                    state.toggle_help();
                    continue;
                }

                if state.show_help {
                    state.toggle_help();
                    continue;
                }

                if code == KeyCode::Char('m') && state.show_changelog {
                    state.toggle_markdown_view();
                    continue;
                }

                let result = match &state.step {
                    WizardStep::ProjectSelection => state.handle_key_project_selection(code),
                    WizardStep::ProjectConfig { .. } => state.handle_key_project_config(code),
                    WizardStep::Confirmation => {
                        let (step_changed, confirmed) = state.handle_key_confirmation(code);
                        if confirmed {
                            return Ok(true);
                        }
                        step_changed
                    }
                };

                if result {
                    continue;
                }
            }
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row,
                ..
            }) => {
                if state.show_changelog && state.handle_mouse_click(column, row) {
                    continue;
                }
            }
            _ => {}
        }
    }
}

fn ui(f: &mut Frame, state: &mut WizardState) {
    render_step(f, f.area(), state);

    if state.show_help {
        render_help_popup(f, state);
    }
}

fn render_step(f: &mut Frame, area: Rect, state: &mut WizardState) {
    let step = state.step.clone();
    let show_changelog = state.show_changelog;
    match step {
        WizardStep::ProjectSelection => render_project_selection(f, area, state),
        WizardStep::ProjectConfig { .. } => {
            if show_changelog {
                render_project_changelog(f, area, state);
            } else {
                render_project_bump_strategy(f, area, state);
            }
        }
        WizardStep::Confirmation => render_confirmation(f, area, state),
    }
}

fn render_project_selection(f: &mut Frame, area: Rect, state: &mut WizardState) {
    let selected_count = state.projects.iter().filter(|p| p.selected).count();
    let total_count = state.projects.len();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " Step 1: Project Selection ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

    let inner_area = block.inner(area);
    f.render_widget(block, area);

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
            Span::styled("🚀 ", Style::default()),
            Span::styled(
                "Select projects to prepare for release",
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("   Selected: ", Style::default().fg(Color::DarkGray)),
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
                Style::default().fg(Color::DarkGray),
            ),
        ]),
    ];
    let header = Paragraph::new(header_lines).alignment(ratatui::layout::Alignment::Center);
    f.render_widget(header, chunks[0]);

    let items: Vec<ListItem> = state
        .projects
        .iter()
        .enumerate()
        .map(|(idx, project)| {
            let is_current = state.project_list_state.selected() == Some(idx);
            let checkbox = if project.selected { "✅" } else { "⬜" };
            let (suggestion_text, suggestion_color) = match project.suggested_bump() {
                BumpRecommendation::Major => ("MAJOR", Color::Red),
                BumpRecommendation::Minor => ("MINOR", Color::Yellow),
                BumpRecommendation::Patch => ("PATCH", Color::Green),
                BumpRecommendation::None => ("", Color::DarkGray),
            };

            let lines = vec![Line::from(vec![
                Span::styled(format!(" {} ", checkbox), Style::default()),
                Span::styled(
                    project.name(),
                    if is_current {
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else if project.selected {
                        Style::default().fg(Color::Green)
                    } else {
                        Style::default().fg(Color::White)
                    },
                ),
                Span::styled(
                    format!(" ({} commits)", project.commit_count()),
                    Style::default().fg(Color::DarkGray),
                ),
                if !suggestion_text.is_empty() {
                    Span::styled(
                        format!("  → {}", suggestion_text),
                        Style::default().fg(suggestion_color),
                    )
                } else {
                    Span::raw("")
                },
            ])];

            let style = if is_current {
                Style::default().bg(Color::Rgb(40, 40, 50))
            } else {
                Style::default()
            };
            ListItem::new(lines).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray))
                .title(Span::styled(" Projects ", Style::default().fg(Color::White))),
        )
        .highlight_symbol("");

    f.render_stateful_widget(list, chunks[1], &mut state.project_list_state);

    let hints = Line::from(vec![
        Span::styled("↑↓", Style::default().fg(Color::Cyan)),
        Span::styled(" navigate  ", Style::default().fg(Color::DarkGray)),
        Span::styled("Space", Style::default().fg(Color::Cyan)),
        Span::styled(" toggle  ", Style::default().fg(Color::DarkGray)),
        Span::styled("a", Style::default().fg(Color::Green)),
        Span::styled(" all  ", Style::default().fg(Color::DarkGray)),
        Span::styled("Enter", Style::default().fg(Color::Green)),
        Span::styled(" continue  ", Style::default().fg(Color::DarkGray)),
        Span::styled("?", Style::default().fg(Color::Yellow)),
        Span::styled(" help  ", Style::default().fg(Color::DarkGray)),
        Span::styled("q", Style::default().fg(Color::Red)),
        Span::styled(" quit", Style::default().fg(Color::DarkGray)),
    ]);
    let hints_para = Paragraph::new(hints).alignment(ratatui::layout::Alignment::Center);
    f.render_widget(hints_para, chunks[2]);
}

fn render_project_bump_strategy(f: &mut Frame, area: Rect, state: &mut WizardState) {
    let strategies = BumpChoice::all();

    let project = match state.get_current_project() {
        Some(p) => p,
        None => return,
    };

    let project_name = project.name().to_string();
    let current_version = project.current_version().to_string();
    let suggested_bump = project.suggested_bump();
    let commits = project.commits().to_vec();

    let selected_index = state.bump_list_state.selected().unwrap_or(0);
    let selected_strategy = strategies
        .get(selected_index)
        .copied()
        .unwrap_or(BumpChoice::Auto);

    let current_project_idx = match &state.step {
        WizardStep::ProjectConfig { project_index } => *project_index + 1,
        _ => 1,
    };
    let total_projects = state.selected_count();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            format!(
                " Step 2: Configure Release ({}/{}) ",
                current_project_idx, total_projects
            ),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

    let inner_area = block.inner(area);
    f.render_widget(block, area);

    let outer_chunks = Layout::default()
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
            Span::styled("📦 ", Style::default()),
            Span::styled(
                project_name.clone(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  v{}", current_version),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                format!("  ({} commits)", commits.len()),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
    ];
    let header = Paragraph::new(header_lines).alignment(ratatui::layout::Alignment::Center);
    f.render_widget(header, outer_chunks[0]);

    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(outer_chunks[1]);

    let items: Vec<ListItem> = strategies
        .iter()
        .enumerate()
        .map(|(idx, strategy)| {
            let is_selected = idx == selected_index;
            let next_ver = match strategy {
                BumpChoice::Auto => calculate_next_version(&current_version, suggested_bump),
                BumpChoice::Major => calculate_major_version(&current_version),
                BumpChoice::Minor => calculate_minor_version(&current_version),
                BumpChoice::Patch => calculate_patch_version(&current_version),
            };
            let (icon, color) = match strategy {
                BumpChoice::Auto => ("🔄", Color::Cyan),
                BumpChoice::Major => ("🔴", Color::Red),
                BumpChoice::Minor => ("🟡", Color::Yellow),
                BumpChoice::Patch => ("🟢", Color::Green),
            };
            let lines = vec![Line::from(vec![
                Span::styled(format!(" {} ", icon), Style::default()),
                Span::styled(
                    strategy.as_str(),
                    if is_selected {
                        Style::default().fg(color).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    },
                ),
                Span::styled(
                    format!("  →  {}", next_ver),
                    Style::default().fg(Color::DarkGray),
                ),
            ])];
            let style = if is_selected {
                Style::default().bg(Color::Rgb(40, 40, 50))
            } else {
                Style::default()
            };
            ListItem::new(lines).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray))
                .title(Span::styled(
                    " Version Bump ",
                    Style::default().fg(Color::White),
                )),
        )
        .highlight_symbol("");

    f.render_stateful_widget(list, main_chunks[0], &mut state.bump_list_state);

    if state.is_loading() {
        let loading_content = build_loading_panel(state, &commits);
        let loading_panel = Paragraph::new(loading_content)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Yellow))
                    .title(Span::styled(
                        " 🤖 Generating Changelog... ",
                        Style::default().fg(Color::Yellow),
                    )),
            )
            .wrap(Wrap { trim: true });
        f.render_widget(loading_panel, main_chunks[1]);
    } else {
        let detail_content = build_detail_panel(
            &selected_strategy,
            &current_version,
            suggested_bump,
            &commits,
        );

        let detail_panel = Paragraph::new(detail_content)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray))
                    .title(Span::styled(
                        format!(" {} Details ", selected_strategy.as_str()),
                        Style::default().fg(Color::White),
                    )),
            )
            .wrap(Wrap { trim: true });

        f.render_widget(detail_panel, main_chunks[1]);
    }

    let hints = if state.is_loading() {
        Line::from(vec![
            Span::styled("⏳ ", Style::default().fg(Color::Yellow)),
            Span::styled(
                "Generating changelog...  ",
                Style::default().fg(Color::Yellow),
            ),
            Span::styled("Esc", Style::default().fg(Color::Red)),
            Span::styled(" cancel", Style::default().fg(Color::DarkGray)),
        ])
    } else {
        Line::from(vec![
            Span::styled("↑↓", Style::default().fg(Color::Cyan)),
            Span::styled(" select  ", Style::default().fg(Color::DarkGray)),
            Span::styled("Tab", Style::default().fg(Color::Cyan)),
            Span::styled(" preview changelog  ", Style::default().fg(Color::DarkGray)),
            Span::styled("Enter", Style::default().fg(Color::Green)),
            Span::styled(" next  ", Style::default().fg(Color::DarkGray)),
            Span::styled("Esc", Style::default().fg(Color::Yellow)),
            Span::styled(" back  ", Style::default().fg(Color::DarkGray)),
            Span::styled("q", Style::default().fg(Color::Red)),
            Span::styled(" quit", Style::default().fg(Color::DarkGray)),
        ])
    };
    let hints_para = Paragraph::new(hints).alignment(ratatui::layout::Alignment::Center);
    f.render_widget(hints_para, outer_chunks[2]);
}

fn build_loading_panel(state: &WizardState, commits: &[Commit]) -> Text<'static> {
    let spinner = state.loading_spinner();
    let commit_count = commits.len();

    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            format!("{} ", spinner),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Analyzing commits with Claude AI",
            Style::default().fg(Color::Cyan),
        ),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!(
            "   Processing {} commit{}...",
            commit_count,
            if commit_count == 1 { "" } else { "s" }
        ),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "   This may take a few seconds.",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "─".repeat(40),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "   Press Esc to cancel",
        Style::default().fg(Color::Yellow),
    )));

    Text::from(lines)
}

fn build_detail_panel(
    strategy: &BumpChoice,
    current_version: &str,
    suggested_bump: BumpRecommendation,
    commits: &[Commit],
) -> Text<'static> {
    let mut lines: Vec<Line> = Vec::new();

    let next_version = match strategy {
        BumpChoice::Auto => calculate_next_version(current_version, suggested_bump),
        BumpChoice::Major => calculate_major_version(current_version),
        BumpChoice::Minor => calculate_minor_version(current_version),
        BumpChoice::Patch => calculate_patch_version(current_version),
    };

    lines.push(Line::from(vec![
        Span::styled("Version: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            current_version.to_string(),
            Style::default().fg(Color::Yellow),
        ),
        Span::styled(" → ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            next_version,
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(""));

    match strategy {
        BumpChoice::Auto => {
            let bump_name = match suggested_bump {
                BumpRecommendation::Major => "MAJOR (breaking changes)",
                BumpRecommendation::Minor => "MINOR (new features)",
                BumpRecommendation::Patch => "PATCH (bug fixes)",
                BumpRecommendation::None => "NO CHANGES",
            };
            lines.push(Line::from(vec![
                Span::styled("Detected: ", Style::default().fg(Color::DarkGray)),
                Span::styled(bump_name.to_string(), Style::default().fg(Color::Cyan)),
            ]));
        }
        BumpChoice::Major => {
            lines.push(Line::from(Span::styled(
                "⚠ Breaking Change Release",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(Span::styled(
                "  Consider updating migration guides",
                Style::default().fg(Color::DarkGray),
            )));
        }
        BumpChoice::Minor => {
            lines.push(Line::from(Span::styled(
                "✓ Feature Release",
                Style::default().fg(Color::Green),
            )));
            lines.push(Line::from(Span::styled(
                "  Backwards compatible additions",
                Style::default().fg(Color::DarkGray),
            )));
        }
        BumpChoice::Patch => {
            lines.push(Line::from(Span::styled(
                "✓ Patch Release",
                Style::default().fg(Color::Blue),
            )));
            lines.push(Line::from(Span::styled(
                "  Bug fixes only",
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    lines.push(Line::from(""));

    let (feat_count, fix_count, breaking_count, other_count) = count_commit_types(commits);

    lines.push(Line::from(Span::styled(
        "Commit Analysis:",
        Style::default().add_modifier(Modifier::BOLD),
    )));

    if breaking_count > 0 {
        lines.push(Line::from(vec![
            Span::styled("  breaking:  ", Style::default().fg(Color::Red)),
            Span::styled(
                format!("{}", breaking_count),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
        ]));
    }
    if feat_count > 0 {
        lines.push(Line::from(vec![
            Span::styled("  feat:      ", Style::default().fg(Color::Green)),
            Span::styled(format!("{}", feat_count), Style::default().fg(Color::Green)),
        ]));
    }
    if fix_count > 0 {
        lines.push(Line::from(vec![
            Span::styled("  fix:       ", Style::default().fg(Color::Yellow)),
            Span::styled(format!("{}", fix_count), Style::default().fg(Color::Yellow)),
        ]));
    }
    if other_count > 0 {
        lines.push(Line::from(vec![
            Span::styled("  other:     ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", other_count),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Commits:",
        Style::default().add_modifier(Modifier::BOLD),
    )));

    for (i, commit) in commits.iter().take(8).enumerate() {
        let msg = &commit.message;
        let truncated = if msg.len() > 50 {
            format!("{}...", &msg[..47])
        } else {
            msg.clone()
        };

        let color = if msg.starts_with("feat") {
            Color::Green
        } else if msg.starts_with("fix") {
            Color::Yellow
        } else if msg.contains("BREAKING") || msg.contains("!:") {
            Color::Red
        } else {
            Color::DarkGray
        };

        lines.push(Line::from(Span::styled(
            format!("  {} {}", if i < 9 { "•" } else { " " }, truncated),
            Style::default().fg(color),
        )));
    }

    if commits.len() > 8 {
        lines.push(Line::from(Span::styled(
            format!("  ... and {} more", commits.len() - 8),
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Press Tab to preview full changelog",
        Style::default().fg(Color::DarkGray),
    )));

    Text::from(lines)
}

fn count_commit_types(commits: &[Commit]) -> (usize, usize, usize, usize) {
    let mut feat = 0;
    let mut fix = 0;
    let mut breaking = 0;
    let mut other = 0;

    for commit in commits {
        let msg = &commit.message;
        let lower = msg.to_lowercase();
        if msg.contains("BREAKING") || msg.contains("!:") {
            breaking += 1;
        } else if lower.starts_with("feat") {
            feat += 1;
        } else if lower.starts_with("fix") {
            fix += 1;
        } else {
            other += 1;
        }
    }

    (feat, fix, breaking, other)
}

fn render_project_changelog(f: &mut Frame, area: Rect, state: &mut WizardState) {
    let current_project = match state.get_current_project() {
        Some(p) => p,
        None => return,
    };

    let chosen_bump = current_project.chosen_bump.unwrap_or(BumpChoice::Auto);
    let current_version = current_project.current_version();
    let suggested_bump = current_project.suggested_bump();
    let new_version = match chosen_bump {
        BumpChoice::Auto => calculate_next_version(current_version, suggested_bump),
        BumpChoice::Major => calculate_major_version(current_version),
        BumpChoice::Minor => calculate_minor_version(current_version),
        BumpChoice::Patch => calculate_patch_version(current_version),
    };

    let new_entry = current_project
        .cached_changelog
        .as_deref()
        .unwrap_or("Loading changelog...");

    let mut changelog_content = String::from(new_entry);

    if !current_project.existing_changelog.is_empty() {
        changelog_content.push_str("\n\n");
        changelog_content.push_str(&current_project.existing_changelog);
    }

    let current_project_idx = match &state.step {
        WizardStep::ProjectConfig { project_index } => *project_index + 1,
        _ => 1,
    };
    let total_projects = state.selected_count();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            format!(
                " Step 2: Changelog Preview ({}/{}) ",
                current_project_idx, total_projects
            ),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

    let inner_area = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(inner_area);

    let header_lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("📝 ", Style::default()),
            Span::styled(
                current_project.name(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {} → {}", current_version, new_version),
                Style::default().fg(Color::Green),
            ),
        ]),
    ];
    let header = Paragraph::new(header_lines).alignment(ratatui::layout::Alignment::Center);
    f.render_widget(header, chunks[0]);

    let title = format!(
        "{} ({} → {})",
        current_project.name(),
        current_version,
        new_version
    );
    state.changelog_toggle.render(f, chunks[1], &title);

    if state.changelog_toggle.is_right() {
        let raw_text = Text::from(changelog_content.clone());
        let paragraph = Paragraph::new(raw_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray))
                    .title(Span::styled(
                        " Markdown Source ",
                        Style::default().fg(Color::Magenta),
                    ))
                    .padding(Padding::horizontal(1)),
            )
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(Color::Rgb(180, 180, 180)));
        f.render_widget(paragraph, chunks[2]);
    } else {
        let markdown_text = markdown::render_markdown(&changelog_content);
        let paragraph = Paragraph::new(markdown_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray))
                    .title(Span::styled(
                        " Rendered Preview ",
                        Style::default().fg(Color::Cyan),
                    ))
                    .padding(Padding::horizontal(1)),
            )
            .wrap(Wrap { trim: false });
        f.render_widget(paragraph, chunks[2]);
    }

    let hints = Line::from(vec![
        Span::styled("m", Style::default().fg(Color::Cyan)),
        Span::styled(" toggle view  ", Style::default().fg(Color::DarkGray)),
        Span::styled("Tab", Style::default().fg(Color::Cyan)),
        Span::styled(" back to bump  ", Style::default().fg(Color::DarkGray)),
        Span::styled("Enter", Style::default().fg(Color::Green)),
        Span::styled(" next  ", Style::default().fg(Color::DarkGray)),
        Span::styled("Esc", Style::default().fg(Color::Yellow)),
        Span::styled(" back  ", Style::default().fg(Color::DarkGray)),
        Span::styled("q", Style::default().fg(Color::Red)),
        Span::styled(" quit", Style::default().fg(Color::DarkGray)),
    ]);
    let hints_para = Paragraph::new(hints).alignment(ratatui::layout::Alignment::Center);
    f.render_widget(hints_para, chunks[3]);
}

fn render_confirmation(f: &mut Frame, area: Rect, state: &WizardState) {
    let selected_projects = state.selected_projects();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " Step 3: Confirmation ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

    let inner_area = block.inner(area);
    f.render_widget(block, area);

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
            Span::styled("🚀 ", Style::default()),
            Span::styled(
                "Ready to prepare release!",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(Span::styled(
            format!("   {} projects selected", selected_projects.len()),
            Style::default().fg(Color::DarkGray),
        )),
    ];
    let header = Paragraph::new(header_lines).alignment(ratatui::layout::Alignment::Center);
    f.render_widget(header, chunks[0]);

    let content_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .margin(1)
        .split(chunks[1]);

    let mut project_lines = vec![
        Line::from(vec![
            Span::styled("📦 ", Style::default()),
            Span::styled("Projects", Style::default().fg(Color::White)),
        ]),
        Line::from(""),
    ];

    for project in selected_projects.iter().take(10) {
        let bump_text = project.effective_bump_str();
        let bump_color = match bump_text {
            "MAJOR" => Color::Red,
            "MINOR" => Color::Yellow,
            "PATCH" => Color::Green,
            _ => Color::Cyan,
        };

        project_lines.push(Line::from(vec![
            Span::styled("   ✅ ", Style::default().fg(Color::Green)),
            Span::styled(project.name(), Style::default().fg(Color::White)),
            Span::styled(
                format!(" ({} commits)", project.commit_count()),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
        project_lines.push(Line::from(vec![
            Span::styled("      → ", Style::default().fg(Color::DarkGray)),
            Span::styled(bump_text, Style::default().fg(bump_color)),
        ]));
    }

    if selected_projects.len() > 10 {
        project_lines.push(Line::from(Span::styled(
            format!("   ... and {} more", selected_projects.len() - 10),
            Style::default().fg(Color::DarkGray),
        )));
    }

    let project_block = Paragraph::new(project_lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(Span::styled(" Summary ", Style::default().fg(Color::White))),
    );
    f.render_widget(project_block, content_chunks[0]);

    let mut file_lines = vec![
        Line::from(vec![
            Span::styled("📄 ", Style::default()),
            Span::styled("Files to Modify", Style::default().fg(Color::White)),
        ]),
        Line::from(""),
    ];

    let mut ecosystems: std::collections::HashSet<EcosystemType> = std::collections::HashSet::new();
    for project in &selected_projects {
        ecosystems.insert(project.project_type());
    }

    for ecosystem in &ecosystems {
        file_lines.push(Line::from(vec![
            Span::styled("   ✏️  ", Style::default().fg(Color::Yellow)),
            Span::styled(
                format!("{}", ecosystem.version_file()),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }
    file_lines.push(Line::from(vec![
        Span::styled("   📝 ", Style::default().fg(Color::Cyan)),
        Span::styled("CHANGELOG.md", Style::default().fg(Color::DarkGray)),
    ]));
    file_lines.push(Line::from(vec![
        Span::styled("   📋 ", Style::default().fg(Color::Magenta)),
        Span::styled(
            "belaf/releases/*.json",
            Style::default().fg(Color::DarkGray),
        ),
    ]));

    let file_block = Paragraph::new(file_lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(Span::styled(
                " Will Execute ",
                Style::default().fg(Color::White),
            )),
    );
    f.render_widget(file_block, content_chunks[1]);

    let hints = Line::from(vec![
        Span::styled("Enter", Style::default().fg(Color::Green)),
        Span::styled(" confirm  ", Style::default().fg(Color::DarkGray)),
        Span::styled("Esc", Style::default().fg(Color::Yellow)),
        Span::styled(" back  ", Style::default().fg(Color::DarkGray)),
        Span::styled("?", Style::default().fg(Color::Yellow)),
        Span::styled(" help  ", Style::default().fg(Color::DarkGray)),
        Span::styled("q", Style::default().fg(Color::Red)),
        Span::styled(" quit", Style::default().fg(Color::DarkGray)),
    ]);
    let hints_para = Paragraph::new(hints).alignment(ratatui::layout::Alignment::Center);
    f.render_widget(hints_para, chunks[2]);
}

fn render_help_popup(f: &mut Frame, state: &WizardState) {
    let area = centered_rect(60, 70, f.area());

    let help_text = match &state.step {
        WizardStep::ProjectSelection => {
            "Project Selection Help\n\n\
             • Use ↑/↓ arrows to navigate projects\n\
             • Press Space to toggle project selection\n\
             • Press 'a' to toggle all projects\n\
             • Press Enter to proceed to next step\n\
             • At least one project must be selected\n\n\
             The wizard analyzes your commits using\n\
             Conventional Commits to suggest version bumps.\n\n\
             Each selected project will be configured\n\
             individually in the next steps."
        }
        WizardStep::ProjectConfig { .. } => {
            if state.show_changelog {
                "Changelog Preview Help\n\n\
                 This shows what will be added to the\n\
                 CHANGELOG.md file for this project.\n\n\
                 The changelog is generated from your\n\
                 Git commit messages using Conventional\n\
                 Commits format.\n\n\
                 • Press Tab to go back to bump selection\n\
                 • Press Enter to move to the next project\n\
                 • Press Esc to go back"
            } else {
                "Bump Strategy Help\n\n\
                 • Auto: Use conventional commits analysis\n\
                 • Major: Breaking changes (x.0.0)\n\
                 • Minor: New features (0.x.0)\n\
                 • Patch: Bug fixes (0.0.x)\n\n\
                 Each project can have its own bump strategy.\n\
                 The 'Auto' option uses the suggested bump\n\
                 based on your commit messages.\n\n\
                 • Press ↑/↓ to select a bump strategy\n\
                 • Press Tab to preview the changelog\n\
                 • Press Enter to confirm and continue"
            }
        }
        WizardStep::Confirmation => {
            "Confirmation Help\n\n\
             Review the changes that will be made:\n\
             • Version numbers in project files\n\
             • CHANGELOG.md entries\n\
             • Dependency version updates\n\n\
             Each project shows its selected bump strategy.\n\n\
             Press Enter to apply all changes.\n\
             You will still need to commit and tag."
        }
    };

    let paragraph = Paragraph::new(help_text)
        .style(Style::default().fg(Color::White).bg(Color::Black))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Help (press any key to close) ")
                .style(Style::default().bg(Color::Black)),
        )
        .wrap(Wrap { trim: true });

    f.render_widget(paragraph, area);
}

fn parse_existing_changelog(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;

    let mut result = String::new();
    let mut found_header = false;

    for line in content.lines() {
        if line.starts_with("## [") || line.starts_with("## Unreleased") {
            found_header = true;
        }

        if found_header {
            result.push_str(line);
            result.push('\n');
        }
    }

    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

fn apply_project_overrides_to_items(
    projects: &mut [ProjectItem],
    overrides: &[String],
) -> Result<()> {
    let project_names: Vec<String> = projects.iter().map(|p| p.name().to_string()).collect();

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

        if let Some(project) = projects.iter_mut().find(|p| p.name() == project_name) {
            let chosen = match bump_type {
                "major" => BumpChoice::Major,
                "minor" => BumpChoice::Minor,
                "patch" => BumpChoice::Patch,
                _ => unreachable!(),
            };
            info!("override: {} -> {}", project_name, bump_type);
            project.chosen_bump = Some(chosen);
        }
    }

    Ok(())
}
