//! An open document: its text, its rendered and source views, and where
//! they're scrolled to.
//!
//! In the split view the two sides scroll together. They're matched through
//! source line numbers: each rendered line knows which top-level block (and
//! so which source lines) it came from, and each source view line knows its
//! line number. A position is a fractional source line, so a 1-line table
//! that renders as 10 lines, or a paragraph that wraps to 5, stays lined up.

use crate::highlight;
use crate::render::{RLine, render};
use crate::theme::Theme;
use crate::wrap;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use std::path::Path;
use std::time::SystemTime;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Rendered,
    Source,
}

/// How to lay out the split view.
pub struct Split {
    /// The source side's share of the width, in percent.
    pub ratio: u16,
    /// The side that has the keyboard.
    pub focus: Side,
    pub source_right: bool,
    /// The widest to wrap the rendered side.
    pub max_width: Option<usize>,
}

/// Which side of the split the source goes on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum SourceSide {
    Left,
    Right,
}

/// A top-level block: its source lines and the rendered lines it became.
#[derive(Debug)]
struct Block {
    first: usize,
    last: usize,
    r0: usize,
    r1: usize,
}

/// One line of the source view: part of source line `line` (1-based), the
/// `seg`th of `segs` pieces it wrapped into.
struct SLine {
    spans: Vec<Span<'static>>,
    line: usize,
    seg: usize,
    segs: usize,
}

pub struct Doc {
    md: String,
    /// When the file was last modified, as of loading it.
    pub modified: Option<SystemTime>,

    lines: Vec<RLine>,
    blocks: Vec<Block>,
    /// The width `lines` was wrapped to (0 before the first layout).
    width: usize,
    /// Index of the first visible rendered line.
    top: usize,
    /// Columns scrolled sideways, for code.
    left: usize,
    /// How far `left` can go.
    max_left: usize,

    source: Vec<SLine>,
    source_width: usize,
    source_top: usize,

    /// The side the user last scrolled; the other follows it.
    lead: Side,
    /// Visible lines at the last draw.
    height: usize,
}

impl Doc {
    pub fn new(md: String) -> Doc {
        Doc {
            md,
            modified: None,
            lines: Vec::new(),
            blocks: Vec::new(),
            width: 0,
            top: 0,
            left: 0,
            max_left: 0,
            source: Vec::new(),
            source_width: 0,
            source_top: 0,
            lead: Side::Rendered,
            height: 0,
        }
    }

