//! An open document: its text, its rendered and source views, and where
//! they're scrolled to.
//!
//! In the split view the two sides scroll together. They're matched through
//! source line numbers: each rendered line knows which top-level block (and
//! so which source lines) it came from, and each source view line knows its
//! line number. A position is a fractional source line, so a 1-line table
//! that renders as 10 lines, or a paragraph that wraps to 5, stays lined up.

use crate::highlight;
use crate::render::{CodeBlock, Heading, RLine, render};
use crate::theme::Theme;
use crate::wrap;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use std::path::{Path, PathBuf};
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
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

/// A search through the rendered text.
struct Search {
    query: String,
    /// Every match: rendered line and columns.
    matches: Vec<(usize, usize, usize)>,
    /// The match last jumped to.
    current: Option<usize>,
}

/// A label typed to follow the link it's drawn on.
pub struct Hint {
    pub label: String,
    line: usize,
    col: usize,
    /// The link's target.
    pub url: String,
}

pub struct Doc {
    md: String,
    /// The directory relative links are relative to.
    pub base: Option<PathBuf>,
    headings: Vec<Heading>,
    links: Vec<String>,
    code_blocks: Vec<CodeBlock>,
    search: Option<Search>,
    /// Link hints on screen, while choosing a link to follow.
    pub hints: Vec<Hint>,
    /// A heading to jump to once the document is laid out.
    pending_anchor: Option<String>,
    /// A search match to jump to once the document is laid out: its source
    /// line and the query.
    pending_match: Option<(usize, String)>,
    /// After a reload: the first line with text at or below the top of the
    /// screen, how far below the top it was, and its old index, to find
    /// the same place in the new text.
    keep: Option<(String, usize, usize)>,
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
    /// Where each side was drawn last, for the mouse. The source's is
    /// empty when it isn't shown.
    rendered_area: Rect,
    source_area: Rect,
}

impl Doc {
    pub fn new(md: String) -> Doc {
        Doc {
            md,
            base: std::env::current_dir().ok(),
            headings: Vec::new(),
            links: Vec::new(),
            code_blocks: Vec::new(),
            search: None,
            hints: Vec::new(),
            pending_anchor: None,
            pending_match: None,
            keep: None,
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
            rendered_area: Rect::default(),
            source_area: Rect::default(),
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
            base: path.parent().map(Path::to_path_buf),
            ..Doc::new(md)
        }
    }

    /// Reads the file again after it's changed. The next draw re-renders
    /// it, keeping the same part of the document on screen.
    pub fn reload(&mut self, path: &Path) {
        let fresh = Doc::load(path);
        self.keep = self.lines[self.top.min(self.lines.len())..]
            .iter()
            .enumerate()
            .map(|(i, l)| {
                (
                    l.spans
                        .iter()
                        .map(|s| s.content.as_ref())
                        .collect::<String>(),
                    i,
                )
            })
            .find(|(text, _)| !text.trim().is_empty())
            .map(|(text, offset)| (text, offset, self.top + offset));
        self.md = fresh.md;
        self.modified = fresh.modified;
        self.width = 0;
        self.source_width = 0;
    }

    /// Wraps the rendered view for `width`, keeping the same part of the
    /// document at the top.
    fn layout(&mut self, width: usize, theme: &Theme) {
        if width == self.width {
            return;
        }
        let pos = (!self.lines.is_empty()).then(|| self.rendered_pos(self.top));
        let rendered = render(&self.md, width, theme, false, self.base.as_deref());
        self.lines = rendered.lines;
        self.headings = rendered.headings;
        self.links = rendered.links;
        self.code_blocks = rendered.code_blocks;
        self.width = width;
        if let Some(query) = self.search.as_ref().map(|s| s.query.clone()) {
            self.find(&query);
        }
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
        if let Some((text, offset, old)) = self.keep.take() {
            // The same text nearest where it was, if it's still there.
            let same = self.lines.iter().enumerate().filter(|(_, l)| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
                    == text
            });
            if let Some((i, _)) = same.min_by_key(|(i, _)| i.abs_diff(old)) {
                self.top = i.saturating_sub(offset);
            }
        }
        if let Some(slug) = self.pending_anchor.take() {
            self.go_to_anchor(&slug);
        }
        if let Some((line, query)) = self.pending_match.take() {
            self.go_to_match(line, &query);
        }
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
        self.source_area = Rect::default();
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
        let source_area = Rect {
            x: source_x,
            width: source_w,
            ..area
        };
        let bar_area = Rect {
            x: bar_x,
            width: rest.min(1),
            ..area
        };
        let rendered_area = Rect {
            x: rendered_x,
            width: rest.saturating_sub(3),
            ..area
        };

        let mut width = usize::from(rendered_area.width).max(1);
        if let Some(max) = split.max_width {
            width = width.min(max);
        }
        self.layout(width, theme);
        self.layout_source(source_w.into(), theme);
        self.height = area.height.into();
        self.sync(true);

