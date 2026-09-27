//! Word wrapping for styled text.

use ratatui::style::Style;
use ratatui::text::Span;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// A run of inline text, before wrapping.
#[derive(Clone, Debug)]
pub enum Piece {
    Text(String, Style),
    /// A hard line break.
    Break,
    /// The text up to the next `LinkEnd` is link number `id`.
    LinkStart(u32),
    LinkEnd,
}

/// Where a link landed on a wrapped line: columns `start..end`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkSpan {
    pub start: usize,
    pub end: usize,
    pub id: u32,
}

/// A wrapped line, with the links on it.
#[derive(Default)]
pub struct WLine {
    pub spans: Vec<Span<'static>>,
    pub links: Vec<LinkSpan>,
}

/// Display width of `s` in terminal columns.
pub fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Total display width of a line of spans.
pub fn spans_width(spans: &[Span]) -> usize {
    spans.iter().map(|s| width(&s.content)).sum()
}

struct Part {
    text: String,
    style: Style,
    link: Option<u32>,
}

enum Unit {
    /// Consecutive non-space text, possibly in several styles (`**foo**bar`).
    Word(Vec<Part>),
    Space(Style, Option<u32>),
    Break,
}

fn units(pieces: &[Piece]) -> Vec<Unit> {
    let mut units = Vec::new();
    let mut word: Vec<Part> = Vec::new();
    let mut link = None;
    for piece in pieces {
        match piece {
            Piece::LinkStart(id) => link = Some(*id),
            Piece::LinkEnd => link = None,
            Piece::Break => {
                if !word.is_empty() {
                    units.push(Unit::Word(std::mem::take(&mut word)));
                }
                units.push(Unit::Break);
            }
            Piece::Text(text, style) => {
                let style = *style;
                let mut run = String::new();
                for c in text.chars() {
                    if c.is_whitespace() && c != '\u{a0}' {
                        if !run.is_empty() {
                            word.push(Part {
                                text: std::mem::take(&mut run),
                                style,
                                link,
                            });
                        }
                        if !word.is_empty() {
                            units.push(Unit::Word(std::mem::take(&mut word)));
                        }
                        if !matches!(units.last(), Some(Unit::Space(..))) {
                            units.push(Unit::Space(style, link));
                        }
                    } else {
                        run.push(c);
                    }
                }
                if !run.is_empty() {
                    word.push(Part {
                        text: run,
                        style,
                        link,
                    });
                }
            }
        }
    }
    if !word.is_empty() {
        units.push(Unit::Word(word));
    }
    units
}

/// Width of the widest word in `pieces`: the narrowest they can wrap to
/// without splitting a word.
pub fn longest_word(pieces: &[Piece]) -> usize {
    units(pieces)
        .iter()
        .map(|u| match u {
            Unit::Word(parts) => parts.iter().map(|p| width(&p.text)).sum(),
            _ => 0,
        })
        .max()
        .unwrap_or(0)
}

/// Wraps styled pieces into lines at most `width` columns wide.
///
/// Runs of whitespace collapse to one space, and spaces at wrap points are
/// dropped. A word longer than a whole line is split between graphemes.
/// Always returns at least one (possibly empty) line.
pub fn wrap(pieces: &[Piece], width: usize) -> Vec<Vec<Span<'static>>> {
    wrap_links(pieces, width)
        .into_iter()
        .map(|l| l.spans)
        .collect()
}

/// Lines being built by [`wrap_links`].
#[derive(Default)]
struct Lines {
    done: Vec<WLine>,
    cur: WLine,
    w: usize,
}

impl Lines {
    fn push(&mut self, text: String, style: Style, link: Option<u32>) {
        let tw = width(&text);
        if let Some(id) = link {
            match self.cur.links.last_mut() {
                Some(l) if l.id == id && l.end == self.w => l.end += tw,
                _ => self.cur.links.push(LinkSpan {
                    start: self.w,
                    end: self.w + tw,
                    id,
                }),
            }
        }
        self.cur.spans.push(Span::styled(text, style));
        self.w += tw;
    }

    fn newline(&mut self) {
        self.done.push(std::mem::take(&mut self.cur));
        self.w = 0;
    }
}