    /// Reads a file. Errors become the document's text, so they show where
    /// the document would have.
    pub fn load(path: &Path) -> Doc {
        let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        let md = match std::fs::read(path) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(e) => format!("> [!CAUTION]\n> Couldn't read {}: {e}\n", path.display()),
        };
        Doc {
            modified,
            ..Doc::new(md)
        }
    }

    /// Wraps the rendered view for `width`, keeping the same part of the
    /// document at the top.
    fn layout(&mut self, width: usize, theme: &Theme) {
        if width == self.width {
            return;
        }
        let pos = (!self.lines.is_empty()).then(|| self.rendered_pos(self.top));
        self.lines = render(&self.md, width, theme, false);
        self.width = width;
        self.blocks = blocks(&self.lines);
        self.max_left = self
            .lines
            .iter()
            .filter(|l| l.scroll_from.is_some())
            .map(|l| wrap::spans_width(&l.spans).saturating_sub(width))
            .max()
            .unwrap_or(0);
        self.left = self.left.min(self.max_left);
        self.top = pos.map_or(0, |p| self.rendered_top_for(p));
    }

    /// Wraps the source view for `width`, keeping the same line at the top.
    fn layout_source(&mut self, width: usize, theme: &Theme) {
        if width == self.source_width {
            return;
        }
        let pos = (!self.source.is_empty()).then(|| self.source_pos(self.source_top));
        let lines = highlight::highlight(&self.md, "md", theme, Style::default());
        let gutter = digits(lines.len()) + 1;
        let text_w = width.saturating_sub(gutter).max(1);
        self.source.clear();
        for (i, spans) in lines.into_iter().enumerate() {
            let pieces = wrap::wrap_source(spans, text_w);
            let segs = pieces.len();
            for (seg, spans) in pieces.into_iter().enumerate() {
                self.source.push(SLine {
                    spans,
                    line: i + 1,
                    seg,
                    segs,
                });
            }
        }
        self.source_width = width;
        self.source_top = pos.map_or(0, |p| self.source_top_for(p));
    }

    /// The source position at rendered line `i`.
    fn rendered_pos(&self, i: usize) -> f64 {
        if let Some(b) = self.blocks.iter().find(|b| b.r0 <= i && i <= b.r1) {
            let into = (i - b.r0) as f64 / (b.r1 - b.r0 + 1) as f64;
            return b.first as f64 + into * (b.last - b.first + 1) as f64;
        }
        // A blank line between blocks: the line after the previous block.
        self.blocks
            .iter()
            .rev()
            .find(|b| b.r1 < i)
            .map_or(1.0, |b| (b.last + 1) as f64)
    }

    /// The rendered line to put at the top to show source position `pos`.
    fn rendered_top_for(&self, pos: f64) -> usize {
        for b in &self.blocks {
            if pos < b.first as f64 {
                return b.r0;
            }
            if pos < (b.last + 1) as f64 {
                let into = (pos - b.first as f64) / (b.last - b.first + 1) as f64;
                return b.r0 + floor(into * (b.r1 - b.r0 + 1) as f64);
            }
        }
        self.lines.len()
    }

    fn source_pos(&self, i: usize) -> f64 {
        self.source
            .get(i)
            .map_or(1.0, |s| s.line as f64 + s.seg as f64 / s.segs as f64)
    }

    fn source_top_for(&self, pos: f64) -> usize {
        let line = pos.floor() as usize;
        let first = self.source.partition_point(|s| s.line < line);
        match self.source.get(first) {
            Some(s) if s.line == line => first + floor(pos.fract() * s.segs as f64),
            _ => first,
        }
    }

    fn max_top(&self, side: Side) -> usize {
        let len = match side {
            Side::Rendered => self.lines.len(),
            Side::Source => self.source.len(),
        };
        len.saturating_sub(self.height)
    }

    /// Brings the side that isn't leading into line with the one that is.
    fn sync(&mut self, split: bool) {
        match self.lead {
            Side::Rendered if split => {
                self.source_top = self.source_top_for(self.rendered_pos(self.top));
            }
            Side::Source => {
                self.top = self.rendered_top_for(self.source_pos(self.source_top));
            }
            Side::Rendered => {}
        }
        self.top = self.top.min(self.max_top(Side::Rendered));
        self.source_top = self.source_top.min(self.max_top(Side::Source));
    }

    /// Draws the rendered view into `area`, wrapped to `width` (which may be
    /// less than the area's).
    pub fn draw(&mut self, f: &mut Frame, area: Rect, width: usize, theme: &Theme) {
        self.layout(width.max(1), theme);
        self.height = area.height.into();
        self.sync(false);
        self.lead = Side::Rendered;
        self.draw_rendered(f, area);
    }

    /// Draws source and rendered side by side.
    pub fn draw_split(&mut self, f: &mut Frame, area: Rect, split: &Split, theme: &Theme) {
        let source_w = (area.width * split.ratio / 100).clamp(1, area.width.max(1));
        let rest = area.width - source_w;
        // The rendered side gets a column of margin on each side, and a bar
        // separates it from the source.
        let (source_x, bar_x, rendered_x) = if split.source_right {
            (area.x + rest, area.x + rest.saturating_sub(1), area.x + 1)
        } else {
            (area.x, area.x + source_w, area.x + source_w + 2)
        };
        let source_area = Rect { x: source_x, width: source_w, ..area };
        let bar_area = Rect { x: bar_x, width: rest.min(1), ..area };
        let rendered_area = Rect { x: rendered_x, width: rest.saturating_sub(3), ..area };

        let mut width = usize::from(rendered_area.width).max(1);
        if let Some(max) = split.max_width {
            width = width.min(max);
        }
        self.layout(width, theme);
        self.layout_source(source_w.into(), theme);
        self.height = area.height.into();
        self.sync(true);

        self.draw_source(f, source_area, split.focus == Side::Source, theme);
        let bar = vec![Line::from("│".dim()); area.height.into()];
        f.render_widget(Paragraph::new(bar), bar_area);
        self.draw_rendered(f, rendered_area);
    }

    fn draw_rendered(&self, f: &mut Frame, area: Rect) {
        let width = usize::from(area.width);
        let visible: Vec<Line> = self.lines[self.top..]
            .iter()
            .take(self.height)
            .map(|l| Line::from(self.scrolled(l, width)))
            .collect();
        f.render_widget(Paragraph::new(visible), area);
    }

    /// A rendered line as it appears scrolled `left` columns sideways, with
    /// ‹ and › where code runs off either edge.
    fn scrolled(&self, line: &RLine, width: usize) -> Vec<Span<'static>> {
        let Some(from) = line.scroll_from else {
            return line.spans.clone();
        };
        let total = wrap::spans_width(&line.spans);
        if total <= width && self.left == 0 {
            return line.spans.clone();
        }
        let room = width.saturating_sub(from);
        let mut out = wrap::slice(&line.spans, 0, from);
        let clipped_left = self.left > 0;
        let clipped_right = from + self.left + room < total;
        let start = from + self.left + usize::from(clipped_left);
        let len = room.saturating_sub(usize::from(clipped_left) + usize::from(clipped_right));
        let middle = wrap::slice(&line.spans, start, len);
        let edge_style = |spans: &[Span]| spans.first().map_or(Style::new(), |s| s.style).dim();
        if clipped_left {
            let style = edge_style(&wrap::slice(&line.spans, from + self.left, 1));
            out.push(Span::styled("‹", style));
        }
        out.extend(middle);
        if clipped_right {
            let style = edge_style(&wrap::slice(&line.spans, start + len, 1));
            out.push(Span::styled("›", style));
        }
        out
    }

    fn draw_source(&self, f: &mut Frame, area: Rect, focused: bool, theme: &Theme) {
        let gutter = digits(self.source.last().map_or(1, |s| s.line));
        // The block at the top of the rendered side gets its line numbers
        // highlighted, to show what's lined up with what.
        let current = self
            .blocks
            .iter()
            .find(|b| b.r1 >= self.top)
            .map(|b| b.first..=b.last);
        let number_style = |line: usize| {
            if current.as_ref().is_some_and(|r| r.contains(&line)) {
                theme.key()
            } else if focused {
                theme.dim()
            } else {
                Style::new().dim()
            }
        };
        let visible: Vec<Line> = self.source[self.source_top..]
            .iter()
            .take(self.height)
            .map(|s| {
                let number = if s.seg == 0 {
                    format!("{:>gutter$} ", s.line)
                } else {
                    " ".repeat(gutter + 1)
                };
                let mut spans = vec![Span::styled(number, number_style(s.line))];
                spans.extend(s.spans.iter().cloned());
                Line::from(spans)
            })
            .collect();
        f.render_widget(Paragraph::new(visible), area);
    }

    pub fn scroll_by(&mut self, delta: isize, side: Side) {
        self.lead = side;
        let max = self.max_top(side);
        let top = match side {
            Side::Rendered => &mut self.top,
            Side::Source => &mut self.source_top,
        };
        *top = top.saturating_add_signed(delta).min(max);
    }

    pub fn scroll_to_top(&mut self, side: Side) {
        self.scroll_by(isize::MIN / 2, side);
    }

    pub fn scroll_to_bottom(&mut self, side: Side) {
        self.scroll_by(isize::MAX / 2, side);
    }

    pub fn scroll_sideways(&mut self, delta: isize) {
        self.left = self.left.saturating_add_signed(delta).min(self.max_left);
    }

    pub fn page(&self) -> isize {
        self.height.max(1) as isize
    }

    /// "Top", "Bot", "All" or a percentage, like less and vim, for the side
    /// that's leading.
    pub fn position(&self) -> String {
        let (top, len) = match self.lead {
            Side::Rendered => (self.top, self.lines.len()),
            Side::Source => (self.source_top, self.source.len()),
        };
        if len <= self.height {
            "All".into()
        } else if top == 0 {
            "Top".into()
        } else if top >= len - self.height {
            "Bot".into()
        } else {
            format!("{}%", (top + self.height) * 100 / len)
        }
    }
}

