//! Header + body + hints rendering for the init wizard's unified
//! selection step. The actual row layout is delegated to
//! [`ReleaseUnitView::render_with_overlay`]; this module only owns
//! the surrounding chrome (block, summary line, key-hints footer).

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::core::ui::release_unit_view::{
    render_summary, PrepareOverlay, ReleaseUnitView, RenderMode, ViewContext, ViewLayout,
};

use super::super::chrome::{self, palette, step_index, STEP_TOTAL};

pub(super) fn render(
    frame: &mut Frame,
    area: Rect,
    view: &ReleaseUnitView,
    overlay: &PrepareOverlay,
    cursor: usize,
) {
    let body = chrome::render_chrome(
        frame,
        area,
        "Select Projects",
        step_index::SELECTION,
        STEP_TOTAL,
    );
    let (content, hints_area) = chrome::split_body_with_hints(body);

    let body_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // section label + summary
            Constraint::Length(1), // blank
            Constraint::Length(1), // divider
            Constraint::Length(1), // blank
            Constraint::Min(0),    // view body
        ])
        .split(content);

    let selected = view.selected_togglable_count();
    let total = view.bundles.len() + view.units.len();
    let header = vec![
        Line::from(vec![
            chrome::section_label("PROJECTS"),
            Span::raw("  "),
            Span::styled(
                format!("{}", selected),
                Style::default()
                    .fg(if selected > 0 {
                        palette::OK
                    } else {
                        palette::WARN
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" / {} selected", total),
                Style::default().fg(palette::MUTED),
            ),
        ]),
        Line::from(Span::styled(
            render_summary(view),
            Style::default().fg(palette::SUBTLE),
        )),
    ];
    frame.render_widget(Paragraph::new(header), body_chunks[0]);

    frame.render_widget(chrome::divider(), body_chunks[2]);

    let ctx = ViewContext {
        mode: RenderMode::Init,
        cursor: Some(cursor),
    };
    view.render_with_overlay(frame, body_chunks[4], &ctx, overlay, ViewLayout::Sectioned);

    chrome::hint_bar(
        frame,
        hints_area,
        &[
            ("↑↓", " navigate"),
            ("Space", " toggle"),
            ("a/n", " all/none"),
            ("c", " cascade-from"),
            ("Enter", " continue"),
            ("Esc", " back"),
            ("q", " quit"),
        ],
    );
}
