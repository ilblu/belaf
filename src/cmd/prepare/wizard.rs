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
use ratatui::{backend::CrosstermBackend, widgets::ListState, Terminal};
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
    session::AppBuilder,
    ui::components::toggle_panel::TogglePanel,
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
    Solo { unit_idx: usize },
    Group { member_indices: Vec<usize> },
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

fn compute_display_rows(units: &[ReleaseUnitItem]) -> Vec<DisplayRow> {
    let mut out: Vec<DisplayRow> = Vec::new();
    let mut group_pos: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (i, p) in units.iter().enumerate() {
        match p.group_id() {
            Some(gid) => {
                if let Some(&pos) = group_pos.get(gid) {
                    if let DisplayRow::Group { member_indices } = &mut out[pos] {
                        member_indices.push(i);
                    }
                } else {
                    group_pos.insert(gid.to_string(), out.len());
                    out.push(DisplayRow::Group {
                        member_indices: vec![i],
                    });
                }
            }
            None => out.push(DisplayRow::Solo { unit_idx: i }),
        }
    }
    out
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

    /// Compute the per-render display rows. Solo (ungrouped) units
    /// are one row each; grouped units collapse into a single
    /// `Group` row whose `member_indices` point into `Self::units`.
    /// Cursor navigation moves through these rows so a 5-member
    /// group still navigates as one stop.
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

    let mut sess = AppBuilder::new()?
        .fetch_tags_first(true)
        .initialize()
        .context("could not initialize app and project graph")?;
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
        terminal.draw(|f| render::ui(f, state))?;

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

mod render;

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
            DisplayRow::Group { member_indices } => {
                assert_eq!(member_indices, &vec![1, 2]);
                assert_eq!(
                    projects[member_indices[0]].group_id(),
                    Some("schema-bundle")
                );
                assert_eq!(
                    projects[member_indices[1]].group_id(),
                    Some("schema-bundle")
                );
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
            DisplayRow::Group { member_indices } => {
                assert_eq!(member_indices, &vec![0, 2]);
                assert_eq!(projects[0].group_id(), Some("schema-bundle"));
                assert_eq!(projects[2].group_id(), Some("schema-bundle"));
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
                DisplayRow::Group { member_indices } => projects[member_indices[0]].group_id(),
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
