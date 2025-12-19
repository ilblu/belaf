use anyhow::{Context, Result};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
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

use crate::{
    atry,
    core::{
        ecosystem::types::EcosystemType,
        release::{
            changelog_generator::parse_existing_changelog,
            commit_analyzer::{self, BumpRecommendation},
            graph::GraphQueryBuilder,
            project::ProjectId,
            session::AppSession,
            repository::RepoPathBuf,
            workflow::{
                cleanup_release_branch, create_release_branch, generate_changelog_entry,
                polish_changelog_with_ai, ReleasePipeline, SelectedProject,
            },
        },
        ui::markdown,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum WizardStep {
    ProjectSelection,
    ProjectConfig { project_index: usize },
    Confirmation,
}

impl WizardStep {
    fn step_number(&self, total_projects: usize) -> String {
        match self {
            Self::ProjectSelection => "Step 1".to_string(),
            Self::ProjectConfig { project_index } => {
                format!("Step {} of {}", project_index + 2, total_projects + 2)
            }
            Self::Confirmation => format!("Step {}", total_projects + 2),
        }
    }

    fn title(&self, project_name: Option<&str>) -> String {
        match self {
            Self::ProjectSelection => "Select Projects".to_string(),
            Self::ProjectConfig { .. } => {
                format!("Configure: {}", project_name.unwrap_or("Unknown"))
            }
            Self::Confirmation => "Confirm Changes".to_string(),
        }
    }
}

struct ProjectItem {
    ident: ProjectId,
    name: String,
    prefix: String,
    current_version: String,
    selected: bool,
    commit_count: usize,
    suggested_bump: BumpRecommendation,
    chosen_bump: Option<BumpStrategy>,
    commit_messages: Vec<String>,
    project_type: EcosystemType,
    cached_changelog: Option<String>,
    existing_changelog: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BumpStrategy {
    Auto,
    Major,
    Minor,
    Patch,
}

impl BumpStrategy {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Major => "major",
            Self::Minor => "minor",
            Self::Patch => "patch",
        }
    }

    fn all() -> Vec<Self> {
        vec![Self::Auto, Self::Major, Self::Minor, Self::Patch]
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
    ai_enabled: bool,
    loading_changelog: bool,
    loading_frame: usize,
    loading_receiver: Option<Receiver<String>>,
    show_raw_markdown: bool,
    toggle_button_area: Option<Rect>,
}

impl WizardState {
    fn new(projects: Vec<ProjectItem>, ai_enabled: bool) -> Self {
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
            ai_enabled,
            loading_changelog: false,
            loading_frame: 0,
            loading_receiver: None,
            show_raw_markdown: false,
            toggle_button_area: None,
        }
    }

    fn toggle_markdown_view(&mut self) {
        self.show_raw_markdown = !self.show_raw_markdown;
    }

    fn handle_mouse_click(&mut self, x: u16, y: u16) -> bool {
        if let Some(area) = self.toggle_button_area {
            if x >= area.x && x < area.x + area.width && y >= area.y && y < area.y + area.height {
                self.toggle_markdown_view();
                return true;
            }
        }
        false
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

        let commit_messages = project.commit_messages.clone();
        let ai_enabled = self.ai_enabled;
        let current_version = project.current_version.clone();
        let chosen_bump = project.chosen_bump.unwrap_or(BumpStrategy::Auto);
        let suggested_bump = project.suggested_bump;

        let new_version = match chosen_bump {
            BumpStrategy::Auto => calculate_next_version(&current_version, suggested_bump),
            BumpStrategy::Major => calculate_major_version(&current_version),
            BumpStrategy::Minor => calculate_minor_version(&current_version),
            BumpStrategy::Patch => calculate_patch_version(&current_version),
        };

        let (tx, rx) = mpsc::channel();
        self.loading_receiver = Some(rx);

        thread::spawn(move || {
            let draft = generate_changelog_entry(&new_version, &commit_messages);
            let result = if ai_enabled {
                match polish_changelog_with_ai(&draft, &commit_messages) {
                    Ok(polished) => polished,
                    Err(e) => {
                        tracing::warn!("AI changelog polishing failed, using draft: {:#}", e);
                        draft
                    }
                }
            } else {
                draft
            };
            let _ = tx.send(result);
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
                            project.chosen_bump = Some(BumpStrategy::all()[selected]);
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
                            let idx = BumpStrategy::all()
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
                    let strategies = BumpStrategy::all();
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

pub fn run() -> Result<i32> {
    info!("starting interactive TUI wizard for release preparation");

    let mut sess =
        AppSession::initialize_default().context("could not initialize app and project graph")?;

    if let Some(dirty) = sess
        .repo
        .check_if_dirty(&[])
        .context("failed to check repository for modified files")?
    {
        info!(
            "preparing release with uncommitted changes in the repository (e.g.: `{}`)",
            dirty.escaped()
        );
    }

    let (base_branch, release_branch) = create_release_branch(&mut sess)?;

    let q = GraphQueryBuilder::default();
    let idents = sess.graph().query(q).context("could not select projects")?;

    if idents.is_empty() {
        info!("no projects found in repository");
        cleanup_release_branch(&mut sess, &base_branch, &release_branch);
        return Ok(0);
    }

    let histories = sess
        .analyze_histories()
        .context("failed to analyze project histories")?;

    let mut projects = Vec::new();
    for ident in &idents {
        let proj = sess.graph().lookup(*ident);
        let history = histories.lookup(*ident);
        let n_commits = history.n_commits();

        if n_commits == 0 {
            continue;
        }

        let commit_messages: Vec<String> = history
            .commits()
            .into_iter()
            .filter_map(|cid| sess.repo.get_commit_summary(*cid).ok())
            .collect();

        let analysis = commit_analyzer::analyze_commit_messages(&commit_messages)
            .context("failed to analyze commit messages")?;

        let qnames = proj.qualified_names();
        let project_type = qnames
            .get(1)
            .and_then(|s| EcosystemType::from_qname(s))
            .unwrap_or(EcosystemType::Cargo);

        let current_version = history
            .release_version()
            .map(|v| v.to_string())
            .unwrap_or_else(|| proj.version.to_string());

        let prefix_str = proj.prefix().escaped();
        let changelog_rel_path = if prefix_str.is_empty() {
            "CHANGELOG.md".to_string()
        } else {
            format!("{}/CHANGELOG.md", prefix_str)
        };
        let changelog_repo_path = RepoPathBuf::new(changelog_rel_path.as_bytes());
        let changelog_path = sess.repo.resolve_workdir(changelog_repo_path.as_ref());
        let existing_changelog = parse_existing_changelog(&changelog_path).unwrap_or_default();

        projects.push(ProjectItem {
            ident: *ident,
            name: proj.user_facing_name.clone(),
            prefix: prefix_str,
            current_version,
            selected: true,
            commit_count: n_commits,
            suggested_bump: analysis.recommendation,
            chosen_bump: None,
            commit_messages,
            project_type,
            cached_changelog: None,
            existing_changelog,
        });
    }

    if projects.is_empty() {
        info!("no projects with changes found");
        cleanup_release_branch(&mut sess, &base_branch, &release_branch);
        return Ok(0);
    }

    let ai_enabled = sess.changelog_config.ai_enabled;
    let wizard_result = run_wizard_ui(projects, ai_enabled)?;

    let selected_projects = match wizard_result {
        Some(projects) => projects,
        None => {
            info!("release preparation cancelled by user");
            cleanup_release_branch(&mut sess, &base_branch, &release_branch);
            return Ok(1);
        }
    };

    info!(
        "applying version bumps to {} project(s)",
        selected_projects.len()
    );

    let mut prepared: Vec<SelectedProject> = Vec::new();

    for project_item in &selected_projects {
        let proj = sess.graph().lookup(project_item.ident);
        let history = histories.lookup(project_item.ident);

        let bump_strategy = project_item.chosen_bump.unwrap_or(BumpStrategy::Auto);

        let bump_scheme_text = match bump_strategy {
            BumpStrategy::Auto => project_item.suggested_bump.as_str(),
            BumpStrategy::Major => "major",
            BumpStrategy::Minor => "minor",
            BumpStrategy::Patch => "patch",
        };

        if bump_scheme_text == "no bump" {
            info!("{}: no version bump needed", proj.user_facing_name);
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

        let old_version = history
            .release_version()
            .map(|v| v.to_string())
            .unwrap_or_else(|| proj.version.to_string());

        let proj_mut = sess.graph_mut().lookup_mut(project_item.ident);

        atry!(
            bump_scheme.apply(&mut proj_mut.version);
            ["failed to apply version bump to {}", proj_mut.user_facing_name]
        );

        let new_version = proj_mut.version.to_string();

        info!(
            "{}: {} -> {} ({} commit{})",
            proj_mut.user_facing_name,
            old_version,
            new_version,
            project_item.commit_count,
            if project_item.commit_count == 1 {
                ""
            } else {
                "s"
            }
        );

        prepared.push(SelectedProject {
            name: proj_mut.user_facing_name.clone(),
            prefix: project_item.prefix.clone(),
            old_version,
            new_version,
            bump_type: bump_scheme_text.to_string(),
            commit_messages: project_item.commit_messages.clone(),
            ecosystem: project_item.project_type,
            cached_changelog: project_item.cached_changelog.clone(),
        });
    }

    if prepared.is_empty() {
        info!("no projects needed version bumps");
        return Ok(0);
    }

    let pipeline = ReleasePipeline::new(&mut sess, base_branch, release_branch)?;
    pipeline.execute(prepared)?;

    Ok(0)
}

fn run_wizard_ui(projects: Vec<ProjectItem>, ai_enabled: bool) -> Result<Option<Vec<ProjectItem>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut state = WizardState::new(projects, ai_enabled);
    let result = run_app(&mut terminal, &mut state);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
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
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(f.area());

    render_header(f, chunks[0], state);
    render_step(f, chunks[1], state);
    render_footer(f, chunks[2], state);

    if state.show_help {
        render_help_popup(f, state);
    }
}

fn render_header(f: &mut Frame, area: Rect, state: &WizardState) {
    let project_name = state.get_current_project().map(|p| p.name.as_str());
    let step_number = state.step.step_number(state.selected_count());
    let title = format!(
        "Release Preparation Wizard - {} - {}",
        step_number,
        state.step.title(project_name)
    );
    let header = Paragraph::new(title)
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, area);
}

fn render_footer(f: &mut Frame, area: Rect, state: &WizardState) {
    let help_text = if state.is_loading() {
        "⏳ Generating changelog with AI... | Esc: Cancel"
    } else {
        match &state.step {
            WizardStep::ProjectSelection => {
                "↑/↓: Navigate | Space: Toggle | A: Toggle All | Enter: Next | Q: Quit | ?: Help"
            }
            WizardStep::ProjectConfig { .. } => {
                if state.show_changelog {
                    "M: Toggle View | Tab: Back to Bump | Enter: Next | Esc: Back | Q: Quit"
                } else {
                    "↑/↓: Navigate | Tab: Preview Changelog | Enter: Next | Esc: Back | Q: Quit | ?: Help"
                }
            }
            WizardStep::Confirmation => "Enter: Confirm | Esc: Back | Q: Quit | ?: Help",
        }
    };

    let footer = Paragraph::new(help_text)
        .style(Style::default().fg(Color::Gray))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, area);
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
    let items: Vec<ListItem> = state
        .projects
        .iter()
        .map(|project| {
            let checkbox = if project.selected { "[✓]" } else { "[ ]" };
            let suggestion = match project.suggested_bump {
                BumpRecommendation::Major => " (suggests: MAJOR)",
                BumpRecommendation::Minor => " (suggests: MINOR)",
                BumpRecommendation::Patch => " (suggests: PATCH)",
                BumpRecommendation::None => "",
            };

            let content = format!(
                "{} {} ({} commits){}",
                checkbox, project.name, project.commit_count, suggestion
            );

            ListItem::new(content).style(if project.selected {
                Style::default().fg(Color::Green)
            } else {
                Style::default()
            })
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Select projects to prepare for release"),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("► ");

    f.render_stateful_widget(list, area, &mut state.project_list_state);
}

fn render_project_bump_strategy(f: &mut Frame, area: Rect, state: &mut WizardState) {
    let strategies = BumpStrategy::all();

    let project = match state.get_current_project() {
        Some(p) => p,
        None => return,
    };

    let project_name = project.name.clone();
    let current_version = project.current_version.clone();
    let suggested_bump = project.suggested_bump;
    let commit_messages = project.commit_messages.clone();

    let selected_index = state.bump_list_state.selected().unwrap_or(0);
    let selected_strategy = strategies.get(selected_index).copied().unwrap_or(BumpStrategy::Auto);

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(area);

    let items: Vec<ListItem> = strategies
        .iter()
        .map(|strategy| {
            let next_ver = match strategy {
                BumpStrategy::Auto => calculate_next_version(&current_version, suggested_bump),
                BumpStrategy::Major => calculate_major_version(&current_version),
                BumpStrategy::Minor => calculate_minor_version(&current_version),
                BumpStrategy::Patch => calculate_patch_version(&current_version),
            };
            let content = format!("  {}  →  {}", strategy.as_str(), next_ver);
            ListItem::new(content)
        })
        .collect();

    let left_title = if state.is_loading() {
        format!("{} ({}) [Processing...]", project_name, current_version)
    } else {
        format!("{} ({})", project_name, current_version)
    };

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(left_title),
        )
        .highlight_style(
            Style::default()
                .bg(if state.is_loading() { Color::DarkGray } else { Color::DarkGray })
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(if state.is_loading() { "⏳" } else { "► " });

    f.render_stateful_widget(list, chunks[0], &mut state.bump_list_state);

    if state.is_loading() {
        let loading_content = build_loading_panel(state, &commit_messages);
        let loading_panel = Paragraph::new(loading_content)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("🤖 Generating Changelog..."),
            )
            .wrap(Wrap { trim: true });
        f.render_widget(loading_panel, chunks[1]);
    } else {
        let detail_content = build_detail_panel(
            &selected_strategy,
            &current_version,
            suggested_bump,
            &commit_messages,
        );

        let detail_panel = Paragraph::new(detail_content)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!("Details: {}", selected_strategy.as_str())),
            )
            .wrap(Wrap { trim: true });

        f.render_widget(detail_panel, chunks[1]);
    }
}

fn build_loading_panel(state: &WizardState, commit_messages: &[String]) -> Text<'static> {
    let spinner = state.loading_spinner();
    let commit_count = commit_messages.len();

    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            format!("{} ", spinner),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Analyzing commits with Claude AI",
            Style::default().fg(Color::Cyan),
        ),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("   Processing {} commit{}...", commit_count, if commit_count == 1 { "" } else { "s" }),
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
    strategy: &BumpStrategy,
    current_version: &str,
    suggested_bump: BumpRecommendation,
    commit_messages: &[String],
) -> Text<'static> {
    let mut lines: Vec<Line> = Vec::new();

    let next_version = match strategy {
        BumpStrategy::Auto => calculate_next_version(current_version, suggested_bump),
        BumpStrategy::Major => calculate_major_version(current_version),
        BumpStrategy::Minor => calculate_minor_version(current_version),
        BumpStrategy::Patch => calculate_patch_version(current_version),
    };

    lines.push(Line::from(vec![
        Span::styled("Version: ", Style::default().fg(Color::DarkGray)),
        Span::styled(current_version.to_string(), Style::default().fg(Color::Yellow)),
        Span::styled(" → ", Style::default().fg(Color::DarkGray)),
        Span::styled(next_version, Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
    ]));
    lines.push(Line::from(""));

    match strategy {
        BumpStrategy::Auto => {
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
        BumpStrategy::Major => {
            lines.push(Line::from(Span::styled(
                "⚠ Breaking Change Release",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(Span::styled(
                "  Consider updating migration guides",
                Style::default().fg(Color::DarkGray),
            )));
        }
        BumpStrategy::Minor => {
            lines.push(Line::from(Span::styled(
                "✓ Feature Release",
                Style::default().fg(Color::Green),
            )));
            lines.push(Line::from(Span::styled(
                "  Backwards compatible additions",
                Style::default().fg(Color::DarkGray),
            )));
        }
        BumpStrategy::Patch => {
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

    let (feat_count, fix_count, breaking_count, other_count) = count_commit_types(commit_messages);

    lines.push(Line::from(Span::styled(
        "Commit Analysis:",
        Style::default().add_modifier(Modifier::BOLD),
    )));

    if breaking_count > 0 {
        lines.push(Line::from(vec![
            Span::styled("  breaking:  ", Style::default().fg(Color::Red)),
            Span::styled(format!("{}", breaking_count), Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
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
            Span::styled(format!("{}", other_count), Style::default().fg(Color::DarkGray)),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Commits:",
        Style::default().add_modifier(Modifier::BOLD),
    )));

    for (i, msg) in commit_messages.iter().take(8).enumerate() {
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

    if commit_messages.len() > 8 {
        lines.push(Line::from(Span::styled(
            format!("  ... and {} more", commit_messages.len() - 8),
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

fn count_commit_types(messages: &[String]) -> (usize, usize, usize, usize) {
    let mut feat = 0;
    let mut fix = 0;
    let mut breaking = 0;
    let mut other = 0;

    for msg in messages {
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

    let chosen_bump = current_project.chosen_bump.unwrap_or(BumpStrategy::Auto);
    let new_version = match chosen_bump {
        BumpStrategy::Auto => calculate_next_version(&current_project.current_version, current_project.suggested_bump),
        BumpStrategy::Major => calculate_major_version(&current_project.current_version),
        BumpStrategy::Minor => calculate_minor_version(&current_project.current_version),
        BumpStrategy::Patch => calculate_patch_version(&current_project.current_version),
    };

    let new_entry = current_project
        .cached_changelog
        .as_deref()
        .unwrap_or("Loading changelog...");

    let mut changelog_content = format!(
        "# Changelog\n\n\
        All notable changes to {} will be documented in this file.\n\n\
        The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),\n\
        and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).\n\n",
        current_project.name
    );

    changelog_content.push_str(new_entry);

    if !current_project.existing_changelog.is_empty() {
        changelog_content.push_str("\n\n");
        changelog_content.push_str(&current_project.existing_changelog);
    }

    let show_raw = state.show_raw_markdown;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let toggle_area = chunks[0];
    let content_area = chunks[1];

    let rendered_style = if !show_raw {
        Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };

    let source_style = if show_raw {
        Style::default().fg(Color::Black).bg(Color::Magenta).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };

    let toggle_line = Line::from(vec![
        Span::raw("  "),
        Span::styled(" ◉ Preview ", rendered_style),
        Span::raw("  "),
        Span::styled(" ◉ Source ", source_style),
        Span::raw("  "),
        Span::styled("(m)", Style::default().fg(Color::DarkGray)),
    ]);

    let title = format!(
        "{} ({} → {})",
        current_project.name, current_project.current_version, new_version
    );

    let toggle_widget = Paragraph::new(toggle_line)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title),
        )
        .alignment(ratatui::layout::Alignment::Center);

    f.render_widget(toggle_widget, toggle_area);

    state.toggle_button_area = Some(toggle_area);

    if show_raw {
        let raw_text = Text::from(changelog_content.clone());
        let paragraph = Paragraph::new(raw_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Markdown Source")
                    .padding(Padding::horizontal(1)),
            )
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(Color::Rgb(180, 180, 180)));
        f.render_widget(paragraph, content_area);
    } else {
        let markdown_text = markdown::render_markdown(&changelog_content);
        let paragraph = Paragraph::new(markdown_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Rendered Preview")
                    .padding(Padding::horizontal(1)),
            )
            .wrap(Wrap { trim: false });
        f.render_widget(paragraph, content_area);
    }
}

fn render_confirmation(f: &mut Frame, area: Rect, state: &WizardState) {
    let selected_projects = state.selected_projects();

    let mut confirmation_lines = vec![
        Line::from(Span::styled(
            "Ready to prepare release!",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!("Selected projects: {}", selected_projects.len()),
            Style::default().fg(Color::Cyan),
        )),
        Line::from(""),
    ];

    for project in &selected_projects {
        let chosen_bump = project.chosen_bump.unwrap_or(BumpStrategy::Auto);
        let bump_text = match chosen_bump {
            BumpStrategy::Auto => project.suggested_bump.as_str(),
            BumpStrategy::Major => "MAJOR",
            BumpStrategy::Minor => "MINOR",
            BumpStrategy::Patch => "PATCH",
        };

        confirmation_lines.push(Line::from(vec![
            Span::styled("  • ", Style::default().fg(Color::Gray)),
            Span::styled(&project.name, Style::default().fg(Color::White)),
            Span::styled(
                format!(" ({} commits) → ", project.commit_count),
                Style::default().fg(Color::Gray),
            ),
            Span::styled(
                bump_text,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    confirmation_lines.push(Line::from(""));
    confirmation_lines.push(Line::from(""));
    confirmation_lines.push(Line::from(Span::styled(
        "Files that will be modified:",
        Style::default().fg(Color::Magenta),
    )));

    let mut ecosystems: std::collections::HashSet<EcosystemType> = std::collections::HashSet::new();
    for project in &selected_projects {
        ecosystems.insert(project.project_type);
    }

    for ecosystem in &ecosystems {
        confirmation_lines.push(Line::from(format!(
            "  • {} (version bump)",
            ecosystem.version_file()
        )));
    }
    confirmation_lines.push(Line::from("  • CHANGELOG.md (new entries)"));
    confirmation_lines.push(Line::from("  • belaf/releases/*.json (release manifest)"));

    confirmation_lines.push(Line::from(""));
    confirmation_lines.push(Line::from(""));
    confirmation_lines.push(Line::from(Span::styled(
        "Press Enter to confirm and apply changes",
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    )));
    confirmation_lines.push(Line::from(Span::styled(
        "Press Esc to go back",
        Style::default().fg(Color::Gray),
    )));

    let text = Text::from(confirmation_lines);
    let paragraph = Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL).title("Confirmation"))
        .wrap(Wrap { trim: true });

    f.render_widget(paragraph, area);
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
