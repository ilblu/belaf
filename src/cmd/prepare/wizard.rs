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
    git::repository::RepoPathBuf,
    session::AppSession,
    ui::{
        components::toggle_panel::TogglePanel,
        markdown,
        release_unit_view::{build_unit_row_line, BumpHint, RenderMode, UnitRow},
        utils::centered_rect,
    },
    wire::known::Ecosystem,
    workflow::{
        generate_changelog_entry, BumpChoice, PrepareContext, ReleaseUnitCandidate,
        ReleaseUnitSelection,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum WizardStep {
    ReleaseUnitSelection,
    UnitConfig { unit_index: usize },
    Confirmation,
}

struct ReleaseUnitItem {
    candidate: ReleaseUnitCandidate,
    selected: bool,
    chosen_bump: Option<BumpChoice>,
    cached_changelog: Option<String>,
    existing_changelog: String,
    /// Resolved group id, if this project is a member of a `[[group]]`.
    /// Group members render as a collapsed tree under one group-header
    /// row instead of as separate rows — see `WizardState::display_rows`
    /// (plan §5).
    group_id: Option<String>,
}

/// One renderable row in the Step 1 project list. Solo (ungrouped)
/// projects are one row each; grouped projects collapse into a single
/// `Group` row whose `member_indices` point into `WizardState::projects`.
/// The cursor moves through `Vec<DisplayRow>`, not the underlying
/// projects, so a 5-member group still navigates as one stop.
#[derive(Debug, Clone)]
enum DisplayRow {
    Solo {
        unit_idx: usize,
    },
    Group {
        id: String,
        member_indices: Vec<usize>,
    },
}

impl ReleaseUnitItem {
    fn from_candidate(candidate: ReleaseUnitCandidate, existing_changelog: String) -> Self {
        Self {
            candidate,
            selected: true,
            chosen_bump: None,
            cached_changelog: None,
            existing_changelog,
            group_id: None,
        }
    }

    fn group_id(&self) -> Option<&str> {
        self.group_id.as_deref()
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

    fn ecosystem(&self) -> &Ecosystem {
        &self.candidate.ecosystem
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
    units: Vec<ReleaseUnitItem>,
    unit_list_state: ListState,
    bump_list_state: ListState,
    show_changelog: bool,
    show_help: bool,
    loading_changelog: bool,
    loading_frame: usize,
    loading_receiver: Option<Receiver<String>>,
    changelog_toggle: TogglePanel,
    changelog_config: ChangelogConfiguration,
    bump_config: BumpConfiguration,
    changelog_scroll_offset: u16,
}

impl WizardState {
    fn new(
        projects: Vec<ReleaseUnitItem>,
        changelog_config: ChangelogConfiguration,
        bump_config: BumpConfiguration,
    ) -> Self {
        let mut unit_list_state = ListState::default();
        if !projects.is_empty() {
            unit_list_state.select(Some(0));
        }

        let mut bump_list_state = ListState::default();
        bump_list_state.select(Some(0));

        Self {
            step: WizardStep::ReleaseUnitSelection,
            units: projects,
            unit_list_state,
            bump_list_state,
            show_changelog: false,
            show_help: false,
            loading_changelog: false,
            loading_frame: 0,
            loading_receiver: None,
            changelog_toggle: TogglePanel::default(),
            changelog_config,
            bump_config,
            changelog_scroll_offset: 0,
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

    fn get_current_project(&self) -> Option<&ReleaseUnitItem> {
        if let WizardStep::UnitConfig { unit_index } = self.step {
            self.units.iter().filter(|p| p.selected).nth(unit_index)
        } else {
            None
        }
    }

    fn get_current_project_mut(&mut self) -> Option<&mut ReleaseUnitItem> {
        if let WizardStep::UnitConfig { unit_index } = self.step {
            self.units.iter_mut().filter(|p| p.selected).nth(unit_index)
        } else {
            None
        }
    }

    fn selected_count(&self) -> usize {
        self.units.iter().filter(|p| p.selected).count()
    }

    /// Compute the per-render display rows. Delegates to the free
    /// `compute_display_rows` so tests can exercise the layout
    /// algorithm without standing up a full WizardState.
    fn display_rows(&self) -> Vec<DisplayRow> {
        compute_display_rows(&self.units)
    }

    /// ResolvedReleaseUnit indices the row at `display_idx` represents — one for
    /// solo, all members for a group.
    fn projects_for_display_row(&self, display_idx: usize) -> Vec<usize> {
        let rows = self.display_rows();
        match rows.get(display_idx) {
            Some(DisplayRow::Solo { unit_idx }) => vec![*unit_idx],
            Some(DisplayRow::Group { member_indices, .. }) => member_indices.clone(),
            None => Vec::new(),
        }
    }

    fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
    }

    fn next_step(&mut self) -> bool {
        if self.loading_changelog {
            return false;
        }

        match &self.step {
            WizardStep::ReleaseUnitSelection => {
                if self.selected_count() == 0 {
                    return false;
                }
                self.step = WizardStep::UnitConfig { unit_index: 0 };
                self.show_changelog = false;
                self.bump_list_state.select(Some(0));
                true
            }
            WizardStep::UnitConfig { unit_index } => {
                if !self.show_changelog {
                    if let Some(selected) = self.bump_list_state.selected() {
                        let choice = BumpChoice::all()[selected];
                        // Group atomicity: changing one member's bump
                        // propagates to every sibling so the user can't
                        // accidentally desync the group through the UI.
                        // Validation at finalize is the safety net; this is
                        // the friendlier path where it just stays consistent.
                        if let Some(project) = self.get_current_project() {
                            let group_id = project.group_id().map(str::to_string);
                            if let Some(gid) = group_id {
                                for p in &mut self.units {
                                    if p.group_id() == Some(gid.as_str()) {
                                        p.chosen_bump = Some(choice);
                                    }
                                }
                            } else if let Some(p) = self.get_current_project_mut() {
                                p.chosen_bump = Some(choice);
                            }
                        }
                    }
                    self.loading_changelog = true;
                    self.start_background_changelog_generation();
                    true
                } else {
                    if *unit_index + 1 < self.selected_count() {
                        self.step = WizardStep::UnitConfig {
                            unit_index: unit_index + 1,
                        };
                        self.show_changelog = false;
                        self.bump_list_state.select(Some(0));
                        self.changelog_scroll_offset = 0;
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
            WizardStep::ReleaseUnitSelection => false,
            WizardStep::UnitConfig { unit_index } => {
                if self.show_changelog {
                    self.show_changelog = false;
                    true
                } else if *unit_index == 0 {
                    self.step = WizardStep::ReleaseUnitSelection;
                    true
                } else {
                    self.step = WizardStep::UnitConfig {
                        unit_index: unit_index - 1,
                    };
                    self.show_changelog = true;
                    true
                }
            }
            WizardStep::Confirmation => {
                let last_idx = self.selected_count().saturating_sub(1);
                self.step = WizardStep::UnitConfig {
                    unit_index: last_idx,
                };
                self.show_changelog = true;
                true
            }
        }
    }

    fn selected_projects(&self) -> Vec<&ReleaseUnitItem> {
        self.units.iter().filter(|p| p.selected).collect()
    }

    fn handle_key_unit_selection(&mut self, key: KeyCode) -> bool {
        // Cursor is in **display-row** space, not project space (plan §5).
        // A 5-member group is one row, so Up/Down skips past members.
        let row_count = self.display_rows().len();
        match key {
            KeyCode::Up => {
                if let Some(selected) = self.unit_list_state.selected() {
                    if selected > 0 {
                        self.unit_list_state.select(Some(selected - 1));
                    }
                }
            }
            KeyCode::Down => {
                if let Some(selected) = self.unit_list_state.selected() {
                    if selected + 1 < row_count {
                        self.unit_list_state.select(Some(selected + 1));
                    }
                }
            }
            KeyCode::Char(' ') => {
                if let Some(selected) = self.unit_list_state.selected() {
                    // Toggle all projects backed by this display row —
                    // group rows flip every member at once.
                    let indices = self.projects_for_display_row(selected);
                    let all_currently_on = indices.iter().all(|&i| self.units[i].selected);
                    for i in indices {
                        self.units[i].selected = !all_currently_on;
                    }
                }
            }
            KeyCode::Char('a') => {
                let all_selected = self.units.iter().all(|p| p.selected);
                for project in &mut self.units {
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

    fn handle_key_unit_config(&mut self, key: KeyCode) -> bool {
        match key {
            KeyCode::Tab => {
                self.show_changelog = !self.show_changelog;
                if self.show_changelog {
                    self.changelog_scroll_offset = 0;
                    if let Some(project) = self.get_current_project() {
                        if project.cached_changelog.is_none() {
                            self.loading_changelog = true;
                            self.start_background_changelog_generation();
                        }
                    }
                } else if let Some(project) = self.get_current_project() {
                    if let Some(chosen) = project.chosen_bump {
                        let idx = BumpChoice::all()
                            .iter()
                            .position(|s| *s == chosen)
                            .unwrap_or(0);
                        self.bump_list_state.select(Some(idx));
                    }
                }
                false
            }
            KeyCode::Up if self.show_changelog => {
                self.changelog_scroll_offset = self.changelog_scroll_offset.saturating_sub(1);
                false
            }
            KeyCode::Down if self.show_changelog => {
                self.changelog_scroll_offset = self.changelog_scroll_offset.saturating_add(1);
                false
            }
            KeyCode::PageUp if self.show_changelog => {
                self.changelog_scroll_offset = self.changelog_scroll_offset.saturating_sub(10);
                false
            }
            KeyCode::PageDown if self.show_changelog => {
                self.changelog_scroll_offset = self.changelog_scroll_offset.saturating_add(10);
                false
            }
            KeyCode::Up => {
                if let Some(selected) = self.bump_list_state.selected() {
                    if selected > 0 {
                        self.bump_list_state.select(Some(selected - 1));
                    }
                }
                false
            }
            KeyCode::Down => {
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

pub fn run_with_overrides_and_decisions(
    project_overrides: Option<Vec<String>>,
    decisions: Vec<crate::core::bump_source::BumpDecision>,
) -> Result<i32> {
    info!("starting interactive TUI wizard for release preparation");

    let mut sess =
        AppSession::initialize_default().context("could not initialize app and project graph")?;
    // Snapshot groups before ctx takes a mutable borrow on sess.
    let groups = sess.graph().groups().clone();

    let mut ctx = PrepareContext::initialize(&mut sess, true)?;
    ctx.discover_projects()?;

    if !ctx.has_candidates() {
        ctx.cleanup();
        print_no_changes_message();
        return Ok(0);
    }

    let projects: Vec<ReleaseUnitItem> = ctx
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
            let mut item = ReleaseUnitItem::from_candidate(candidate.clone(), existing_changelog);
            item.group_id = groups
                .group_of(candidate.ident)
                .map(|g| g.id.as_str().to_string());
            item
        })
        .collect();

    let mut projects = projects;
    // Precedence: external decisions feed in first; explicit
    // `--project name:bump` CLI overrides win on top. The wizard then
    // shows the resulting `chosen_bump` so the user can still change it
    // interactively before confirming.
    apply_decisions_to_items(&mut projects, &decisions)?;
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

    let selections: Vec<ReleaseUnitSelection> = selected_items
        .into_iter()
        .map(|item| ReleaseUnitSelection {
            candidate: item.candidate,
            bump_choice: item.chosen_bump.unwrap_or(BumpChoice::Auto),
            cached_changelog: item.cached_changelog,
        })
        .collect();

    // Group atomicity: same check as CI mode. The wizard already auto-syncs
    // bumps when the user edits one group member (see `WizardState::set_bump_choice`),
    // so this should only fire when the user *also* passed --project flags
    // that point in different directions.
    if let Err(e) = super::validate_group_consistency(&selections, &groups) {
        ctx.cleanup();
        return Err(e);
    }

    println!();
    let mut spinner = spinoff::Spinner::new(
        spinoff::spinners::Dots,
        "Creating release...",
        spinoff::Color::Yellow,
    );

    let pr_url = match ctx.finalize(selections) {
        Ok(url) => {
            spinner.success("Release preparation complete!");
            url
        }
        Err(e) => {
            spinner.fail("Release preparation failed!");
            return Err(e);
        }
    };

    println!();
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
    projects: Vec<ReleaseUnitItem>,
    changelog_config: ChangelogConfiguration,
    bump_config: BumpConfiguration,
) -> Result<Option<Vec<ReleaseUnitItem>>> {
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
        let selected = state.units.into_iter().filter(|p| p.selected).collect();
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
                    WizardStep::ReleaseUnitSelection => state.handle_key_unit_selection(code),
                    WizardStep::UnitConfig { .. } => state.handle_key_unit_config(code),
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
        WizardStep::ReleaseUnitSelection => render_project_selection(f, area, state),
        WizardStep::UnitConfig { .. } => {
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
    let selected_count = state.units.iter().filter(|p| p.selected).count();
    let total_count = state.units.len();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " Step 1: ReleaseUnit Selection ",
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
    f.render_widget(header, chunks[0]);

    // Plan §5: render groups as a single header row with members shown
    // as a read-only tree below. Cursor lands on the header row only.
    let display_rows = state.display_rows();
    // Compute the label width across solo rows so the secondary column
    // lines up — same shape the init wizard uses via the shared view.
    let label_width = state
        .units
        .iter()
        .map(|u| u.name().chars().count())
        .max()
        .unwrap_or(0);
    let items: Vec<ListItem> = display_rows
        .iter()
        .enumerate()
        .map(|(row_idx, row)| {
            let is_current = state.unit_list_state.selected() == Some(row_idx);

            match row {
                DisplayRow::Solo { unit_idx } => {
                    let project = &state.units[*unit_idx];
                    render_solo_row(project, is_current, label_width)
                }
                DisplayRow::Group { id, member_indices } => {
                    render_group_row(id, member_indices, &state.units, is_current)
                }
            }
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Gray))
                .title(Span::styled(
                    " Projects ",
                    Style::default().fg(Color::White),
                )),
        )
        .highlight_symbol("");

    f.render_stateful_widget(list, chunks[1], &mut state.unit_list_state);

    let hints = Line::from(vec![
        Span::styled("↑↓", Style::default().fg(Color::Cyan)),
        Span::styled(" navigate  ", Style::default().fg(Color::Gray)),
        Span::styled("Space", Style::default().fg(Color::Cyan)),
        Span::styled(" toggle  ", Style::default().fg(Color::Gray)),
        Span::styled("a", Style::default().fg(Color::Green)),
        Span::styled(" all  ", Style::default().fg(Color::Gray)),
        Span::styled("Enter", Style::default().fg(Color::Green)),
        Span::styled(" continue  ", Style::default().fg(Color::Gray)),
        Span::styled("?", Style::default().fg(Color::Yellow)),
        Span::styled(" help  ", Style::default().fg(Color::Gray)),
        Span::styled("q", Style::default().fg(Color::Red)),
        Span::styled(" quit", Style::default().fg(Color::Gray)),
    ]);
    let hints_para = Paragraph::new(hints).alignment(ratatui::layout::Alignment::Center);
    f.render_widget(hints_para, chunks[2]);
}

/// Solo projects are one row each; grouped projects collapse into one
/// row per group with member indices in original-order. Plan §5.
fn compute_display_rows(units: &[ReleaseUnitItem]) -> Vec<DisplayRow> {
    let mut out: Vec<DisplayRow> = Vec::new();
    let mut group_pos: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (i, p) in units.iter().enumerate() {
        match p.group_id() {
            Some(gid) => {
                if let Some(&pos) = group_pos.get(gid) {
                    if let DisplayRow::Group { member_indices, .. } = &mut out[pos] {
                        member_indices.push(i);
                    }
                } else {
                    group_pos.insert(gid.to_string(), out.len());
                    out.push(DisplayRow::Group {
                        id: gid.to_string(),
                        member_indices: vec![i],
                    });
                }
            }
            None => out.push(DisplayRow::Solo { unit_idx: i }),
        }
    }
    out
}

/// Render one ungrouped project as a single list row by delegating
/// to the shared [`build_unit_row_line`] helper. The caller maps each
/// `ReleaseUnitItem` into a [`UnitRow`] (with bump_hint + commit_count
/// populated from the candidate analysis), and the shared component
/// owns the visual layout — same component init/dashboard consume.
fn render_solo_row(
    project: &ReleaseUnitItem,
    is_current: bool,
    label_width: usize,
) -> ListItem<'static> {
    let row = solo_to_unit_row(project);
    let (line, bg) = build_unit_row_line(&row, is_current, RenderMode::Prepare, label_width);
    ListItem::new(vec![line]).style(bg)
}

fn solo_to_unit_row(project: &ReleaseUnitItem) -> UnitRow {
    let bump_hint = match project.suggested_bump() {
        BumpRecommendation::Major => Some(BumpHint::Major),
        BumpRecommendation::Minor => Some(BumpHint::Minor),
        BumpRecommendation::Patch => Some(BumpHint::Patch),
        BumpRecommendation::None => Some(BumpHint::None),
    };
    UnitRow {
        name: project.name().to_string(),
        version: project.current_version().to_string(),
        prefix: String::new(),
        ecosystem: Some(project.ecosystem().as_str().to_string()),
        annotations: Vec::new(),
        selected: project.selected,
        backref: 0, // prepare doesn't route through view-toggles; uses ListState directly
        bump_hint,
        commit_count: Some(project.commit_count()),
    }
}

/// Render one group as a header row + a tree of read-only member rows
/// below. Plan §5 reference layout:
///
/// ```text
/// [✅] schema (group, 2 members)            minor
///      ├─ @org/schema           (npm)
///      └─ com.org:schema        (maven)
/// ```
///
/// Member ecosystems are pulled off the underlying `ReleaseUnitItem` via
/// `project_type().as_str()` so the dashboard's display name matches
/// the wire format.
fn render_group_row(
    group_id: &str,
    member_indices: &[usize],
    projects: &[ReleaseUnitItem],
    is_current: bool,
) -> ListItem<'static> {
    let members: Vec<&ReleaseUnitItem> = member_indices.iter().map(|i| &projects[*i]).collect();
    if members.is_empty() {
        return ListItem::new(Line::from(""));
    }

    // Group selection-state: all/some/none of the members selected.
    let all_selected = members.iter().all(|m| m.selected);
    let any_selected = members.iter().any(|m| m.selected);
    let checkbox = if all_selected {
        "✅"
    } else if any_selected {
        "🟨" // partial
    } else {
        "⬜"
    };

    // Members of one group share a bump (validated CLI-side); pick the
    // first one's suggestion as the row's overall hint.
    let (suggestion_text, suggestion_color) = match members[0].suggested_bump() {
        BumpRecommendation::Major => ("MAJOR", Color::Red),
        BumpRecommendation::Minor => ("MINOR", Color::Yellow),
        BumpRecommendation::Patch => ("PATCH", Color::Green),
        BumpRecommendation::None => ("", Color::Gray),
    };

    let header_label_color = if is_current {
        Color::Cyan
    } else if all_selected {
        Color::Green
    } else if any_selected {
        Color::Yellow
    } else {
        Color::White
    };

    let header = Line::from(vec![
        Span::styled(format!(" {} ", checkbox), Style::default()),
        Span::styled(
            group_id.to_string(),
            Style::default()
                .fg(header_label_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" (group, {} members)", members.len()),
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::ITALIC),
        ),
        Span::styled(
            if !suggestion_text.is_empty() {
                format!("  → {}", suggestion_text)
            } else {
                String::new()
            },
            Style::default().fg(suggestion_color),
        ),
    ]);

    let mut lines = vec![header];
    for (i, m) in members.iter().enumerate() {
        let is_last = i == members.len() - 1;
        let connector = if is_last {
            "    └─ "
        } else {
            "    ├─ "
        };
        let eco_label = m.ecosystem().display_name();
        lines.push(Line::from(vec![
            Span::styled(connector, Style::default().fg(Color::DarkGray)),
            Span::styled(m.name().to_string(), Style::default().fg(Color::Gray)),
            Span::styled(
                format!("  ({})", eco_label),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    let style = if is_current {
        Style::default().bg(Color::Rgb(40, 40, 50))
    } else {
        Style::default()
    };
    ListItem::new(lines).style(style)
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
        WizardStep::UnitConfig { unit_index } => *unit_index + 1,
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
                Style::default().fg(Color::Gray),
            ),
            Span::styled(
                format!("  ({} commits)", commits.len()),
                Style::default().fg(Color::Gray),
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
                    Style::default().fg(Color::Gray),
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
                .border_style(Style::default().fg(Color::Gray))
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
                    .border_style(Style::default().fg(Color::Gray))
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
            Span::styled(" cancel", Style::default().fg(Color::Gray)),
        ])
    } else {
        Line::from(vec![
            Span::styled("↑↓", Style::default().fg(Color::Cyan)),
            Span::styled(" select  ", Style::default().fg(Color::Gray)),
            Span::styled("Tab", Style::default().fg(Color::Cyan)),
            Span::styled(" preview changelog  ", Style::default().fg(Color::Gray)),
            Span::styled("Enter", Style::default().fg(Color::Green)),
            Span::styled(" next  ", Style::default().fg(Color::Gray)),
            Span::styled("Esc", Style::default().fg(Color::Yellow)),
            Span::styled(" back  ", Style::default().fg(Color::Gray)),
            Span::styled("q", Style::default().fg(Color::Red)),
            Span::styled(" quit", Style::default().fg(Color::Gray)),
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
        Style::default().fg(Color::Gray),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "   This may take a few seconds.",
        Style::default().fg(Color::Gray),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "─".repeat(40),
        Style::default().fg(Color::Gray),
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
        Span::styled("Version: ", Style::default().fg(Color::Gray)),
        Span::styled(
            current_version.to_string(),
            Style::default().fg(Color::Yellow),
        ),
        Span::styled(" → ", Style::default().fg(Color::Gray)),
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
                Span::styled("Detected: ", Style::default().fg(Color::Gray)),
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
                Style::default().fg(Color::Gray),
            )));
        }
        BumpChoice::Minor => {
            lines.push(Line::from(Span::styled(
                "✓ Feature Release",
                Style::default().fg(Color::Green),
            )));
            lines.push(Line::from(Span::styled(
                "  Backwards compatible additions",
                Style::default().fg(Color::Gray),
            )));
        }
        BumpChoice::Patch => {
            lines.push(Line::from(Span::styled(
                "✓ Patch Release",
                Style::default().fg(Color::Blue),
            )));
            lines.push(Line::from(Span::styled(
                "  Bug fixes only",
                Style::default().fg(Color::Gray),
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
            Span::styled("  other:     ", Style::default().fg(Color::Gray)),
            Span::styled(format!("{}", other_count), Style::default().fg(Color::Gray)),
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
            Color::Gray
        };

        lines.push(Line::from(Span::styled(
            format!("  {} {}", if i < 9 { "•" } else { " " }, truncated),
            Style::default().fg(color),
        )));
    }

    if commits.len() > 8 {
        lines.push(Line::from(Span::styled(
            format!("  ... and {} more", commits.len() - 8),
            Style::default().fg(Color::Gray),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Press Tab to preview full changelog",
        Style::default().fg(Color::Gray),
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
        WizardStep::UnitConfig { unit_index } => *unit_index + 1,
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

    let scroll_offset = state.changelog_scroll_offset;
    if state.changelog_toggle.is_right() {
        let raw_text = Text::from(changelog_content.clone());
        let paragraph = Paragraph::new(raw_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Gray))
                    .title(Span::styled(
                        " Markdown Source ",
                        Style::default().fg(Color::Magenta),
                    ))
                    .padding(Padding::horizontal(1)),
            )
            .wrap(Wrap { trim: false })
            .scroll((scroll_offset, 0))
            .style(Style::default().fg(Color::Rgb(180, 180, 180)));
        f.render_widget(paragraph, chunks[2]);
    } else {
        let markdown_text = markdown::render_markdown(&changelog_content);
        let paragraph = Paragraph::new(markdown_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Gray))
                    .title(Span::styled(
                        " Rendered Preview ",
                        Style::default().fg(Color::Cyan),
                    ))
                    .padding(Padding::horizontal(1)),
            )
            .wrap(Wrap { trim: false })
            .scroll((scroll_offset, 0));
        f.render_widget(paragraph, chunks[2]);
    }

    let hints = Line::from(vec![
        Span::styled("↑↓", Style::default().fg(Color::Cyan)),
        Span::styled(" scroll  ", Style::default().fg(Color::Gray)),
        Span::styled("m", Style::default().fg(Color::Cyan)),
        Span::styled(" toggle view  ", Style::default().fg(Color::Gray)),
        Span::styled("Tab", Style::default().fg(Color::Cyan)),
        Span::styled(" back to bump  ", Style::default().fg(Color::Gray)),
        Span::styled("Enter", Style::default().fg(Color::Green)),
        Span::styled(" next  ", Style::default().fg(Color::Gray)),
        Span::styled("Esc", Style::default().fg(Color::Yellow)),
        Span::styled(" back  ", Style::default().fg(Color::Gray)),
        Span::styled("q", Style::default().fg(Color::Red)),
        Span::styled(" quit", Style::default().fg(Color::Gray)),
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
            Style::default().fg(Color::Gray),
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
                Style::default().fg(Color::Gray),
            ),
        ]));
        project_lines.push(Line::from(vec![
            Span::styled("      → ", Style::default().fg(Color::Gray)),
            Span::styled(bump_text, Style::default().fg(bump_color)),
        ]));
    }

    if selected_projects.len() > 10 {
        project_lines.push(Line::from(Span::styled(
            format!("   ... and {} more", selected_projects.len() - 10),
            Style::default().fg(Color::Gray),
        )));
    }

    let project_block = Paragraph::new(project_lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Gray))
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

    let mut ecosystems: std::collections::HashSet<Ecosystem> = std::collections::HashSet::new();
    for project in &selected_projects {
        ecosystems.insert(project.ecosystem().clone());
    }

    for ecosystem in &ecosystems {
        file_lines.push(Line::from(vec![
            Span::styled("   ✏️  ", Style::default().fg(Color::Yellow)),
            Span::styled(
                ecosystem.version_file().to_string(),
                Style::default().fg(Color::Gray),
            ),
        ]));
    }
    file_lines.push(Line::from(vec![
        Span::styled("   📝 ", Style::default().fg(Color::Cyan)),
        Span::styled("CHANGELOG.md", Style::default().fg(Color::Gray)),
    ]));
    file_lines.push(Line::from(vec![
        Span::styled("   📋 ", Style::default().fg(Color::Magenta)),
        Span::styled("belaf/releases/*.json", Style::default().fg(Color::Gray)),
    ]));

    let file_block = Paragraph::new(file_lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Gray))
            .title(Span::styled(
                " Will Execute ",
                Style::default().fg(Color::White),
            )),
    );
    f.render_widget(file_block, content_chunks[1]);

    let hints = Line::from(vec![
        Span::styled("Enter", Style::default().fg(Color::Green)),
        Span::styled(" confirm  ", Style::default().fg(Color::Gray)),
        Span::styled("Esc", Style::default().fg(Color::Yellow)),
        Span::styled(" back  ", Style::default().fg(Color::Gray)),
        Span::styled("?", Style::default().fg(Color::Yellow)),
        Span::styled(" help  ", Style::default().fg(Color::Gray)),
        Span::styled("q", Style::default().fg(Color::Red)),
        Span::styled(" quit", Style::default().fg(Color::Gray)),
    ]);
    let hints_para = Paragraph::new(hints).alignment(ratatui::layout::Alignment::Center);
    f.render_widget(hints_para, chunks[2]);
}

fn render_help_popup(f: &mut Frame, state: &WizardState) {
    let area = centered_rect(60, 70, f.area());

    let help_text = match &state.step {
        WizardStep::ReleaseUnitSelection => {
            "ReleaseUnit Selection Help\n\n\
             • Use ↑/↓ arrows to navigate units\n\
             • Press Space to toggle unit selection\n\
             • Press 'a' to toggle all units\n\
             • Press Enter to proceed to next step\n\
             • At least one project must be selected\n\n\
             The wizard analyzes your commits using\n\
             Conventional Commits to suggest version bumps.\n\n\
             Each selected project will be configured\n\
             individually in the next steps."
        }
        WizardStep::UnitConfig { .. } => {
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

fn apply_decisions_to_items(
    projects: &mut [ReleaseUnitItem],
    decisions: &[crate::core::bump_source::BumpDecision],
) -> Result<()> {
    if decisions.is_empty() {
        return Ok(());
    }
    let names: Vec<String> = projects.iter().map(|p| p.name().to_string()).collect();
    for d in decisions {
        let Some(p) = projects.iter_mut().find(|p| p.name() == d.release_unit) else {
            return Err(anyhow::anyhow!(
                "bump-source decision for `{}` does not match any project. Available: {}",
                d.release_unit,
                names.join(", ")
            ));
        };
        let choice = match d.bump.as_str() {
            "major" => BumpChoice::Major,
            "minor" => BumpChoice::Minor,
            "patch" => BumpChoice::Patch,
            other => {
                return Err(anyhow::anyhow!(
                    "bump-source decision for `{}` has invalid bump value `{}`",
                    d.release_unit,
                    other
                ));
            }
        };
        info!(
            "bump-source decision: {} -> {}",
            d.release_unit,
            choice.as_str()
        );
        p.chosen_bump = Some(choice);
    }
    Ok(())
}

fn apply_project_overrides_to_items(
    projects: &mut [ReleaseUnitItem],
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::bump::BumpRecommendation;
    use crate::core::wire::known::Ecosystem;

    /// Build a minimal `ReleaseUnitItem` for layout tests. We don't need
    /// realistic Commit / config state — the layout algorithm only
    /// reads `group_id()`.
    fn item(name: &str, group_id: Option<&str>) -> ReleaseUnitItem {
        ReleaseUnitItem {
            candidate: ReleaseUnitCandidate {
                ident: 0,
                name: name.into(),
                prefix: String::new(),
                current_version: "0.1.0".into(),
                commits: Vec::new(),
                commit_count: 0,
                suggested_bump: BumpRecommendation::Patch,
                ecosystem: Ecosystem::classify("npm"),
            },
            selected: true,
            chosen_bump: None,
            cached_changelog: None,
            existing_changelog: String::new(),
            group_id: group_id.map(str::to_string),
        }
    }

    /// Plan §5: solo + group projects collapse correctly into display
    /// rows. Two grouped npm + maven members render as ONE row;
    /// other solos each get their own.
    #[test]
    fn compute_display_rows_collapses_grouped_into_one_row() {
        let projects = vec![
            item("@org/utils", None),
            item("@org/schema", Some("schema-bundle")),
            item("com.org:schema", Some("schema-bundle")),
            item("cli", None),
        ];
        let rows = compute_display_rows(&projects);
        assert_eq!(
            rows.len(),
            3,
            "expected 3 display rows (utils + schema-bundle + cli), got {}",
            rows.len(),
        );
        match &rows[0] {
            DisplayRow::Solo { unit_idx } => assert_eq!(*unit_idx, 0),
            other => panic!("row 0 should be Solo, got {other:?}"),
        }
        match &rows[1] {
            DisplayRow::Group { id, member_indices } => {
                assert_eq!(id, "schema-bundle");
                assert_eq!(member_indices, &vec![1, 2]);
            }
            other => panic!("row 1 should be Group(schema-bundle), got {other:?}"),
        }
        match &rows[2] {
            DisplayRow::Solo { unit_idx } => assert_eq!(*unit_idx, 3),
            other => panic!("row 2 should be Solo(cli), got {other:?}"),
        }
    }

    /// Non-adjacent group members still collapse — the parser walks in
    /// project order but the layout keys by group id, not by adjacency.
    #[test]
    fn compute_display_rows_handles_non_adjacent_group_members() {
        let projects = vec![
            item("@org/schema", Some("schema-bundle")),
            item("@org/utils", None), // unrelated solo between members
            item("com.org:schema", Some("schema-bundle")),
        ];
        let rows = compute_display_rows(&projects);
        assert_eq!(rows.len(), 2, "got {rows:?}");
        match &rows[0] {
            DisplayRow::Group { id, member_indices } => {
                assert_eq!(id, "schema-bundle");
                assert_eq!(member_indices, &vec![0, 2]);
            }
            other => panic!("expected Group first, got {other:?}"),
        }
    }

    /// Two distinct groups in the same project list each get one row.
    #[test]
    fn compute_display_rows_separates_distinct_groups() {
        let projects = vec![
            item("@org/a", Some("group-a")),
            item("@org/b", Some("group-b")),
            item("@org/a-helper", Some("group-a")),
        ];
        let rows = compute_display_rows(&projects);
        assert_eq!(rows.len(), 2);
        let group_ids: Vec<&str> = rows
            .iter()
            .filter_map(|r| match r {
                DisplayRow::Group { id, .. } => Some(id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(group_ids, vec!["group-a", "group-b"]);
    }

    /// Empty input → empty rows. Edge case for `belaf prepare` against
    /// a freshly-released repo with no candidates.
    #[test]
    fn compute_display_rows_empty_input() {
        let rows = compute_display_rows(&[]);
        assert!(rows.is_empty());
    }
}
