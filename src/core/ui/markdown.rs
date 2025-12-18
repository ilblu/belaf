use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};

pub fn render_markdown(input: &str) -> Text<'static> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    let parser = Parser::new_ext(input, options);

    let mut renderer = MarkdownRenderer::new();
    renderer.process(parser);
    renderer.into_text()
}

struct MarkdownRenderer {
    lines: Vec<Line<'static>>,
    current_line: Vec<Span<'static>>,
    style_stack: Vec<Style>,
    list_level: usize,
    list_item_number: Vec<Option<u64>>,
    in_code_block: bool,
}

impl MarkdownRenderer {
    fn new() -> Self {
        Self {
            lines: Vec::new(),
            current_line: Vec::new(),
            style_stack: vec![Style::default()],
            list_level: 0,
            list_item_number: Vec::new(),
            in_code_block: false,
        }
    }

    fn process(&mut self, parser: Parser) {
        for event in parser {
            self.handle_event(event);
        }
        self.flush_line();
    }

    fn handle_event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start_tag(tag),
            Event::End(tag) => self.end_tag(tag),
            Event::Text(text) => self.add_text(&text),
            Event::Code(code) => self.add_code(&code),
            Event::SoftBreak | Event::HardBreak => self.flush_line(),
            Event::TaskListMarker(checked) => {
                let marker = if checked { "[✓] " } else { "[ ] " };
                self.current_line.push(Span::styled(
                    marker.to_string(),
                    Style::default().fg(if checked { Color::Green } else { Color::Gray }),
                ));
            }
            _ => {}
        }
    }

    fn start_tag(&mut self, tag: Tag) {
        match tag {
            Tag::Heading { level, .. } => {
                self.flush_line();
                let style = match level {
                    HeadingLevel::H1 => Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                    HeadingLevel::H2 => Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                    HeadingLevel::H3 => Style::default()
                        .fg(Color::Blue)
                        .add_modifier(Modifier::BOLD),
                    _ => Style::default()
                        .fg(Color::LightBlue)
                        .add_modifier(Modifier::BOLD),
                };
                self.style_stack.push(style);
            }
            Tag::Paragraph => {
                if !self.lines.is_empty() && !self.current_line.is_empty() {
                    self.flush_line();
                }
            }
            Tag::List(start_number) => {
                self.list_level += 1;
                self.list_item_number.push(start_number);
                if self.list_level == 1 && !self.lines.is_empty() {
                    self.lines.push(Line::default());
                }
            }
            Tag::Item => {
                self.flush_line();
                let indent = "  ".repeat(self.list_level.saturating_sub(1));
                self.current_line.push(Span::raw(indent));

                if let Some(Some(num)) = self.list_item_number.last_mut() {
                    *num += 1;
                    self.current_line.push(Span::styled(
                        format!("{}. ", *num - 1),
                        Style::default().fg(Color::LightBlue),
                    ));
                } else {
                    self.current_line.push(Span::styled(
                        "• ".to_string(),
                        Style::default().fg(Color::Yellow),
                    ));
                }
            }
            Tag::CodeBlock(_) => {
                self.flush_line();
                self.in_code_block = true;
                self.style_stack
                    .push(Style::default().fg(Color::White).bg(Color::DarkGray));
            }
            Tag::BlockQuote(_) => {
                self.style_stack.push(
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::ITALIC),
                );
            }
            Tag::Emphasis => {
                let current = *self.style_stack.last().unwrap();
                self.style_stack
                    .push(current.add_modifier(Modifier::ITALIC));
            }
            Tag::Strong => {
                let current = *self.style_stack.last().unwrap();
                self.style_stack.push(current.add_modifier(Modifier::BOLD));
            }
            Tag::Strikethrough => {
                let current = *self.style_stack.last().unwrap();
                self.style_stack
                    .push(current.add_modifier(Modifier::CROSSED_OUT));
            }
            _ => {}
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Heading(_) => {
                self.style_stack.pop();
                self.flush_line();
                self.lines.push(Line::default());
            }
            TagEnd::Paragraph => {
                self.flush_line();
            }
            TagEnd::List(_) => {
                self.list_level = self.list_level.saturating_sub(1);
                self.list_item_number.pop();
                if self.list_level == 0 {
                    self.lines.push(Line::default());
                }
            }
            TagEnd::CodeBlock => {
                self.in_code_block = false;
                self.style_stack.pop();
                self.flush_line();
                self.lines.push(Line::default());
            }
            TagEnd::BlockQuote(_) => {
                self.style_stack.pop();
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.style_stack.pop();
            }
            _ => {}
        }
    }

    fn add_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }

        let style = *self.style_stack.last().unwrap();

        if self.in_code_block {
            for line in text.lines() {
                if !self.current_line.is_empty() {
                    self.flush_line();
                }
                self.current_line.push(Span::raw("  "));
                self.current_line
                    .push(Span::styled(line.to_string(), style));
                self.flush_line();
            }
        } else {
            self.current_line
                .push(Span::styled(text.to_string(), style));
        }
    }

    fn add_code(&mut self, code: &str) {
        self.current_line.push(Span::styled(
            code.to_string(),
            Style::default()
                .fg(Color::Rgb(255, 182, 193))
                .bg(Color::Rgb(40, 40, 40)),
        ));
    }

    fn flush_line(&mut self) {
        if !self.current_line.is_empty() || self.in_code_block {
            let line = Line::from(std::mem::take(&mut self.current_line));
            self.lines.push(line);
        }
    }

    fn into_text(mut self) -> Text<'static> {
        self.flush_line();
        Text::from(self.lines)
    }
}
