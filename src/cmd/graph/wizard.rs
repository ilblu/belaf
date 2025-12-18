use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    prelude::*,
    widgets::{
        Block, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState,
    },
};
use std::collections::HashMap;
use std::io::stdout;

use crate::{
    atry,
    core::release::{graph::GraphQueryBuilder, session::AppSession},
};

use super::browser;

struct ProjectInfo {
    name: String,
    version: String,
    deps: Vec<String>,
    dependents: Vec<String>,
}

struct App {
    projects: Vec<ProjectInfo>,
    list_state: ListState,
    release_order: Vec<String>,
    show_help: bool,
}

impl App {
    fn new(sess: &AppSession, idents: &[usize]) -> Self {
        let mut projects = Vec::new();
        let mut name_to_idx: HashMap<String, usize> = HashMap::new();

        for (idx, &ident) in idents.iter().enumerate() {
            let proj = sess.graph().lookup(ident);
            name_to_idx.insert(proj.user_facing_name.clone(), idx);
        }

        for &ident in idents.iter() {
            let proj = sess.graph().lookup(ident);
            let deps: Vec<String> = proj
                .internal_deps
                .iter()
                .map(|d| {
                    let dep_proj = sess.graph().lookup(d.ident);
                    dep_proj.user_facing_name.clone()
                })
                .collect();

            let dependents: Vec<String> = idents
                .iter()
                .filter_map(|&other_ident| {
                    if other_ident == ident {
                        return None;
                    }
                    let other_proj = sess.graph().lookup(other_ident);
                    if other_proj.internal_deps.iter().any(|d| d.ident == ident) {
                        Some(other_proj.user_facing_name.clone())
                    } else {
                        None
                    }
                })
                .collect();

            projects.push(ProjectInfo {
                name: proj.user_facing_name.clone(),
                version: proj.version.to_string(),
                deps,
                dependents,
            });
        }

        let release_order: Vec<String> = sess
            .graph()
            .toposorted()
            .map(|id| sess.graph().lookup(id).user_facing_name.clone())
            .collect();

        let mut list_state = ListState::default();
        list_state.select(Some(0));

        Self {
            projects,
            list_state,
            release_order,
            show_help: false,
        }
    }

    fn selected_idx(&self) -> usize {
        self.list_state.selected().unwrap_or(0)
    }

    fn selected_project(&self) -> Option<&ProjectInfo> {
        self.projects.get(self.selected_idx())
    }

    fn next(&mut self) {
        if self.projects.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) => (i + 1) % self.projects.len(),
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    fn previous(&mut self) {
        if self.projects.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.projects.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    fn release_position(&self, name: &str) -> Option<usize> {
        self.release_order
            .iter()
            .position(|n| n == name)
            .map(|p| p + 1)
    }
}

pub fn run() -> Result<i32> {
    let sess = atry!(
        AppSession::initialize_default();
        ["could not initialize app and project graph"]
    );

    let q = GraphQueryBuilder::default();
    let idents = sess
        .graph()
        .query(q)
        .map_err(|e| anyhow::anyhow!("could not select projects: {}", e))?;

    if idents.is_empty() {
        println!("No projects found in repository");
        return Ok(0);
    }

    let mut app = App::new(&sess, &idents);

    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let result = run_app(&mut terminal, &mut app);

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;

    result
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> Result<i32> {
    loop {
        terminal.draw(|f| ui(f, app))?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            if app.show_help {
                match key.code {
                    KeyCode::Esc
                    | KeyCode::Char('h')
                    | KeyCode::Char('?')
                    | KeyCode::Char('q')
                    | KeyCode::Enter => {
                        app.show_help = false;
                    }
                    _ => {}
                }
                continue;
            }

            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(0),
                KeyCode::Down | KeyCode::Char('j') => app.next(),
                KeyCode::Up | KeyCode::Char('k') => app.previous(),
                KeyCode::Char('h') | KeyCode::Char('?') => app.show_help = true,
                KeyCode::Char('g') => {
                    disable_raw_mode()?;
                    stdout().execute(LeaveAlternateScreen)?;
                    browser::open_browser(None)?;
                    enable_raw_mode()?;
                    stdout().execute(EnterAlternateScreen)?;
                }
                KeyCode::Home => app.list_state.select(Some(0)),
                KeyCode::End => app
                    .list_state
                    .select(Some(app.projects.len().saturating_sub(1))),
                _ => {}
            }
        }
    }
}

