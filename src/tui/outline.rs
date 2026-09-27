//! The outline pane (`O`): the document's headings beside it, with the
//! section on screen highlighted as you scroll.

use super::App;
use super::picker::Target;
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

impl App<'_> {
    /// How wide the pane is beside a document area of `width` columns.
    pub(super) fn outline_width(width: u16) -> u16 {
        // Constant bounds, so clamp is safe here, unlike with a screen size.
        (width / 4).clamp(22, 40).min(width / 2)
    }

    pub(super) fn draw_outline_pane(&mut self, f: &mut Frame, area: Rect) {
        let block = Block::new()
            .borders(Borders::LEFT)
            .border_style(Style::new().dim())
            .title(" Outline ".dim());
        let inner = block.inner(area);
        f.render_widget(block, area);
        self.outline_area = inner;
        let Some(doc) = self.current() else { return };
        if doc.headings().is_empty() {
            f.render_widget(Paragraph::new(" No headings").dim(), inner);
            return;
        }
        let current = doc.current_heading();
        let items: Vec<ListItem> = doc
            .headings()
            .iter()
            .map(|h| {
                let indent = "  ".repeat(usize::from(h.level.saturating_sub(1)));
                let text = Span::raw(h.text.clone());
                let text = if h.level <= 2 { text.bold() } else { text };
                ListItem::new(Line::from(vec![Span::raw(format!(" {indent}")), text]))
            })
            .collect();
        self.outline_list.select(current);
        let list = List::new(items).highlight_style(Style::new().add_modifier(Modifier::REVERSED));
        f.render_stateful_widget(list, inner, &mut self.outline_list);
    }

    /// A click on the outline pane jumps to that heading. Returns whether
    /// the click was on it.
    pub(super) fn outline_click(&mut self, x: u16, y: u16) -> bool {
        if !self.outline_area.contains(Position { x, y }) {
            return false;
        }
        let i = self.outline_list.offset() + usize::from(y - self.outline_area.y);
        let line = self
            .current()
            .and_then(|d| d.headings().get(i).map(|h| h.line));
        if let Some(line) = line {
            self.go(Target::Line(line));
        }
        true
    }
}
