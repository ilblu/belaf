use ratatui::{
    layout::Rect,
    style::Style,
    widgets::{Block, Row, Table as RatatuiTable},
    Frame,
};

pub(crate) struct Table<'a> {
    rows: Vec<Row<'a>>,
    header: Option<Row<'a>>,
    widths: &'a [ratatui::layout::Constraint],
    block: Option<Block<'a>>,
    highlight_style: Option<Style>,
}

impl<'a> Table<'a> {
    pub fn new(rows: Vec<Row<'a>>, widths: &'a [ratatui::layout::Constraint]) -> Self {
        Self {
            rows,
            header: None,
            widths,
            block: None,
            highlight_style: None,
        }
    }

    pub fn header(mut self, header: Row<'a>) -> Self {
        self.header = Some(header);
        self
    }

    pub fn block(mut self, block: Block<'a>) -> Self {
        self.block = Some(block);
        self
    }

    pub fn highlight_style(mut self, style: Style) -> Self {
        self.highlight_style = Some(style);
        self
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let mut table = RatatuiTable::new(self.rows.clone(), self.widths);

        if let Some(header) = &self.header {
            table = table.header(header.clone());
        }

        if let Some(block) = self.block.clone() {
            table = table.block(block);
        }

        if let Some(style) = self.highlight_style {
            table = table.row_highlight_style(style);
        }

        frame.render_widget(table, area);
    }
}
