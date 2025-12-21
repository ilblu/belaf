use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToggleTab {
    #[default]
    Left,
    Right,
}

pub struct TogglePanel {
    active: ToggleTab,
    click_area: Option<Rect>,
    left_label: &'static str,
    right_label: &'static str,
    left_active_color: Color,
    right_active_color: Color,
}

impl Default for TogglePanel {
    fn default() -> Self {
        Self::new("Preview", "Source")
    }
}

impl TogglePanel {
    pub fn new(left_label: &'static str, right_label: &'static str) -> Self {
        Self {
            active: ToggleTab::Left,
            click_area: None,
            left_label,
            right_label,
            left_active_color: Color::Cyan,
            right_active_color: Color::Magenta,
        }
    }

    pub fn with_colors(mut self, left_color: Color, right_color: Color) -> Self {
        self.left_active_color = left_color;
        self.right_active_color = right_color;
        self
    }

    pub fn toggle(&mut self) {
        self.active = match self.active {
            ToggleTab::Left => ToggleTab::Right,
            ToggleTab::Right => ToggleTab::Left,
        };
    }

    pub fn active(&self) -> ToggleTab {
        self.active
    }

    pub fn is_left(&self) -> bool {
        self.active == ToggleTab::Left
    }

    pub fn is_right(&self) -> bool {
        self.active == ToggleTab::Right
    }

    pub fn set_active(&mut self, tab: ToggleTab) {
        self.active = tab;
    }

    pub fn handle_click(&mut self, x: u16, y: u16) -> bool {
        if let Some(area) = self.click_area {
            if x >= area.x && x < area.x + area.width && y >= area.y && y < area.y + area.height {
                self.toggle();
                return true;
            }
        }
        false
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect, title: &str) {
        self.click_area = Some(area);

        let left_style = if self.is_left() {
            Style::default()
                .fg(Color::Black)
                .bg(self.left_active_color)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };

        let right_style = if self.is_right() {
            Style::default()
                .fg(Color::Black)
                .bg(self.right_active_color)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };

        let toggle_line = Line::from(vec![
            Span::raw("  "),
            Span::styled(format!(" ◉ {} ", self.left_label), left_style),
            Span::raw("  "),
            Span::styled(format!(" ◉ {} ", self.right_label), right_style),
            Span::raw("  "),
            Span::styled("(m)", Style::default().fg(Color::DarkGray)),
        ]);

        let widget = Paragraph::new(toggle_line)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" {} ", title)),
            )
            .alignment(ratatui::layout::Alignment::Center);

        frame.render_widget(widget, area);
    }
}
