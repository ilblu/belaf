use ratatui::{
    layout::{Alignment, Rect},
    style::Style,
    widgets::Paragraph,
    Frame,
};

pub(crate) struct StatusBar<'a> {
    left_text: &'a str,
    center_text: &'a str,
    right_text: &'a str,
    style: Style,
}

impl<'a> StatusBar<'a> {
    pub fn new(left_text: &'a str, center_text: &'a str, right_text: &'a str) -> Self {
        Self {
            left_text,
            center_text,
            right_text,
            style: Style::default(),
        }
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let left = Paragraph::new(self.left_text)
            .style(self.style)
            .alignment(Alignment::Left);

        let center = Paragraph::new(self.center_text)
            .style(self.style)
            .alignment(Alignment::Center);

        let right = Paragraph::new(self.right_text)
            .style(self.style)
            .alignment(Alignment::Right);

        frame.render_widget(left, area);
        frame.render_widget(center, area);
        frame.render_widget(right, area);
    }
}