/// [`wrap`], also saying where each link ended up.
pub fn wrap_links(pieces: &[Piece], width: usize) -> Vec<WLine> {
    let width = width.max(1);
    let mut out = Lines::default();
    let mut space: Option<(Style, Option<u32>)> = None;

    for unit in units(pieces) {
        match unit {
            Unit::Break => {
                out.newline();
                space = None;
            }
            Unit::Space(style, link) => {
                if out.w > 0 {
                    space = Some((style, link));
                }
            }
            Unit::Word(parts) => {
                let w: usize = parts.iter().map(|p| self::width(&p.text)).sum();
                let space_w = usize::from(space.is_some());
                if out.w > 0 && out.w + space_w + w > width {
                    out.newline();
                } else if let Some((style, link)) = space {
                    out.push(" ".into(), style, link);
                }
                space = None;

                if w <= width - out.w {
                    for p in parts {
                        out.push(p.text, p.style, p.link);
                    }
                    continue;
                }
                // Too long for any line: split it wherever it runs out of room.
                for p in parts {
                    let mut chunk = String::new();
                    let mut chunk_w = 0;
                    for g in p.text.graphemes(true) {
                        let gw = self::width(g);
                        if out.w + chunk_w + gw > width && out.w + chunk_w > 0 {
                            if !chunk.is_empty() {
                                out.push(std::mem::take(&mut chunk), p.style, p.link);
                                chunk_w = 0;
                            }
                            out.newline();
                        }
                        chunk.push_str(g);
                        chunk_w += gw;
                    }
                    if !chunk.is_empty() {
                        out.push(chunk, p.style, p.link);
                    }
                }
            }
        }
    }
    if !out.cur.spans.is_empty() || out.done.is_empty() {
        out.newline();
    }
    out.done
        .into_iter()
        .map(|l| WLine {
            spans: merge(l.spans),
            links: l.links,
        })
        .collect()
}

/// Joins neighboring spans that have the same style.
fn merge(spans: Vec<Span<'static>>) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::with_capacity(spans.len());
    for span in spans {
        match out.last_mut() {
            Some(last) if last.style == span.style => {
                last.content = format!("{}{}", last.content, span.content).into();
            }
            _ => out.push(span),
        }
    }
    out
}

