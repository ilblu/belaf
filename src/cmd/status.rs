use std::io;

use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row},
    Terminal,
};
use tracing::info;

use crate::atry;
use crate::cli::ReleaseOutputFormat;
use crate::core::ui::{
    components::{status_bar::StatusBar, table::Table},
    theme::AppColors,
};
use crate::core::{graph::GraphQueryBuilder, session::AppSession};

struct ProjectStatus {
    name: String,
    version: Option<String>,
    commits_count: usize,
    age: Option<usize>,
    commits: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectablePanel {
    Projects,
    Commits,
}

impl SelectablePanel {
    fn next(self) -> Self {
        match self {
            Self::Projects => Self::Commits,
            Self::Commits => Self::Projects,
        }
    }
}

struct TuiState {
    selected_panel: SelectablePanel,
    selected_project_index: usize,
    commit_scroll_offset: usize,
    project_data: Vec<ProjectStatus>,
    colors: AppColors,
    should_quit: bool,
    show_help: bool,
}

impl TuiState {
    fn new(project_data: Vec<ProjectStatus>) -> Self {
        Self {
            selected_panel: SelectablePanel::Projects,
            selected_project_index: 0,
            commit_scroll_offset: 0,
            project_data,
            colors: AppColors::default(),
            should_quit: false,
            show_help: false,
        }
    }

    fn current_project_commits(&self) -> usize {
        self.project_data
            .get(self.selected_project_index)
            .map(|p| p.commits.len())
            .unwrap_or(0)
    }

    fn handle_key_event(&mut self, key: KeyCode, modifiers: KeyModifiers) {
        if self.show_help {
            match key {
                KeyCode::Esc
                | KeyCode::Char('h')
                | KeyCode::Char('?')
                | KeyCode::Char('q')
                | KeyCode::Enter => {
                    self.show_help = false;
                }
                _ => {}
            }
            return;
        }

        match (key, modifiers) {
            (KeyCode::Char('q'), _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }
            (KeyCode::Char('h'), _) | (KeyCode::Char('?'), _) => {
                self.show_help = true;
            }
            (KeyCode::Tab, _) | (KeyCode::BackTab, _) => {
                self.selected_panel = self.selected_panel.next();
            }
            (KeyCode::Down | KeyCode::Char('j'), _) => match self.selected_panel {
                SelectablePanel::Projects => {
                    if !self.project_data.is_empty() {
                        self.selected_project_index =
                            (self.selected_project_index + 1) % self.project_data.len();
                        self.commit_scroll_offset = 0;
                    }
                }
                SelectablePanel::Commits => {
                    let total_commits = self.current_project_commits();
                    if total_commits > 0 {
                        self.commit_scroll_offset =
                            (self.commit_scroll_offset + 1).min(total_commits.saturating_sub(1));
                    }
                }
            },
            (KeyCode::Up | KeyCode::Char('k'), _) => match self.selected_panel {
                SelectablePanel::Projects => {
                    if !self.project_data.is_empty() {
                        self.selected_project_index = if self.selected_project_index == 0 {
                            self.project_data.len() - 1
                        } else {
                            self.selected_project_index - 1
                        };
                        self.commit_scroll_offset = 0;
                    }
                }
                SelectablePanel::Commits => {
                    if self.commit_scroll_offset > 0 {
                        self.commit_scroll_offset -= 1;
                    }
                }
            },
            (KeyCode::Home | KeyCode::Char('g'), _) => match self.selected_panel {
                SelectablePanel::Projects => self.selected_project_index = 0,
                SelectablePanel::Commits => self.commit_scroll_offset = 0,
            },
            (KeyCode::End | KeyCode::Char('G'), KeyModifiers::SHIFT) => match self.selected_panel {
                SelectablePanel::Projects => {
                    if !self.project_data.is_empty() {
                        self.selected_project_index = self.project_data.len() - 1;
                    }
                }
                SelectablePanel::Commits => {
                    let total_commits = self.current_project_commits();
                    if total_commits > 0 {
                        self.commit_scroll_offset = total_commits.saturating_sub(1);
                    }
                }
            },
            (KeyCode::PageDown, _) => {
                if self.selected_panel == SelectablePanel::Commits {
                    let total_commits = self.current_project_commits();
                    if total_commits > 0 {
                        self.commit_scroll_offset =
                            (self.commit_scroll_offset + 10).min(total_commits.saturating_sub(1));
                    }
                }
            }
            (KeyCode::PageUp, _) => {
                if self.selected_panel == SelectablePanel::Commits {
                    self.commit_scroll_offset = self.commit_scroll_offset.saturating_sub(10);
                }
            }
            _ => {}
        }
    }

