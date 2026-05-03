use std::io::IsTerminal;

use ratatui::layout::{Constraint, Direction, Layout, Rect};

pub fn is_interactive_terminal() -> bool {
    std::io::stdout().is_terminal() && std::io::stdin().is_terminal()
}

pub fn should_use_tui<T>(ci_mode: bool, format: &Option<T>) -> bool {
    !ci_mode && format.is_none() && is_interactive_terminal()
}

pub fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
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
