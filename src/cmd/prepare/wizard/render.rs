//! All Ratatui rendering for the prepare wizard.
//!
//! Lives in a child module so wizard.rs stays focused on state,
//! navigation, and key handling. Private items in `super` (WizardState,
//! WizardStep, ReleaseUnitItem, DisplayRow, the `calculate_*_version`
//! helpers) are visible here because Rust grants child modules access to
//! the parent's private items.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, Padding, Paragraph, Wrap},
    Frame,
};

use crate::core::{
    bump::BumpRecommendation,
    changelog::Commit,
    ui::{
        markdown,
        release_unit_view::{
            BumpHint, ReleaseUnitView, RenderMode, ResolvedEntry, ViewContext, ViewLayout,
        },
        utils::centered_rect,
    },
    wire::known::Ecosystem,
    workflow::BumpChoice,
};

use super::{
    calculate_major_version, calculate_minor_version, calculate_next_version,
    calculate_patch_version, WizardState, WizardStep,
};

pub(super) fn ui(f: &mut Frame, state: &mut WizardState) {
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

    let entries: Vec<ResolvedEntry> = state
        .units
        .iter()
        .map(|u| ResolvedEntry {
            name: u.name().to_string(),
            version: u.current_version().to_string(),
            ecosystem: Some(u.ecosystem().as_str().to_string()),
            selected: u.selected,
            group_id: u.group_id().map(str::to_string),
            commit_count: u.commit_count(),
            bump_hint: bump_hint_from(u.suggested_bump()),
        })
        .collect();
    let (view, overlay) = ReleaseUnitView::from_resolved(&entries);

    let cursor = state.unit_list_state.selected();
    let ctx = ViewContext {
        mode: RenderMode::Prepare,
        cursor,
    };

    let list_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Gray))
        .title(Span::styled(
            " Projects ",
            Style::default().fg(Color::White),
        ));
    let inner = list_block.inner(chunks[1]);
    f.render_widget(list_block, chunks[1]);
    view.render_with_overlay(f, inner, &ctx, &overlay, ViewLayout::Grouped);

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

fn bump_hint_from(rec: BumpRecommendation) -> BumpHint {
    match rec {
        BumpRecommendation::Major => BumpHint::Major,
        BumpRecommendation::Minor => BumpHint::Minor,
        BumpRecommendation::Patch => BumpHint::Patch,
        BumpRecommendation::None => BumpHint::None,
    }
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

    let (feat_count, fix_count, revert_count, breaking_count, other_count) =
        count_commit_types(commits);

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
    if revert_count > 0 {
        lines.push(Line::from(vec![
            Span::styled("  revert:    ", Style::default().fg(Color::Yellow)),
            Span::styled(format!("{}", revert_count), Style::default().fg(Color::Yellow)),
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

fn count_commit_types(commits: &[Commit]) -> (usize, usize, usize, usize, usize) {
    let mut feat = 0;
    let mut fix = 0;
    let mut revert = 0;
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
        } else if lower.starts_with("revert") || msg.starts_with("Revert \"") {
            // Both conventional `revert:` and git's auto-generated
            // `Revert "<subject>"` count here — they both drive a
            // patch bump in `core::bump::analyze_commits`.
            revert += 1;
        } else {
            other += 1;
        }
    }

    (feat, fix, revert, breaking, other)
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