/// Splits one line of preformatted text into chunks at most `width` columns
/// wide, without looking for word boundaries (for code).
pub fn hard_wrap(spans: Vec<Span<'static>>, width: usize) -> Vec<Vec<Span<'static>>> {
    let width = width.max(1);
    if spans_width(&spans) <= width {
        return vec![spans];
    }
    let mut lines = Vec::new();
    let mut cur = Vec::new();
    let mut cur_w = 0;
    for span in spans {
        let mut chunk = String::new();
        for g in span.content.graphemes(true) {
            let gw = width_of(g);
            if cur_w + gw > width && cur_w > 0 {
                if !chunk.is_empty() {
                    cur.push(Span::styled(std::mem::take(&mut chunk), span.style));
                }
                lines.push(std::mem::take(&mut cur));
                cur_w = 0;
            }
            chunk.push_str(g);
            cur_w += gw;
        }
        if !chunk.is_empty() {
            cur.push(Span::styled(chunk, span.style));
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

fn width_of(s: &str) -> usize {
    width(s)
}

/// Wraps one line of source text to `width` columns, keeping its spacing
/// exactly. Breaks after the last space that fits, or mid-word if a word
/// won't fit on a line of its own.
pub fn wrap_source(spans: Vec<Span<'static>>, width: usize) -> Vec<Vec<Span<'static>>> {
    let width = width.max(1);
    if spans_width(&spans) <= width {
        return vec![spans];
    }
    let cells: Vec<(&str, Style, usize)> = spans
        .iter()
        .flat_map(|s| {
            s.content
                .graphemes(true)
                .map(move |g| (g, s.style, width_of(g)))
        })
        .collect();
    let mut lines = Vec::new();
    let mut start = 0;
    while start < cells.len() {
        let mut end = start;
        let mut w = 0;
        while end < cells.len() && w + cells[end].2 <= width {
            w += cells[end].2;
            end += 1;
        }
        if end < cells.len() {
            // Back up to just after the last space, if there's one.
            if let Some(space) = (start + 1..end).rev().find(|&i| cells[i - 1].0 == " ") {
                end = space;
            }
            end = end.max(start + 1);
        }
        let line = cells[start..end]
            .iter()
            .map(|&(g, style, _)| Span::styled(g.to_string(), style))
            .collect();
        lines.push(merge(line));
        start = end;
    }
    lines
}

/// The part of a line from column `start`, at most `len` columns wide. A
/// wide character cut in half by either edge becomes spaces.
pub fn slice(spans: &[Span<'static>], start: usize, len: usize) -> Vec<Span<'static>> {
    let end = start + len;
    let mut out = Vec::new();
    let mut col = 0;
    for span in spans {
        let mut text = String::new();
        for g in span.content.graphemes(true) {
            let w = width_of(g);
            let (from, to) = (col, col + w);
            col = to;
            if to <= start || from >= end {
                continue;
            }
            if from < start || to > end {
                let visible = to.min(end) - from.max(start);
                text.extend(std::iter::repeat_n(' ', visible));
            } else {
                text.push_str(g);
            }
        }
        if !text.is_empty() {
            out.push(Span::styled(text, span.style));
        }
        if col >= end {
            break;
        }
    }
    out
}

/// Patches `style` onto columns `start..end` of a line.
pub fn restyle(
    spans: Vec<Span<'static>>,
    start: usize,
    end: usize,
    style: Style,
) -> Vec<Span<'static>> {
    let mut out = Vec::with_capacity(spans.len() + 2);
    let mut col = 0;
    for span in spans {
        let w = width(&span.content);
        if col + w <= start || col >= end {
            col += w;
            out.push(span);
            continue;
        }
        // Split the span into the parts before, inside and after the range.
        let mut parts: [String; 3] = Default::default();
        for g in span.content.graphemes(true) {
            let part = if col < start {
                0
            } else if col < end {
                1
            } else {
                2
            };
            parts[part].push_str(g);
            col += width_of(g);
        }
        for (i, text) in parts.into_iter().enumerate() {
            if !text.is_empty() {
                let s = if i == 1 {
                    span.style.patch(style)
                } else {
                    span.style
                };
                out.push(Span::styled(text, s));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[Vec<Span>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    fn plain(s: &str) -> Vec<Piece> {
        vec![Piece::Text(s.into(), Style::default())]
    }

    #[test]
    fn wraps_at_word_boundaries() {
        let lines = wrap(&plain("the quick brown fox jumps"), 10);
        assert_eq!(text(&lines), ["the quick", "brown fox", "jumps"]);
    }

    #[test]
    fn collapses_whitespace() {
        let lines = wrap(&plain("  a   b  "), 10);
        assert_eq!(text(&lines), ["a b"]);
    }

    #[test]
    fn keeps_styled_word_together() {
        let bold = Style::default().bold();
        let pieces = vec![
            Piece::Text("aaaa ".into(), Style::default()),
            Piece::Text("bb".into(), bold),
            Piece::Text("cc".into(), Style::default()),
        ];
        assert_eq!(text(&wrap(&pieces, 6)), ["aaaa", "bbcc"]);
    }

    #[test]
    fn splits_long_words() {
        assert_eq!(
            text(&wrap(&plain("abcdefgh ij"), 3)),
            ["abc", "def", "gh", "ij"]
        );
    }

    #[test]
    fn counts_wide_characters() {
        assert_eq!(
            text(&wrap(&plain("日本語 日本語"), 7)),
            ["日本語", "日本語"]
        );
    }

    #[test]
    fn merges_same_style_spans() {
        let lines = wrap(&plain("one two three"), 80);
        assert_eq!(lines[0].len(), 1);
    }

    #[test]
    fn tracks_links_across_lines() {
        let pieces = vec![
            Piece::Text("see ".into(), Style::default()),
            Piece::LinkStart(7),
            Piece::Text("the docs".into(), Style::default()),
            Piece::LinkEnd,
            Piece::Text(" now".into(), Style::default()),
        ];
        let lines = wrap_links(&pieces, 80);
        assert_eq!(
            lines[0].links,
            [LinkSpan {
                start: 4,
                end: 12,
                id: 7
            }]
        );
        let lines = wrap_links(&pieces, 7);
        assert_eq!(
            lines[0].links,
            [LinkSpan {
                start: 4,
                end: 7,
                id: 7
            }]
        );
        assert_eq!(
            lines[1].links,
            [LinkSpan {
                start: 0,
                end: 4,
                id: 7
            }]
        );
    }

    #[test]
    fn honors_hard_breaks() {
        let pieces = vec![
            Piece::Text("a".into(), Style::default()),
            Piece::Break,
            Piece::Text("b".into(), Style::default()),
        ];
        assert_eq!(text(&wrap(&pieces, 10)), ["a", "b"]);
    }

    #[test]
    fn wrap_source_breaks_after_spaces() {
        let lines = wrap_source(vec![Span::raw("  indented words here")], 12);
        assert_eq!(text(&lines), ["  indented ", "words here"]);
        let lines = wrap_source(vec![Span::raw("abcdefgh")], 3);
        assert_eq!(text(&lines), ["abc", "def", "gh"]);
    }

    #[test]
    fn slices_columns() {
        let spans = vec![Span::raw("ab"), Span::raw("日本")];
        assert_eq!(text(&[slice(&spans, 1, 3)]), ["b日"]);
        assert_eq!(text(&[slice(&spans, 3, 3)]), [" 本"]);
        assert_eq!(text(&[slice(&spans, 3, 2)]), ["  "]);
        assert_eq!(text(&[slice(&spans, 2, 3)]), ["日 "]);
    }

    #[test]
    fn restyles_a_range() {
        let bold = Style::new().bold();
        let spans = restyle(vec![Span::raw("hello"), Span::raw(" world")], 3, 7, bold);
        assert_eq!(text(std::slice::from_ref(&spans)), ["hello world"]);
        let styled: Vec<_> = spans
            .iter()
            .filter(|s| s.style == bold)
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(styled, ["lo", " w"]);
    }

    #[test]
    fn hard_wrap_splits_code() {
        let lines = hard_wrap(vec![Span::raw("abcdef")], 4);
        assert_eq!(text(&lines), ["abcd", "ef"]);
    }
}