    fn render(&self, frame: &mut ratatui::Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(frame.area());

        self.render_header(frame, chunks[0]);
        self.render_projects(frame, chunks[1]);
        self.render_footer(frame, chunks[2]);

        if self.show_help {
            self.render_help_popup(frame);
        }
    }

    fn render_help_popup(&self, frame: &mut ratatui::Frame) {
        let area = frame.area();
        let popup_width = 65u16.min(area.width.saturating_sub(4));
        let popup_height = 20u16.min(area.height.saturating_sub(4));

        let popup_area = Rect {
            x: (area.width.saturating_sub(popup_width)) / 2,
            y: (area.height.saturating_sub(popup_height)) / 2,
            width: popup_width,
            height: popup_height,
        };

        frame.render_widget(Clear, popup_area);

        let block = Block::default()
            .title(" Help ")
            .title_alignment(Alignment::Center)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .style(Style::default().bg(Color::Rgb(20, 20, 30)));

        let help_items = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("  Tab / Shift+Tab  ", Style::default().fg(Color::Yellow)),
                Span::styled("Switch between panels", Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("  ↑ / k            ", Style::default().fg(Color::Yellow)),
                Span::styled("Move selection up", Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("  ↓ / j            ", Style::default().fg(Color::Yellow)),
                Span::styled("Move selection down", Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("  g / Home         ", Style::default().fg(Color::Yellow)),
                Span::styled("Jump to first item", Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("  G / End          ", Style::default().fg(Color::Yellow)),
                Span::styled("Jump to last item", Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("  PgUp / PgDn      ", Style::default().fg(Color::Yellow)),
                Span::styled("Scroll commits by 10", Style::default().fg(Color::White)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  h / ?            ", Style::default().fg(Color::Cyan)),
                Span::styled("Toggle this help", Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("  q / Ctrl+C       ", Style::default().fg(Color::Red)),
                Span::styled("Quit", Style::default().fg(Color::White)),
            ]),
            Line::from(""),
            Span::styled(
                "Press any key to close",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            )
            .into_centered_line(),
        ];

        let help_text = Paragraph::new(help_items)
            .block(block)
            .alignment(Alignment::Left);

        frame.render_widget(help_text, popup_area);
    }

    fn render_header(&self, frame: &mut ratatui::Frame, area: Rect) {
        let block = Block::default()
            .title("Release Status")
            .borders(Borders::ALL)
            .style(
                Style::default()
                    .fg(self.colors.headers_bar.text)
                    .bg(self.colors.headers_bar.background),
            );

        frame.render_widget(block, area);
    }

    fn render_projects(&self, frame: &mut ratatui::Frame, area: Rect) {
        if self.project_data.is_empty() {
            return;
        }

        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(area);

        self.render_project_list(frame, chunks[0]);
        self.render_project_details(frame, chunks[1]);
    }

    fn render_project_list(&self, frame: &mut ratatui::Frame, area: Rect) {
        let header = Row::new(vec![
            Cell::from("Project"),
            Cell::from("Version"),
            Cell::from("Commits"),
        ])
        .style(
            Style::default()
                .fg(self.colors.headers_bar.text)
                .add_modifier(Modifier::BOLD),
        );

        let rows: Vec<Row> = self
            .project_data
            .iter()
            .enumerate()
            .map(|(idx, proj)| {
                let style = if idx == self.selected_project_index {
                    Style::default()
                        .bg(self.colors.containers.background)
                        .fg(self.colors.containers.text)
                } else {
                    Style::default()
                };

                Row::new(vec![
                    Cell::from(proj.name.clone()),
                    Cell::from(proj.version.clone().unwrap_or_else(|| "N/A".to_string())),
                    Cell::from(proj.commits_count.to_string()),
                ])
                .style(style)
            })
            .collect();

        let widths = [
            Constraint::Percentage(50),
            Constraint::Percentage(30),
            Constraint::Percentage(20),
        ];

        let border_color = if self.selected_panel == SelectablePanel::Projects {
            self.colors.borders.selected
        } else {
            self.colors.borders.unselected
        };

        let block = Block::default()
            .title("Projects")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color));

        let table = Table::new(rows, &widths)
            .header(header)
            .block(block)
            .highlight_style(
                Style::default()
                    .bg(self.colors.containers.background)
                    .fg(self.colors.containers.text),
            );

        table.render(frame, area);
    }

    fn render_project_details(&self, frame: &mut ratatui::Frame, area: Rect) {
        if let Some(proj) = self.project_data.get(self.selected_project_index) {
            let header = Row::new(vec![Cell::from("#"), Cell::from("Commit Summary")]).style(
                Style::default()
                    .fg(self.colors.headers_bar.text)
                    .add_modifier(Modifier::BOLD),
            );

            let available_height = area.height.saturating_sub(3);
            let visible_start = self.commit_scroll_offset;
            let visible_end = (visible_start + available_height as usize).min(proj.commits.len());

            let rows: Vec<Row> = proj
                .commits
                .iter()
                .enumerate()
                .skip(visible_start)
                .take(available_height as usize)
                .map(|(idx, commit)| {
                    Row::new(vec![
                        Cell::from((idx + 1).to_string()),
                        Cell::from(commit.clone()),
                    ])
                })
                .collect();

            let widths = [Constraint::Length(5), Constraint::Percentage(95)];

            let scroll_indicator = if proj.commits.len() > available_height as usize {
                format!(
                    " [{}-{}/{}]",
                    visible_start + 1,
                    visible_end,
                    proj.commits.len()
                )
            } else {
                String::new()
            };

            let title = if let Some(version) = &proj.version {
                if proj.age.unwrap_or(0) == 0 {
                    format!(
                        "{} - {} commit(s) since {}{}",
                        proj.name, proj.commits_count, version, scroll_indicator
                    )
                } else {
                    format!(
                        "{} - ≤{} commit(s) since {} (inexact){}",
                        proj.name, proj.commits_count, version, scroll_indicator
                    )
                }
            } else {
                format!(
                    "{} - {} commit(s) (no releases){}",
                    proj.name, proj.commits_count, scroll_indicator
                )
            };

            let border_color = if self.selected_panel == SelectablePanel::Commits {
                self.colors.borders.selected
            } else {
                self.colors.borders.unselected
            };

            let block = Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color));

            if rows.is_empty() {
                frame.render_widget(block, area);
            } else {
                let table = Table::new(rows, &widths).header(header).block(block);
                table.render(frame, area);
            }
        }
    }