        self.source_area = source_area;
        self.draw_source(f, source_area, split.focus == Side::Source, theme);
        let bar = vec![Line::from("│".dim()); area.height.into()];
        f.render_widget(Paragraph::new(bar), bar_area);
        self.draw_rendered(f, rendered_area);
    }

    fn draw_rendered(&mut self, f: &mut Frame, area: Rect) {
        self.rendered_area = area;
        let width = usize::from(area.width);
        let visible: Vec<Line> = self.lines[self.top..]
            .iter()
            .take(self.height)
            .enumerate()
            .map(|(i, l)| {
                let line = self.highlighted(self.top + i, l);
                Line::from(self.scrolled(&line, width))
            })
            .collect();
        f.render_widget(Paragraph::new(visible), area);

        let style = Style::new().black().on_yellow().bold();
        for hint in &self.hints {
            let Some(row) = hint.line.checked_sub(self.top).filter(|&r| r < self.height) else {
                continue;
            };
            if hint.col < width {
                let x = area.x + hint.col as u16;
                f.buffer_mut().set_stringn(
                    x,
                    area.y + row as u16,
                    &hint.label,
                    width - hint.col,
                    style,
                );
            }
        }
    }

    /// Line `i` with any search matches on it highlighted.
    fn highlighted(&self, i: usize, line: &RLine) -> RLine {
        let Some(search) = &self.search else {
            return line.clone();
        };
        let first = search.matches.partition_point(|m| m.0 < i);
        let mut line = line.clone();
        for (n, &(_, start, end)) in search.matches[first..]
            .iter()
            .take_while(|m| m.0 == i)
            .enumerate()
        {
            let style = if search.current == Some(first + n) {
                Style::new().black().on_yellow()
            } else {
                Style::new().reversed()
            };
            line.spans = wrap::restyle(line.spans, start, end, style);
        }
        line
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

    /// The side of the document at screen position (x, y), if any.
    pub fn side_at(&self, x: u16, y: u16) -> Option<Side> {
        let pos = ratatui::layout::Position { x, y };
        if self.rendered_area.contains(pos) {
            Some(Side::Rendered)
        } else if self.source_area.contains(pos) {
            Some(Side::Source)
        } else {
            None
        }
    }

    /// The target of the link at screen position (x, y), if there's one.
    pub fn link_at(&self, x: u16, y: u16) -> Option<String> {
        if self.side_at(x, y) != Some(Side::Rendered) {
            return None;
        }
        let area = self.rendered_area;
        let line = self.lines.get(self.top + usize::from(y - area.y))?;
        let col = usize::from(x - area.x);
        let link = line.links.iter().find(|l| l.start <= col && col < l.end)?;
        self.links.get(link.id as usize).cloned()
    }

    /// The first code block on screen.
    pub fn code_on_screen(&self) -> Option<&CodeBlock> {
        let screen = self.top..self.top + self.height;
        self.code_blocks
            .iter()
            .find(|b| b.lines.start < screen.end && screen.start < b.lines.end)
    }

    /// The source line (1-based) at the top of the screen, on whichever
    /// side is leading.
    pub fn source_line(&self) -> usize {
        match self.lead {
            Side::Source => self.source.get(self.source_top).map_or(1, |s| s.line),
            Side::Rendered => self.rendered_pos(self.top) as usize,
        }
        .max(1)
    }

    /// Every link target in the document, in order, as written.
    pub fn links(&self) -> &[String] {
        &self.links
    }

    pub fn headings(&self) -> &[Heading] {
        &self.headings
    }

    /// Scrolls the rendered side so line `i` is at the top.
    pub fn jump_to(&mut self, i: usize) {
        self.lead = Side::Rendered;
        self.top = i.min(self.max_top(Side::Rendered));
    }

    /// Jumps to the next heading below the top of the screen, or the
    /// previous one above it. Returns false if there isn't one.
    pub fn jump_heading(&mut self, forward: bool) -> bool {
        let top = self.top;
        let line = if forward {
            self.headings.iter().map(|h| h.line).find(|&l| l > top)
        } else {
            self.headings
                .iter()
                .map(|h| h.line)
                .rev()
                .find(|&l| l < top)
        };
        line.map(|l| self.jump_to(l)).is_some()
    }

    /// The line of the heading with anchor `slug`.
    pub fn anchor(&self, slug: &str) -> Option<usize> {
        let slug = slug.to_lowercase();
        self.headings
            .iter()
            .find(|h| h.slug == slug)
            .map(|h| h.line)
    }

    /// Jumps to the heading with anchor `slug`, now or, if the document
    /// hasn't been laid out yet, as soon as it is.
    pub fn go_to_anchor(&mut self, slug: &str) {
        if self.width == 0 {
            self.pending_anchor = Some(slug.to_string());
        } else if let Some(line) = self.anchor(slug) {
            self.jump_to(line);
        }
    }

    /// Searches for `query` from source line `line`, highlighting its
    /// matches and jumping to the first there, now or once laid out.
    pub fn go_to_match(&mut self, line: usize, query: &str) {
        if self.width == 0 {
            self.pending_match = Some((line, query.to_string()));
            return;
        }
        let from = self.rendered_top_for(line as f64);
        self.search(query, from);
    }

    pub fn top(&self) -> usize {
        self.top
    }

    /// Searches the rendered text for `query` and jumps to the first match
    /// at or after line `from`. Smart case: case matters only if the query
    /// has capitals. Returns false if nothing matches.
    pub fn search(&mut self, query: &str, from: usize) -> bool {
        if query.is_empty() {
            self.search = None;
            return true;
        }
        self.find(query);
        let search = self.search.as_mut().unwrap();
        let next = search
            .matches
            .iter()
            .position(|m| m.0 >= from)
            .or((!search.matches.is_empty()).then_some(0));
        search.current = next;
        if let Some(i) = next {
            let line = search.matches[i].0;
            self.reveal(line);
        }
        next.is_some()
    }

    /// Moves to the next (or previous) match, wrapping around.
    pub fn search_next(&mut self, forward: bool) -> bool {
        let Some(search) = &mut self.search else {
            return false;
        };
        let n = search.matches.len();
        if n == 0 {
            return false;
        }
        let i = match search.current {
            Some(i) if forward => (i + 1) % n,
            Some(i) => (i + n - 1) % n,
            None => 0,
        };
        search.current = Some(i);
        let line = search.matches[i].0;
        self.reveal(line);
        true
    }

    pub fn clear_search(&mut self) {
        self.search = None;
    }

    /// "3/17" for the current match, or "0/0".
    pub fn search_status(&self) -> Option<String> {
        let s = self.search.as_ref()?;
        let current = s.current.map_or(0, |i| i + 1);
        Some(format!("{current}/{}", s.matches.len()))
    }

    /// Scrolls so line `i` is visible, a third of the way down if it wasn't.
    fn reveal(&mut self, i: usize) {
        self.lead = Side::Rendered;
        if i < self.top || i >= self.top + self.height {
            self.top = i
                .saturating_sub(self.height / 3)
                .min(self.max_top(Side::Rendered));
        }
    }

    fn find(&mut self, query: &str) {
        let case = query.chars().any(char::is_uppercase);
        let fold = |c: char| {
            if case {
                c
            } else {
                c.to_lowercase().next().unwrap_or(c)
            }
        };
        let needle: Vec<char> = query.chars().map(fold).collect();
        let mut matches = Vec::new();
        for (i, line) in self.lines.iter().enumerate() {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            let chars: Vec<char> = text.chars().collect();
            let mut col = 0;
            let mut cols = Vec::with_capacity(chars.len() + 1);
            for &c in &chars {
                cols.push(col);
                col += wrap::width(c.encode_utf8(&mut [0; 4]));
            }
            cols.push(col);
            let mut at = 0;
            while at + needle.len() <= chars.len() {
                if chars[at..at + needle.len()]
                    .iter()
                    .map(|&c| fold(c))
                    .eq(needle.iter().copied())
                {
                    matches.push((i, cols[at], cols[at + needle.len()]));
                    at += needle.len().max(1);
                } else {
                    at += 1;
                }
            }
        }
        let current = self.search.as_ref().and_then(|s| s.current);
        self.search = Some(Search {
            query: query.to_string(),
            current: current.filter(|&c| c < matches.len()),
            matches,
        });
    }

    /// The links on screen in the rendered view, top to bottom.
    pub fn visible_links(&self) -> Vec<(usize, usize, String)> {
        let mut out = Vec::new();
        for (i, line) in self
            .lines
            .iter()
            .enumerate()
            .skip(self.top)
            .take(self.height)
        {
            for l in &line.links {
                out.push((i, l.start, self.links[l.id as usize].clone()));
            }
        }
        out
    }

    pub fn set_hints(&mut self, hints: Vec<(String, usize, usize, String)>) {
        self.hints = hints
            .into_iter()
            .map(|(label, line, col, url)| Hint {
                label,
                line,
                col,
                url,
            })
            .collect();
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
        let Some((first, last)) = line.src else {
            continue;
        };
        match out.last_mut() {
            Some(b) if b.first == first && b.r1 + 1 == i => b.r1 = i,
            _ => out.push(Block {
                first,
                last,
                r0: i,
                r1: i,
            }),
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
        let table = doc
            .lines
            .iter()
            .position(|l| l.src == Some((3, 5)))
            .unwrap();
        assert_eq!(doc.rendered_pos(table), 3.0);
    }

    #[test]
    fn scrolling_one_side_moves_the_other() {
        let mut doc = laid_out(20);
        let para = doc
            .lines
            .iter()
            .position(|l| l.src == Some((7, 7)))
            .unwrap();
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
