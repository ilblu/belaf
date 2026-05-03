//! Header + body + hints rendering for the init wizard's unified
//! selection step. The actual row layout is delegated to
//! [`ReleaseUnitView::render_with_overlay`]; this module only owns
//! the surrounding chrome (block, summary line, key-hints footer).

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::core::ui::glyphs;
use crate::core::ui::release_unit_view::{
    render_summary, PrepareOverlay, ReleaseUnitView, RenderMode, ViewContext,
};

pub(super) fn render(
    frame: &mut Frame,
    area: Rect,
    view: &ReleaseUnitView,
    overlay: &PrepareOverlay,
    cursor: usize,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " ReleaseUnit Selection ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(inner);

    let selected = view.selected_togglable_count();
    let total = view.bundles.len() + view.units.len();
    let header = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(
                format!("{} ", glyphs::header_clipboard()),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(
                "Review and toggle each ReleaseUnit you want belaf to manage",
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("   ", Style::default()),
            Span::styled(
                format!("{}", selected),
                Style::default()
                    .fg(if selected > 0 {
                        Color::Green
                    } else {
                        Color::Yellow
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" / {} selected · {}", total, render_summary(view)),
                Style::default().fg(Color::Gray),
            ),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(header).alignment(Alignment::Center),
        chunks[0],
    );

    let ctx = ViewContext {
        mode: RenderMode::Init,
        cursor: Some(cursor),
    };
    view.render_with_overlay(frame, chunks[1], &ctx, overlay);

    let hints = Line::from(vec![
        Span::styled("↑↓", Style::default().fg(Color::Cyan)),
        Span::styled(" navigate  ", Style::default().fg(Color::Gray)),
        Span::styled("Space", Style::default().fg(Color::Cyan)),
        Span::styled(" toggle  ", Style::default().fg(Color::Gray)),
        Span::styled("c", Style::default().fg(Color::Magenta)),
        Span::styled(" cascade-from  ", Style::default().fg(Color::Gray)),
        Span::styled("a/n", Style::default().fg(Color::Green)),
        Span::styled(" all/none  ", Style::default().fg(Color::Gray)),
        Span::styled("Enter", Style::default().fg(Color::Green)),
        Span::styled(" continue  ", Style::default().fg(Color::Gray)),
        Span::styled("Esc", Style::default().fg(Color::Yellow)),
        Span::styled(" back  ", Style::default().fg(Color::Gray)),
        Span::styled("q", Style::default().fg(Color::Red)),
        Span::styled(" quit", Style::default().fg(Color::Gray)),
    ]);
    frame.render_widget(
        Paragraph::new(hints).alignment(Alignment::Center),
        chunks[2],
    );
}
