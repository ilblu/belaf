//! Shared visual chrome for every init-wizard step.
//!
//! Centralises the outer block, progress dots in the title bar,
//! section labels, dividers, hint bar and the colour palette so
//! every step renders with the same visual language.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

/// How many top-level steps the user goes through in the standard
/// multi-project flow. Sub-steps (cascade-from, tag-format) share the
/// dot of the parent step that triggered them.
pub const STEP_TOTAL: u8 = 5;

pub mod step_index {
    pub const WELCOME: u8 = 1;
    pub const SELECTION: u8 = 2;
    pub const PRESET: u8 = 3;
    pub const UPSTREAM: u8 = 4;
    pub const CONFIRMATION: u8 = 5;
}

pub mod palette {
    use ratatui::style::Color;

    pub const ACCENT: Color = Color::Cyan;
    pub const TITLE: Color = Color::Cyan;
    pub const SECTION: Color = Color::Rgb(140, 140, 160);
    pub const DIVIDER: Color = Color::Rgb(50, 50, 60);
    pub const HINT: Color = Color::Rgb(120, 120, 140);
    pub const HINT_SEP: Color = Color::Rgb(60, 60, 75);
    pub const VALUE: Color = Color::White;
    pub const SUBTLE: Color = Color::Rgb(150, 150, 160);
    pub const MUTED: Color = Color::Rgb(95, 95, 110);
    pub const ACTION: Color = Color::Rgb(80, 200, 140);
    pub const PLACEHOLDER: Color = Color::Rgb(80, 80, 95);
    pub const DOT_DONE: Color = Color::Cyan;
    pub const DOT_FUTURE: Color = Color::Rgb(60, 60, 75);
    pub const ROW_HIGHLIGHT: Color = Color::Rgb(28, 28, 36);
    pub const WARN: Color = Color::Rgb(220, 160, 70);
    pub const ERROR: Color = Color::Rgb(220, 90, 90);
    pub const OK: Color = Color::Rgb(80, 200, 140);
}

/// Draws the outer bordered chrome with the title and progress dots,
/// returns the inner body rect (already padded so steps don't have to
/// re-add margins). Pass `dot_index = 0` to suppress the progress
/// indicator (used by stand-alone screens like Single-Mobile).
pub fn render_chrome(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    dot_index: u8,
    dot_total: u8,
) -> Rect {
    let title_line = build_title(title, dot_index, dot_total);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette::ACCENT))
        .title(title_line);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    Layout::default()
        .direction(Direction::Vertical)
        .horizontal_margin(2)
        .vertical_margin(1)
        .constraints([Constraint::Min(0)])
        .split(inner)[0]
}

/// Splits the chromed body into a content area + a single hint line
/// at the bottom. Use this when the step has a hint bar.
pub fn split_body_with_hints(body: Rect) -> (Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(body);
    (chunks[0], chunks[1])
}

fn build_title(title: &str, dot_index: u8, dot_total: u8) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = vec![
        Span::raw(" "),
        Span::styled(
            title.to_string(),
            Style::default()
                .fg(palette::TITLE)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ];

    if dot_index > 0 && dot_total > 0 {
        spans.push(Span::styled("  ", Style::default().fg(palette::DIVIDER)));
        for i in 1..=dot_total {
            let (glyph, style) = if i < dot_index {
                ("●", Style::default().fg(palette::DOT_DONE))
            } else if i == dot_index {
                (
                    "●",
                    Style::default()
                        .fg(palette::DOT_DONE)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                ("○", Style::default().fg(palette::DOT_FUTURE))
            };
            spans.push(Span::styled(glyph, style));
            if i < dot_total {
                spans.push(Span::raw(" "));
            }
        }
        spans.push(Span::raw(" "));
    }

    Line::from(spans)
}

/// `REPOSITORY`-style section label: small caps, bold, muted.
pub fn section_label(text: &str) -> Span<'static> {
    Span::styled(
        text.to_string(),
        Style::default()
            .fg(palette::SECTION)
            .add_modifier(Modifier::BOLD),
    )
}

/// Thin horizontal rule used between sections inside the body.
/// Width is hard-padded; the paragraph clips to the cell rect.
pub fn divider() -> Paragraph<'static> {
    Paragraph::new(Line::from(Span::styled(
        "─".repeat(400),
        Style::default().fg(palette::DIVIDER),
    )))
}

/// One key hint pair: `Enter` + ` continue`. Combined into a single
/// span so the hint bar code can interleave separators cleanly.
pub fn key_hint(key: &str, label: &str) -> Span<'static> {
    Span::styled(
        format!("{}{}", key, label),
        Style::default().fg(palette::HINT),
    )
}

pub fn sep() -> Span<'static> {
    Span::styled("  ·  ", Style::default().fg(palette::HINT_SEP))
}

/// Renders a centred hint bar from a list of (key, label) pairs.
/// Empty list → nothing rendered. Caller passes the bottom strip rect
/// from [`split_body_with_hints`].
pub fn hint_bar(frame: &mut Frame, area: Rect, items: &[(&str, &str)]) {
    if items.is_empty() {
        return;
    }
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(items.len() * 2);
    for (i, (key, label)) in items.iter().enumerate() {
        if i > 0 {
            spans.push(sep());
        }
        spans.push(key_hint(key, label));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).alignment(Alignment::Center),
        area,
    );
}

/// Action bullet used in confirmation-style summaries.
pub fn action_row(text: &str) -> Vec<Span<'static>> {
    vec![
        Span::raw("  "),
        Span::styled(
            "▸ ",
            Style::default()
                .fg(palette::ACTION)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(text.to_string(), Style::default().fg(palette::VALUE)),
    ]
}

/// Brand-coloured ecosystem labels so the same name reads with the
/// same colour wherever it appears in the wizard.
pub fn ecosystem_color(eco: &str) -> Color {
    match eco {
        "cargo" => Color::Rgb(200, 110, 60),
        "npm" => Color::Rgb(220, 80, 80),
        "pypa" | "pypi" => Color::Rgb(80, 160, 220),
        "go" => Color::Rgb(0, 173, 216),
        "maven" => Color::Rgb(220, 140, 60),
        "elixir" => Color::Rgb(180, 100, 200),
        "csproj" | "csharp" => Color::Rgb(120, 100, 200),
        "swift" => Color::Rgb(240, 130, 80),
        _ => palette::SECTION,
    }
}