/// Groups rendered lines into the top-level blocks they came from.
fn blocks(lines: &[RLine]) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let Some((first, last)) = line.src else { continue };
        match out.last_mut() {
            Some(b) if b.first == first && b.r1 + 1 == i => b.r1 = i,
            _ => out.push(Block { first, last, r0: i, r1: i }),
        }
    }
    out
}

/// Rounds down, but treats 2.9999999 as the 3 it's meant to be.
fn floor(x: f64) -> usize {
    (x + 1e-9) as usize
}

fn digits(n: usize) -> usize {
    n.max(1).to_string().len()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MD: &str = "# Title\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\nOne paragraph that is long enough to wrap across several rendered lines at a narrow width.\n\nlast\n";

    fn laid_out(width: usize) -> Doc {
        let theme = Theme::plain();
        let mut doc = Doc::new(MD.into());
        doc.layout(width, &theme);
        doc.layout_source(width, &theme);
        doc.height = 1;
        doc
    }

    #[test]
    fn maps_rendered_lines_to_source_and_back() {
        let doc = laid_out(20);
        for i in 0..doc.lines.len() {
            let pos = doc.rendered_pos(i);
            let back = doc.rendered_top_for(pos);
            if doc.lines[i].src.is_some() {
                assert_eq!(back, i, "line {i} at {pos}");
            }
        }
        // The table starts on source line 3.
        let table = doc.lines.iter().position(|l| l.src == Some((3, 5))).unwrap();
        assert_eq!(doc.rendered_pos(table), 3.0);
    }

    #[test]
    fn scrolling_one_side_moves_the_other() {
        let mut doc = laid_out(20);
        let para = doc.lines.iter().position(|l| l.src == Some((7, 7))).unwrap();
        doc.top = para;
        doc.lead = Side::Rendered;
        doc.sync(true);
        assert_eq!(doc.source[doc.source_top].line, 7);

        doc.source_top = doc.source.iter().position(|s| s.line == 9).unwrap();
        doc.lead = Side::Source;
        doc.sync(true);
        assert_eq!(doc.lines[doc.top].src, Some((9, 9)));
    }
}
