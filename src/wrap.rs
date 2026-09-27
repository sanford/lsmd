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
}

/// Display width of `s` in terminal columns.
pub fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Total display width of a line of spans.
pub fn spans_width(spans: &[Span]) -> usize {
    spans.iter().map(|s| width(&s.content)).sum()
}

enum Unit {
    /// Consecutive non-space text, possibly in several styles (`**foo**bar`).
    Word(Vec<(String, Style)>),
    Space(Style),
    Break,
}

fn units(pieces: &[Piece]) -> Vec<Unit> {
    let mut units = Vec::new();
    let mut word: Vec<(String, Style)> = Vec::new();
    for piece in pieces {
        match piece {
            Piece::Break => {
                if !word.is_empty() {
                    units.push(Unit::Word(std::mem::take(&mut word)));
                }
                units.push(Unit::Break);
            }
            Piece::Text(text, style) => {
                let mut run = String::new();
                for c in text.chars() {
                    if c.is_whitespace() && c != '\u{a0}' {
                        if !run.is_empty() {
                            word.push((std::mem::take(&mut run), *style));
                        }
                        if !word.is_empty() {
                            units.push(Unit::Word(std::mem::take(&mut word)));
                        }
                        if !matches!(units.last(), Some(Unit::Space(_))) {
                            units.push(Unit::Space(*style));
                        }
                    } else {
                        run.push(c);
                    }
                }
                if !run.is_empty() {
                    word.push((run, *style));
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
            Unit::Word(parts) => parts.iter().map(|(t, _)| width(t)).sum(),
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
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut cur: Vec<Span<'static>> = Vec::new();
    let mut cur_w = 0;
    let mut space: Option<Style> = None;

    for unit in units(pieces) {
        match unit {
            Unit::Break => {
                lines.push(std::mem::take(&mut cur));
                cur_w = 0;
                space = None;
            }
            Unit::Space(style) => {
                if cur_w > 0 {
                    space = Some(style);
                }
            }
            Unit::Word(parts) => {
                let w: usize = parts.iter().map(|(t, _)| self::width(t)).sum();
                let space_w = usize::from(space.is_some());
                if cur_w > 0 && cur_w + space_w + w > width {
                    lines.push(std::mem::take(&mut cur));
                    cur_w = 0;
                } else if let Some(style) = space {
                    cur.push(Span::styled(" ", style));
                    cur_w += 1;
                }
                space = None;

                if w <= width - cur_w {
                    for (text, style) in parts {
                        cur.push(Span::styled(text, style));
                    }
                    cur_w += w;
                    continue;
                }
                // Too long for any line: split it wherever it runs out of room.
                for (text, style) in parts {
                    let mut chunk = String::new();
                    for g in text.graphemes(true) {
                        let gw = self::width(g);
                        if cur_w + gw > width && cur_w > 0 {
                            if !chunk.is_empty() {
                                cur.push(Span::styled(std::mem::take(&mut chunk), style));
                            }
                            lines.push(std::mem::take(&mut cur));
                            cur_w = 0;
                        }
                        chunk.push_str(g);
                        cur_w += gw;
                    }
                    if !chunk.is_empty() {
                        cur.push(Span::styled(chunk, style));
                    }
                }
            }
        }
    }
    if !cur.is_empty() || lines.is_empty() {
        lines.push(cur);
    }
    lines.into_iter().map(merge).collect()
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
        assert_eq!(text(&wrap(&plain("abcdefgh ij"), 3)), ["abc", "def", "gh", "ij"]);
    }

    #[test]
    fn counts_wide_characters() {
        assert_eq!(text(&wrap(&plain("日本語 日本語"), 7)), ["日本語", "日本語"]);
    }

    #[test]
    fn merges_same_style_spans() {
        let lines = wrap(&plain("one two three"), 80);
        assert_eq!(lines[0].len(), 1);
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
    fn hard_wrap_splits_code() {
        let lines = hard_wrap(vec![Span::raw("abcdef")], 4);
        assert_eq!(text(&lines), ["abcd", "ef"]);
    }
}