    fn render_footer(&self, frame: &mut ratatui::Frame, area: Rect) {
        let (panel_name, count_text) = if self.project_data.is_empty() {
            ("Projects", "No projects".to_string())
        } else {
            match self.selected_panel {
                SelectablePanel::Projects => (
                    "Projects",
                    format!(
                        "Project {}/{}",
                        self.selected_project_index + 1,
                        self.project_data.len()
                    ),
                ),
                SelectablePanel::Commits => {
                    let total_commits = self.current_project_commits();
                    if total_commits > 0 {
                        (
                            "Commits",
                            format!("Commit {}/{}", self.commit_scroll_offset + 1, total_commits),
                        )
                    } else {
                        ("Commits", "No commits".to_string())
                    }
                }
            }
        };

        let center_text = format!("[{}] {}", panel_name, count_text);

        let status_bar = StatusBar::new(
            " Tab: Switch  ↑/↓: Scroll  PgUp/PgDn  g/G: Top/Bottom  h: Help",
            &center_text,
            "q: Quit ",
        )
        .style(
            Style::default()
                .bg(self.colors.headers_bar.background)
                .fg(self.colors.headers_bar.text),
        );

        status_bar.render(frame, area);
    }
}

fn run_tui(sess: &AppSession, idents: &[usize]) -> Result<()> {
    let histories = sess.analyze_histories()?;
    let mut project_data = Vec::new();

    for ident in idents {
        let proj = sess.graph().lookup(*ident);
        let history = histories.lookup(*ident);
        let n = history.n_commits();
        let rel_info = history.release_info(&sess.repo)?;

        let mut commits = Vec::new();
        for cid in history.commits() {
            let summary = sess.repo.get_commit_summary(*cid)?;
            commits.push(summary);
        }

        let (version, age) = if let Some(this_info) = rel_info.lookup_project(proj) {
            (Some(this_info.version.to_string()), Some(this_info.age))
        } else {
            (None, None)
        };

        project_data.push(ProjectStatus {
            name: proj.user_facing_name.clone(),
            version,
            commits_count: n,
            age,
            commits,
        });
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut state = TuiState::new(project_data);

    loop {
        terminal.draw(|frame| state.render(frame))?;

        if state.should_quit {
            break;
        }

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                state.handle_key_event(key.code, key.modifiers);
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

    Ok(())
}

pub fn run(format: Option<ReleaseOutputFormat>, ci: bool) -> Result<i32> {
    use crate::core::ui::utils::should_use_tui;

    info!(
        "checking release status with belaf version {}",
        env!("CARGO_PKG_VERSION")
    );

    let sess = atry!(
        AppSession::initialize_default();
        ["could not initialize app and project graph"]
    );

    let q = GraphQueryBuilder::default();
    let idents = sess
        .graph()
        .query(q)
        .context("cannot get requested statuses")?;

    let histories = sess.analyze_histories()?;

    let use_tui = should_use_tui(ci, &format);

    let output_format = if ci {
        ReleaseOutputFormat::Json
    } else {
        format.unwrap_or(ReleaseOutputFormat::Text)
    };

    if use_tui {
        run_tui(&sess, &idents)?;
        return Ok(0);
    }

    match output_format {
        ReleaseOutputFormat::Json => {
            use serde_json::json;

            let mut projects = Vec::new();

            for ident in &idents {
                let proj = sess.graph().lookup(*ident);
                let history = histories.lookup(*ident);
                let n = history.n_commits();
                let rel_info = history.release_info(&sess.repo)?;

                let mut commits = Vec::new();
                for cid in history.commits() {
                    let summary = sess.repo.get_commit_summary(*cid)?;
                    commits.push(summary);
                }

                let project_data = if let Some(this_info) = rel_info.lookup_project(proj) {
                    json!({
                        "name": proj.user_facing_name,
                        "current_version": this_info.version.to_string(),
                        "commits_count": n,
                        "commits": commits,
                        "age": this_info.age,
                    })
                } else {
                    json!({
                        "name": proj.user_facing_name,
                        "current_version": null,
                        "commits_count": n,
                        "commits": commits,
                        "age": null,
                    })
                };

                projects.push(project_data);
            }

            let output = json!({
                "projects": projects
            });

            println!("{}", serde_json::to_string_pretty(&output)?);
        }
        _ => {
            for ident in idents {
                let proj = sess.graph().lookup(ident);
                let history = histories.lookup(ident);
                let n = history.n_commits();
                let rel_info = history.release_info(&sess.repo)?;

                if let Some(this_info) = rel_info.lookup_project(proj) {
                    if this_info.age == 0 {
                        if n == 0 {
                            println!(
                                "{}: no relevant commits since {}",
                                proj.user_facing_name, this_info.version
                            );
                        } else {
                            println!(
                                "{}: {} relevant commit(s) since {}",
                                proj.user_facing_name, n, this_info.version
                            );
                        }
                    } else {
                        println!(
                            "{}: no more than {} relevant commit(s) since {} (unable to track in detail)",
                            proj.user_facing_name, n, this_info.version
                        );
                    }
                } else {
                    println!(
                        "{}: {} relevant commit(s) since start of history (no releases on record)",
                        proj.user_facing_name, n
                    );
                }

                for (idx, cid) in history.commits().into_iter().enumerate() {
                    let summary = sess.repo.get_commit_summary(*cid)?;
                    println!("    {}. {}", idx + 1, summary);
                }

                if n > 0 {
                    println!();
                }
            }
        }
    }

    Ok(0)
}
