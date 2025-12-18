use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind},
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
use tracing::info;

use crate::{
    atry,
    core::{
        ecosystem::types::EcosystemType,
        release::{
            commit_analyzer::{self, BumpRecommendation},
            graph::GraphQueryBuilder,
            project::ProjectId,
            session::AppSession,
            workflow::{
                create_release_branch, generate_changelog_body, polish_changelog_with_ai,
                ReleasePipeline, SelectedProject,
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
    selected: bool,
    commit_count: usize,
    suggested_bump: BumpRecommendation,
    chosen_bump: Option<BumpStrategy>,
    commit_messages: Vec<String>,
    project_type: EcosystemType,
    cached_changelog: Option<String>,
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

    fn description(&self) -> &'static str {
        match self {
            Self::Auto => "Use conventional commits to determine bump type automatically",
            Self::Major => "Breaking changes (1.0.0 → 2.0.0)",
            Self::Minor => "New features (1.0.0 → 1.1.0)",
            Self::Patch => "Bug fixes (1.0.0 → 1.0.1)",
        }
    }

    fn all() -> Vec<Self> {
        vec![Self::Auto, Self::Major, Self::Minor, Self::Patch]
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
        }
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

    fn ensure_current_changelog_cached(&mut self) {
        let ai_enabled = self.ai_enabled;
        let changelog_data = {
            let project = match self.get_current_project() {
                Some(p) => p,
                None => return,
            };

            if project.cached_changelog.is_some() {
                return;
            }

            let commit_messages = project.commit_messages.clone();
            let draft = generate_changelog_body(&commit_messages);

            if ai_enabled {
                match polish_changelog_with_ai(&draft, &commit_messages) {
                    Ok(polished) => polished,
                    Err(e) => {
                        tracing::warn!("AI changelog polishing failed, using draft: {:#}", e);
                        draft
                    }
                }
            } else {
                draft
            }
        };

        if let Some(project) = self.get_current_project_mut() {
            project.cached_changelog = Some(changelog_data);
        }
    }

    fn next_step(&mut self) -> bool {
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
                    self.show_changelog = true;
                    self.ensure_current_changelog_cached();
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

        projects.push(ProjectItem {
            ident: *ident,
            name: proj.user_facing_name.clone(),
            prefix: proj.prefix().escaped(),
            selected: true,
            commit_count: n_commits,
            suggested_bump: analysis.recommendation,
            chosen_bump: None,
            commit_messages,
            project_type,
            cached_changelog: None,
        });
    }

    if projects.is_empty() {
        info!("no projects with changes found");
        return Ok(0);
    }

    let ai_enabled = sess.changelog_config.ai_enabled;
    let wizard_result = run_wizard_ui(projects, ai_enabled)?;

    let selected_projects = match wizard_result {
        Some(projects) => projects,
        None => {
            info!("release preparation cancelled by user");
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
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut state = WizardState::new(projects, ai_enabled);
    let result = run_app(&mut terminal, &mut state);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
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

        if let Event::Key(KeyEvent {
            code,
            kind: KeyEventKind::Press,
            ..
        }) = event::read()?
        {
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
    let help_text = match &state.step {
        WizardStep::ProjectSelection => {
            "↑/↓: Navigate | Space: Toggle | A: Toggle All | Enter: Next | Q: Quit | ?: Help"
        }
        WizardStep::ProjectConfig { .. } => {
            if state.show_changelog {
                "Tab: Back to Bump | Enter: Next Project | Esc: Back | Q: Quit | ?: Help"
            } else {
                "↑/↓: Navigate | Tab: Preview Changelog | Enter: Next | Esc: Back | Q: Quit | ?: Help"
            }
        }
        WizardStep::Confirmation => "Enter: Confirm | Esc: Back | Q: Quit | ?: Help",
    };

    let footer = Paragraph::new(help_text)
        .style(Style::default().fg(Color::Gray))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, area);
}

fn render_step(f: &mut Frame, area: Rect, state: &mut WizardState) {
    match &state.step {
        WizardStep::ProjectSelection => render_project_selection(f, area, state),
        WizardStep::ProjectConfig { .. } => {
            if state.show_changelog {
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

    let (project_name, suggested_bump, commit_count) = match state.get_current_project() {
        Some(p) => (p.name.clone(), p.suggested_bump, p.commit_count),
        None => return,
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    let items: Vec<ListItem> = strategies
        .iter()
        .map(|strategy| {
            let content = format!("{} - {}", strategy.as_str(), strategy.description());
            ListItem::new(content)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!("Choose version bump for {}", project_name)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("► ");

    f.render_stateful_widget(list, chunks[0], &mut state.bump_list_state);

    let suggestion_text = match suggested_bump {
        BumpRecommendation::Major => "MAJOR (breaking changes)",
        BumpRecommendation::Minor => "MINOR (new features)",
        BumpRecommendation::Patch => "PATCH (bug fixes)",
        BumpRecommendation::None => "NO BUMP (no changes)",
    };

    let suggestions = Paragraph::new(format!(
        "Suggested bump based on conventional commits:\n\n  {}\n\nProject has {} commit{}.\n\nPress Tab to preview the changelog.",
        suggestion_text,
        commit_count,
        if commit_count == 1 { "" } else { "s" }
    ))
    .style(Style::default().fg(Color::Yellow))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("Analysis"),
    )
    .wrap(Wrap { trim: true });

    f.render_widget(suggestions, chunks[1]);
}

fn render_project_changelog(f: &mut Frame, area: Rect, state: &WizardState) {
    let current_project = match state.get_current_project() {
        Some(p) => p,
        None => return,
    };

    let chosen_bump = current_project.chosen_bump.unwrap_or(BumpStrategy::Auto);
    let bump_text = match chosen_bump {
        BumpStrategy::Auto => current_project.suggested_bump.as_str(),
        BumpStrategy::Major => "MAJOR",
        BumpStrategy::Minor => "MINOR",
        BumpStrategy::Patch => "PATCH",
    };

    let mut changelog_content = format!(
        "# Changelog Preview for {}\n\n\
        **Selected bump:** `{}`\n\
        **Commits analyzed:** {}\n\n",
        current_project.name, bump_text, current_project.commit_count
    );

    let cached_body = current_project
        .cached_changelog
        .as_deref()
        .unwrap_or("Loading changelog...");

    changelog_content.push_str(cached_body);

    changelog_content.push_str(
        "\n\n---\n\n\
        Press Tab to go back to bump selection.\n\
        Press Enter to continue to the next project.",
    );

    let markdown_text = markdown::render_markdown(&changelog_content);

    let paragraph = Paragraph::new(markdown_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!("Changelog Preview: {}", current_project.name))
                .padding(Padding::horizontal(2)),
        )
        .wrap(Wrap { trim: false })
        .scroll((0, 0));

    f.render_widget(paragraph, area);
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