fn ui(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(10),
            Constraint::Length(2),
        ])
        .split(f.area());

    let title = Paragraph::new(" ◆ Dependency Graph").style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );
    f.render_widget(title, chunks[0]);

    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[1]);

    render_packages_panel(f, app, main_chunks[0]);
    render_details_panel(f, app, main_chunks[1]);

    let help_text = " ↑↓/jk: Navigate │ g: Browser Graph │ h: Help │ q: Quit";
    let help = Paragraph::new(help_text)
        .style(Style::default().fg(Color::DarkGray))
        .block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(Color::DarkGray)),
        );
    f.render_widget(help, chunks[2]);

    if app.show_help {
        render_help_popup(f);
    }
}

fn render_help_popup(f: &mut Frame) {
    let area = f.area();
    let popup_width = 60u16.min(area.width.saturating_sub(4));
    let popup_height = 18u16.min(area.height.saturating_sub(4));

    let popup_area = Rect {
        x: (area.width.saturating_sub(popup_width)) / 2,
        y: (area.height.saturating_sub(popup_height)) / 2,
        width: popup_width,
        height: popup_height,
    };

    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" Help ")
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::Rgb(20, 20, 30)));

    let help_items = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  ↑ / k      ", Style::default().fg(Color::Yellow)),
            Span::styled("Move selection up", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("  ↓ / j      ", Style::default().fg(Color::Yellow)),
            Span::styled("Move selection down", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("  Home       ", Style::default().fg(Color::Yellow)),
            Span::styled("Jump to first project", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("  End        ", Style::default().fg(Color::Yellow)),
            Span::styled("Jump to last project", Style::default().fg(Color::White)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  g          ", Style::default().fg(Color::Green)),
            Span::styled(
                "Open interactive graph in browser",
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  h / ?      ", Style::default().fg(Color::Cyan)),
            Span::styled("Toggle this help", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("  q / Esc    ", Style::default().fg(Color::Red)),
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

    f.render_widget(help_text, popup_area);
}

fn render_packages_panel(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .title(format!(" Packages ({}) ", app.projects.len()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let items: Vec<ListItem> = app
        .projects
        .iter()
        .enumerate()
        .map(|(idx, p)| {
            let symbol = if p.deps.is_empty() { "○" } else { "●" };
            let deps_count = if p.deps.is_empty() {
                String::new()
            } else {
                format!(" [{}]", p.deps.len())
            };

            let is_selected = app.list_state.selected() == Some(idx);
            let marker = if is_selected { "▶" } else { " " };

            let line = format!(
                "{} {} {}  {}{}",
                marker, symbol, p.name, p.version, deps_count
            );

            let style = if is_selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else if p.deps.is_empty() {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::White)
            };

            ListItem::new(line).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default());

    f.render_stateful_widget(list, area, &mut app.list_state);

    if app.projects.len() > (area.height as usize).saturating_sub(2) {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"))
            .track_symbol(Some("│"))
            .thumb_symbol("█");

        let mut scrollbar_state = ScrollbarState::new(app.projects.len())
            .position(app.list_state.selected().unwrap_or(0));

        f.render_stateful_widget(
            scrollbar,
            area.inner(Margin {
                horizontal: 0,
                vertical: 1,
            }),
            &mut scrollbar_state,
        );
    }
}

fn render_details_panel(f: &mut Frame, app: &App, area: Rect) {
    let selected = app.selected_project();
    let title = selected
        .map(|p| format!(" {} ", p.name))
        .unwrap_or_else(|| " Details ".to_string());

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let Some(proj) = selected else {
        let no_selection =
            Paragraph::new("No project selected").style(Style::default().fg(Color::DarkGray));
        f.render_widget(no_selection, inner);
        return;
    };

    let release_pos = app.release_position(&proj.name).unwrap_or(0);
    let total = app.release_order.len();

    let mut lines = vec![
        Line::from(vec![
            Span::styled("Version: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&proj.version, Style::default().fg(Color::White)),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            format!("Dependencies ({})", proj.deps.len()),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )]),
    ];

    if proj.deps.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (none)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for dep in &proj.deps {
            lines.push(Line::from(Span::styled(
                format!("  → {}", dep),
                Style::default().fg(Color::Green),
            )));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled(
        format!("Dependents ({})", proj.dependents.len()),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )]));

    if proj.dependents.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (none)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for dep in &proj.dependents {
            lines.push(Line::from(Span::styled(
                format!("  ← {}", dep),
                Style::default().fg(Color::Cyan),
            )));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("Release Order: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("#{} of {}", release_pos, total),
            Style::default().fg(Color::Yellow),
        ),
    ]));

    let text = Text::from(lines);
    let paragraph = Paragraph::new(text);
    f.render_widget(paragraph, inner);
}
