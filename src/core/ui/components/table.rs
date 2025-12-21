use ratatui::{
    layout::Rect,
    widgets::{Block, Row, Table as RatatuiTable},
    Frame,
};

pub(crate) struct Table<'a> {
    rows: Vec<Row<'a>>,
    header: Option<Row<'a>>,
    widths: &'a [ratatui::layout::Constraint],
    block: Option<Block<'a>>,
}

impl<'a> Table<'a> {
    pub fn new(rows: Vec<Row<'a>>, widths: &'a [ratatui::layout::Constraint]) -> Self {
        Self {
            rows,
            header: None,
            widths,
            block: None,
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

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let mut table = RatatuiTable::new(self.rows.clone(), self.widths);

        if let Some(header) = &self.header {
            table = table.header(header.clone());
        }

        if let Some(block) = self.block.clone() {
            table = table.block(block);
        }

        frame.render_widget(table, area);
    }
}
