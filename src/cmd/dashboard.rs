use std::io::{self, stdout};

use anyhow::Result;
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
    widgets::Paragraph,
    Frame, Terminal,
};

use crate::core::{git::repository::Repository, session::AppSession};

const LOGO: [&str; 7] = [
    "██████╗ ███████╗██╗      █████╗ ███████╗",
    "██╔══██╗██╔════╝██║     ██╔══██╗██╔════╝",
    "██████╔╝█████╗  ██║     ███████║█████╗  ",
    "██╔══██╗██╔══╝  ██║     ██╔══██║██╔══╝  ",
    "██████╔╝███████╗███████╗██║  ██║██║     ",
    "╚═════╝ ╚══════╝╚══════╝╚═╝  ╚═╝╚═╝     ",
    "        Release Management              ",
];

struct MenuItem {
    icon: &'static str,
    label: &'static str,
    key: char,
}

const MENU_ITEMS: [MenuItem; 8] = [
    MenuItem {
        icon: "📦",
        label: "Prepare release",
        key: 'p',
    },
    MenuItem {
        icon: "📊",
        label: "Show status",
        key: 's',
    },
    MenuItem {
        icon: "🔗",
        label: "Dependency graph",
        key: 'g',
    },
    MenuItem {
        icon: "📝",
        label: "Generate changelog",
        key: 'c',
    },
    MenuItem {
        icon: "⚙ ",
        label: "Initialize project",
        key: 'i',
    },
    MenuItem {
        icon: "🌐",
        label: "Open web dashboard",
        key: 'w',
    },
    MenuItem {
        icon: "❓",
        label: "Help",
        key: '?',
    },
    MenuItem {
        icon: "🚪",
        label: "Quit",
        key: 'q',
    },
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DashboardAction {
    Prepare,
    Status,
    Graph,
    Changelog,
    Init,
    Web,
    Help,
    Quit,
    None,
}

struct DashboardStats {
    project_count: usize,
    pending_commits: usize,
    current_branch: String,
    is_initialized: bool,
}

impl Default for DashboardStats {
    fn default() -> Self {
        Self {
            project_count: 0,
            pending_commits: 0,
            current_branch: String::from("unknown"),
            is_initialized: false,
        }
    }
}

pub fn run() -> Result<DashboardAction> {
    let stats = load_stats();

    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_dashboard(&mut terminal, &stats);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

    result
}

fn load_stats() -> DashboardStats {
    let mut stats = DashboardStats::default();

    if let Ok(repo) = Repository::open_from_env() {
        if let Ok(Some(branch)) = repo.current_branch_name() {
            stats.current_branch = branch;
        }

        let config_path = repo.resolve_config_dir().join("config.toml");
        stats.is_initialized = config_path.exists();

        if stats.is_initialized {
            if let Ok(session) = AppSession::initialize_default() {
                stats.project_count = session.graph().projects().count();
                stats.pending_commits = count_pending_commits(&session);
            }
        }
    }

    stats
}

fn count_pending_commits(session: &AppSession) -> usize {
    if let Ok(histories) = session.analyze_histories() {
        session
            .graph()
            .projects()
            .map(|p| histories.lookup(p.ident()).n_commits())
            .sum()
    } else {
        0
    }
}

fn run_dashboard(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    stats: &DashboardStats,
) -> Result<DashboardAction> {
    loop {
        terminal.draw(|f| render(f, stats))?;

        if let Event::Key(key) = event::read()? {
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                return Ok(DashboardAction::Quit);
            }

            match key.code {
                KeyCode::Char('p') => return Ok(DashboardAction::Prepare),
                KeyCode::Char('s') => return Ok(DashboardAction::Status),
                KeyCode::Char('g') => return Ok(DashboardAction::Graph),
                KeyCode::Char('c') => return Ok(DashboardAction::Changelog),
                KeyCode::Char('i') => return Ok(DashboardAction::Init),
                KeyCode::Char('w') => return Ok(DashboardAction::Web),
                KeyCode::Char('?') | KeyCode::Char('h') => return Ok(DashboardAction::Help),
                KeyCode::Char('q') | KeyCode::Esc => return Ok(DashboardAction::Quit),
                _ => {}
            }
        }
    }
}

fn render(frame: &mut Frame, stats: &DashboardStats) {
    let area = frame.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(9),
            Constraint::Length(2),
            Constraint::Length(MENU_ITEMS.len() as u16 + 2),
            Constraint::Length(2),
            Constraint::Min(0),
        ])
        .split(area);

    render_logo(frame, chunks[1]);
    render_menu(frame, chunks[3], stats.is_initialized);
    render_stats(frame, chunks[4], stats);
}

fn render_logo(frame: &mut Frame, area: Rect) {
    let logo_lines: Vec<Line> = LOGO
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let color = if i == LOGO.len() - 1 {
                Color::Gray
            } else {
                Color::Cyan
            };
            Line::from(Span::styled(*line, Style::default().fg(color)))
        })
        .collect();

    let logo = Paragraph::new(logo_lines).alignment(Alignment::Center);
    frame.render_widget(logo, area);
}

fn render_menu(frame: &mut Frame, area: Rect, is_initialized: bool) {
    let menu_lines: Vec<Line> = MENU_ITEMS
        .iter()
        .map(|item| {
            let is_init_item = item.key == 'i';
            let should_highlight = !is_initialized && is_init_item;

            let key_style = if should_highlight {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Cyan)
            };

            let label_style = if should_highlight {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::White)
            };

            Line::from(vec![
                Span::raw("         "),
                Span::raw(item.icon),
                Span::raw("  "),
                Span::styled(format!("{:<24}", item.label), label_style),
                Span::styled(format!("{}", item.key), key_style),
            ])
        })
        .collect();

    let menu = Paragraph::new(menu_lines).alignment(Alignment::Left);

    let centered_area = centered_horizontal(area, 60);
    frame.render_widget(menu, centered_area);
}

fn render_stats(frame: &mut Frame, area: Rect, stats: &DashboardStats) {
    let stats_line = if stats.is_initialized {
        Line::from(vec![
            Span::raw("         "),
            Span::styled("⚡ ", Style::default().fg(Color::Yellow)),
            Span::styled(
                format!("{} projects", stats.project_count),
                Style::default().fg(Color::Gray),
            ),
            Span::styled(" │ ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("{} commits pending", stats.pending_commits),
                Style::default().fg(Color::Gray),
            ),
            Span::styled(" │ ", Style::default().fg(Color::Gray)),
            Span::styled(
                stats.current_branch.clone(),
                Style::default().fg(Color::Cyan),
            ),
        ])
    } else {
        Line::from(vec![
            Span::raw("         "),
            Span::styled("⚠️  ", Style::default().fg(Color::Yellow)),
            Span::styled("Not initialized - press ", Style::default().fg(Color::Gray)),
            Span::styled(
                "i",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" to set up", Style::default().fg(Color::Gray)),
        ])
    };

    let stats_widget = Paragraph::new(vec![Line::from(""), stats_line]).alignment(Alignment::Left);

    let centered_area = centered_horizontal(area, 60);
    frame.render_widget(stats_widget, centered_area);
}

fn centered_horizontal(area: Rect, width: u16) -> Rect {
    let actual_width = width.min(area.width);
    let x = area.x + (area.width.saturating_sub(actual_width)) / 2;
    Rect::new(x, area.y, actual_width, area.height)
}
