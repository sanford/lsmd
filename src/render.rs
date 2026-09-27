//! Renders Markdown to styled lines wrapped to a given width.

use crate::highlight::{self, expand_tabs};
use crate::theme::Theme;
use crate::wrap::{self, Piece};
use comrak::nodes::{AlertType, ListDelimType, ListType, NodeValue, TableAlignment};
use comrak::{Arena, Node, Options, parse_document};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

/// One rendered line.
#[derive(Clone, Debug)]
pub struct RLine {
    pub spans: Vec<Span<'static>>,
    /// The 1-based source lines (first, last) of the top-level block this
    /// line was rendered from. `None` for blank lines between blocks.
    pub src: Option<(usize, usize)>,
}

impl RLine {
    #[cfg(test)]
    pub fn text(&self) -> String {
        self.spans.iter().map(|s| s.content.as_ref()).collect()
    }
}

/// GitHub-flavored Markdown, plus the extensions GitHub renders.
fn options() -> Options<'static> {
    let mut options = Options::default();
    let ext = &mut options.extension;
    ext.strikethrough = true;
    ext.table = true;
    ext.autolink = true;
    ext.tasklist = true;
    ext.footnotes = true;
    ext.alerts = true;
    ext.math_dollars = true;
    ext.description_lists = true;
    ext.front_matter_delimiter = Some("---".into());
    options
}

/// Renders `md` into lines at most `width` columns wide (except where the
/// width is too small for a table's borders or a list's indent).
pub fn render(md: &str, width: usize, theme: &Theme) -> Vec<RLine> {
    let arena = Arena::new();
    let root = parse_document(&arena, md, &options());
    let mut r = Renderer {
        theme,
        width: width.max(1),
        out: Vec::new(),
        prefix: Vec::new(),
        gap: false,
        src: None,
        list_depth: 0,
        footnotes: false,
    };
    for child in root.children() {
        let pos = child.data().sourcepos;
        r.src = Some((pos.start.line, pos.end.line));
        r.block(child, false);
    }
    r.out
}

/// What goes in front of each line inside a container block: a quote bar,
/// or a list marker on the first line and an indent on the rest.
struct Prefix {
    first: Vec<Span<'static>>,
    rest: Vec<Span<'static>>,
    used: bool,
}

struct Renderer<'t> {
    theme: &'t Theme,
    width: usize,
    out: Vec<RLine>,
    prefix: Vec<Prefix>,
    /// A blank line is due before the next line.
    gap: bool,
    src: Option<(usize, usize)>,
    list_depth: usize,
    footnotes: bool,
}

impl Renderer<'_> {
    /// Columns left for content inside the current prefixes.
    fn avail(&self) -> usize {
        let used: usize = self.prefix.iter().map(|p| wrap::spans_width(&p.rest)).sum();
        self.width.saturating_sub(used).max(1)
    }

    fn push_prefix(&mut self, first: Vec<Span<'static>>, rest: Vec<Span<'static>>) {
        // A blank line owed from before this container belongs outside it.
        self.flush_gap();
        self.prefix.push(Prefix {
            first,
            rest,
            used: false,
        });
    }

    fn pop_prefix(&mut self) {
        self.prefix.pop();
    }

    fn gap(&mut self) {
        self.gap = true;
    }

    fn flush_gap(&mut self) {
        if std::mem::take(&mut self.gap) && !self.out.is_empty() {
            let mut blank: Vec<Span<'static>> =
                self.prefix.iter().flat_map(|p| p.rest.iter().cloned()).collect();
            trim_end(&mut blank);
            self.out.push(RLine {
                spans: blank,
                src: None,
            });
        }
    }

    fn emit(&mut self, spans: Vec<Span<'static>>) {
        self.flush_gap();
        let mut line = Vec::new();
        for p in &mut self.prefix {
            line.extend(if p.used { p.rest.clone() } else { p.first.clone() });
            p.used = true;
        }
        line.extend(spans);
        self.out.push(RLine {
            spans: line,
            src: self.src,
        });
    }

    fn para(&mut self, pieces: &[Piece]) {
        for line in wrap::wrap(pieces, self.avail()) {
            self.emit(line);
        }
    }

    fn children(&mut self, node: Node<'_>, tight: bool) {
        for child in node.children() {
            self.block(child, tight);
        }
    }

    /// Renders a block. `tight` is set inside tight lists, where paragraphs
    /// aren't separated by blank lines.
    fn block(&mut self, node: Node<'_>, tight: bool) {
        let ast = node.data();
        match &ast.value {
            NodeValue::FrontMatter(text) => self.front_matter(text),
            NodeValue::Paragraph => {
                let pieces = self.inlines(node, Style::default());
                self.para(&pieces);
                if !tight {
                    self.gap();
                }
            }
            NodeValue::Heading(h) => self.heading(node, h.level),
            NodeValue::BlockQuote | NodeValue::MultilineBlockQuote(_) => {
                let bar = Span::styled("│ ", self.theme.quote());
                self.push_prefix(vec![bar.clone()], vec![bar]);
                self.children(node, false);
                self.pop_prefix();
                self.gap();
            }
            NodeValue::Alert(alert) => self.alert(node, alert.alert_type, alert.title.as_deref()),
            NodeValue::List(list) => self.list(node, list, tight),
            NodeValue::CodeBlock(code) => {
                self.code_block(&code.info, &code.literal);
                self.gap();
            }
            NodeValue::HtmlBlock(html) => self.html_block(&html.literal),
            NodeValue::ThematicBreak => {
                let rule = "─".repeat(self.avail());
                self.emit(vec![Span::styled(rule, self.theme.dim())]);
                self.gap();
            }
            NodeValue::Table(table) => {
                self.table(node, &table.alignments);
                self.gap();
            }
            NodeValue::FootnoteDefinition(def) => self.footnote(node, &def.name),
            NodeValue::DescriptionTerm => {
                let bold = self.theme.modifier(Style::default(), Modifier::BOLD);
                let pieces = self.inlines(node, bold);
                self.para(&pieces);
            }
            NodeValue::DescriptionDetails => {
                let indent = vec![Span::raw("    ")];
                self.push_prefix(indent.clone(), indent);
                self.children(node, false);
                self.pop_prefix();
            }
            NodeValue::DescriptionList => {
                self.children(node, false);
                self.gap();
            }
            _ => self.children(node, tight),
        }
    }

    fn heading(&mut self, node: Node<'_>, level: u8) {
        let style = self.theme.heading(level);
        let mut pieces = Vec::new();
        if level >= 3 {
            pieces.push(Piece::Text(format!("{} ", "#".repeat(level.into())), self.theme.dim()));
        }
        pieces.extend(self.inlines(node, style));
        let lines = wrap::wrap(&pieces, self.avail());
        let w = lines.iter().map(|l| wrap::spans_width(l)).max().unwrap_or(0);
        for line in lines {
            self.emit(line);
        }
        if level <= 2 {
            let rule = if level == 1 { "═" } else { "─" };
            self.emit(vec![Span::styled(rule.repeat(w), style)]);
        }
        self.gap();
    }

    fn alert(&mut self, node: Node<'_>, kind: AlertType, title: Option<&str>) {
        let style = self.theme.alert(kind);
        let bar = Span::styled("│ ", style);
        self.push_prefix(vec![bar.clone()], vec![bar]);
        let title = title.unwrap_or(match kind {
            AlertType::Note => "Note",
            AlertType::Tip => "Tip",
            AlertType::Important => "Important",
            AlertType::Warning => "Warning",
            AlertType::Caution => "Caution",
        });
        let title_style = self.theme.modifier(style, Modifier::BOLD);
        self.para(&[Piece::Text(title.to_string(), title_style)]);
        self.children(node, false);
        self.pop_prefix();
        self.gap();
    }

    fn list(&mut self, node: Node<'_>, list: &comrak::nodes::NodeList, tight: bool) {
        let items: Vec<_> = node.children().collect();
        let last = list.start + items.len().saturating_sub(1);
        let num_w = last.to_string().len();
        let delim = if list.delimiter == ListDelimType::Paren { ')' } else { '.' };
        let bullet = ["•", "◦", "▪"][self.list_depth % 3];
        self.list_depth += 1;
        for (i, item) in items.into_iter().enumerate() {
            let (marker, style) = match &item.data().value {
                NodeValue::TaskItem(task) if task.symbol.is_some() => {
                    ("☑".to_string(), self.theme.task_done())
                }
                NodeValue::TaskItem(_) => ("☐".to_string(), self.theme.bullet()),
                _ if list.list_type == ListType::Ordered => {
                    (format!("{:>num_w$}{delim}", list.start + i), self.theme.bullet())
                }
                _ => (bullet.to_string(), self.theme.bullet()),
            };
            let indent = " ".repeat(wrap::width(&marker) + 1);
            self.push_prefix(
                vec![Span::styled(marker, style), Span::raw(" ")],
                vec![Span::raw(indent)],
            );
            if item.first_child().is_none() {
                self.emit(Vec::new());
            }
            self.children(item, list.tight);
            self.pop_prefix();
        }
        self.list_depth -= 1;
        if !tight {
            self.gap();
        }
    }

    fn code_block(&mut self, info: &str, literal: &str) {
        let lang = highlight::language(info);
        let theme = self.theme;
        let base = theme.code_block();
        if !theme.color {
            // No background to set the code apart, so indent it instead.
            let avail = self.avail().saturating_sub(4);
            for line in highlight::highlight(literal, lang, theme, base) {
                for chunk in wrap::hard_wrap(line, avail) {
                    let mut spans = vec![Span::raw("    ")];
                    spans.extend(chunk);
                    trim_end(&mut spans);
                    self.emit(spans);
                }
            }
            return;
        }
        // A block of background color the full width, with a column of
        // padding on each side and the language in the top right corner.
        let inner = self.avail().saturating_sub(2).max(1);
        let mut first = true;
        for line in highlight::highlight(literal, lang, theme, base) {
            for chunk in wrap::hard_wrap(line, inner) {
                let mut pad = inner.saturating_sub(wrap::spans_width(&chunk)) + 1;
                let mut spans = vec![Span::styled(" ", base)];
                spans.extend(chunk);
                let label_w = wrap::width(lang);
                if first && label_w > 0 && pad > label_w + 2 {
                    spans.push(Span::styled(" ".repeat(pad - label_w - 1), base));
                    spans.push(Span::styled(lang.to_string(), base.patch(theme.dim())));
                    pad = 1;
                }
                spans.push(Span::styled(" ".repeat(pad), base));
                self.emit(spans);
                first = false;
            }
        }
    }

    fn html_block(&mut self, literal: &str) {
        let pieces = crate::html::block(literal, self.theme);
        if !pieces.is_empty() {
            self.para(&pieces);
            self.gap();
        }
    }

    /// YAML front matter as dimmed `key: value` lines.
    fn front_matter(&mut self, text: &str) {
        for line in text.lines() {
            let line = expand_tabs(line.trim_end());
            if line.is_empty() || line == "---" {
                continue;
            }
            let pieces = match line.split_once(':') {
                Some((key, value)) if !key.starts_with([' ', '-', '#']) => vec![
                    Piece::Text(format!("{key}:"), self.theme.key()),
                    Piece::Text(value.to_string(), self.theme.dim()),
                ],
                _ => vec![Piece::Text(line, self.theme.dim())],
            };
            self.para(&pieces);
        }
        self.gap();
    }

    fn footnote(&mut self, node: Node<'_>, name: &str) {
        if !self.footnotes {
            self.footnotes = true;
            let rule = "─".repeat(self.avail().min(20));
            self.emit(vec![Span::styled(rule, self.theme.dim())]);
            self.gap();
        }
        let label = format!("[{name}]");
        let indent = " ".repeat(wrap::width(&label) + 1);
        self.push_prefix(
            vec![Span::styled(label, self.theme.key()), Span::raw(" ")],
            vec![Span::raw(indent)],
        );
        self.children(node, false);
        self.pop_prefix();
        self.gap();
    }

    fn table(&mut self, node: Node<'_>, aligns: &[TableAlignment]) {
        let ncols = aligns.len();
        if ncols == 0 {
            return;
        }
        let header_style = self.theme.table_header();
        let rows: Vec<(bool, Vec<Vec<Piece>>)> = node
            .children()
            .map(|row| {
                let header = matches!(row.data().value, NodeValue::TableRow(true));
                let base = if header { header_style } else { Style::default() };
                let mut cells: Vec<_> = row.children().map(|c| self.inlines(c, base)).collect();
                cells.resize_with(ncols, Vec::new);
                (header, cells)
            })
            .collect();

        let natural: Vec<usize> = (0..ncols)
            .map(|c| {
                rows.iter()
                    .flat_map(|(_, cells)| wrap::wrap(&cells[c], usize::MAX / 2))
                    .map(|l| wrap::spans_width(&l))
                    .max()
                    .unwrap_or(0)
                    .max(1)
            })
            .collect();
        let words: Vec<usize> = (0..ncols)
            .map(|c| rows.iter().map(|(_, cells)| wrap::longest_word(&cells[c])).max().unwrap_or(0))
            .collect();
        let borders = 3 * ncols + 1;
        let widths = fit_columns(&natural, &words, self.avail().saturating_sub(borders));

        let border = self.theme.border();
        let rule = |l: &str, m: &str, r: &str| {
            let mut s = l.to_string();
            for (i, w) in widths.iter().enumerate() {
                s.push_str(&"─".repeat(w + 2));
                s.push_str(if i + 1 == ncols { r } else { m });
            }
            vec![Span::styled(s, border)]
        };
        self.emit(rule("┌", "┬", "┐"));
        let nrows = rows.len();
        for (r, (header, cells)) in rows.iter().enumerate() {
            let wrapped: Vec<_> = cells
                .iter()
                .zip(&widths)
                .map(|(cell, &w)| wrap::wrap(cell, w))
                .collect();
            let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);
            for i in 0..height {
                let mut spans = vec![Span::styled("│", border)];
                for (c, lines) in wrapped.iter().enumerate() {
                    let line = lines.get(i).cloned().unwrap_or_default();
                    let slack = widths[c].saturating_sub(wrap::spans_width(&line));
                    let left = match aligns[c] {
                        TableAlignment::Center => slack / 2,
                        TableAlignment::Right => slack,
                        _ => 0,
                    };
                    spans.push(Span::raw(" ".repeat(left + 1)));
                    spans.extend(line);
                    spans.push(Span::raw(" ".repeat(slack - left + 1)));
                    spans.push(Span::styled("│", border));
                }
                self.emit(spans);
            }
            if *header && r + 1 < nrows {
                self.emit(rule("├", "┼", "┤"));
            }
        }
        self.emit(rule("└", "┴", "┘"));
    }

    fn inlines(&self, node: Node<'_>, style: Style) -> Vec<Piece> {
        let mut out = Vec::new();
        for child in node.children() {
            self.inline(child, style, &mut out);
        }
        out
    }

    fn inline(&self, node: Node<'_>, style: Style, out: &mut Vec<Piece>) {
        let theme = self.theme;
        let styled = |m| theme.modifier(style, m);
        let ast = node.data();
        match &ast.value {
            NodeValue::Text(text) => out.push(Piece::Text(text.to_string(), style)),
            NodeValue::SoftBreak => out.push(Piece::Text(" ".into(), style)),
            NodeValue::LineBreak => out.push(Piece::Break),
            NodeValue::Code(code) => out.push(Piece::Text(
                code.literal.clone(),
                style.patch(theme.inline_code()),
            )),
            NodeValue::Math(math) => out.push(Piece::Text(
                math.literal.clone(),
                style.patch(theme.inline_code()),
            )),
            NodeValue::Emph => self.inline_children(node, styled(Modifier::ITALIC), out),
            NodeValue::Strong => self.inline_children(node, styled(Modifier::BOLD), out),
            NodeValue::Strikethrough => {
                self.inline_children(node, styled(Modifier::CROSSED_OUT), out)
            }
            NodeValue::Underline | NodeValue::Insert => {
                self.inline_children(node, styled(Modifier::UNDERLINED), out)
            }
            NodeValue::Highlight => self.inline_children(node, styled(Modifier::REVERSED), out),
            NodeValue::SpoileredText => self.inline_children(node, styled(Modifier::DIM), out),
            NodeValue::Link(link) => {
                let text_start = out.len();
                self.inline_children(node, style.patch(theme.link()), out);
                let text: String = out[text_start..]
                    .iter()
                    .filter_map(|p| match p {
                        Piece::Text(t, _) => Some(t.as_str()),
                        Piece::Break => None,
                    })
                    .collect();
                let url = link.url.as_str();
                let shown = url.strip_prefix("mailto:").unwrap_or(url);
                if !url.is_empty() && !url.starts_with('#') && text != shown {
                    out.push(Piece::Text(format!(" ({url})"), theme.dim()));
                }
            }
            NodeValue::WikiLink(_) => self.inline_children(node, style.patch(theme.link()), out),
            NodeValue::Image(link) => {
                let alt = plain_text(node);
                let label = if alt.is_empty() { "[image]".into() } else { format!("[image: {alt}]") };
                out.push(Piece::Text(label, style.patch(theme.key())));
                if !link.url.is_empty() {
                    out.push(Piece::Text(format!(" ({})", link.url), theme.dim()));
                }
            }
            NodeValue::FootnoteReference(r) => {
                out.push(Piece::Text(format!("[{}]", r.name), theme.key()));
            }
            NodeValue::HtmlInline(html) => out.extend(crate::html::inline(html, theme)),
            NodeValue::Raw(text) => out.push(Piece::Text(text.clone(), style)),
            // Paragraphs inside description terms and anything else unexpected.
            _ => self.inline_children(node, style, out),
        }
    }

    fn inline_children(&self, node: Node<'_>, style: Style, out: &mut Vec<Piece>) {
        for child in node.children() {
            self.inline(child, style, out);
        }
    }
}

/// The text of a node's descendants with formatting dropped (image alt text).
fn plain_text(node: Node<'_>) -> String {
    let mut s = String::new();
    for d in node.descendants().skip(1) {
        match &d.data().value {
            NodeValue::Text(t) => s.push_str(t),
            NodeValue::Code(c) => s.push_str(&c.literal),
            NodeValue::SoftBreak | NodeValue::LineBreak => s.push(' '),
            _ => {}
        }
    }
    s
}

/// Shares `room` columns between table columns that would like `natural`
/// widths. If there's room, every column first gets enough for its longest
/// word (`words`), so words aren't split. Every column gets at least 1, even
/// if that overflows `room`.
fn fit_columns(natural: &[usize], words: &[usize], room: usize) -> Vec<usize> {
    if natural.iter().sum::<usize>() <= room {
        return natural.to_vec();
    }
    let min: Vec<usize> = natural.iter().zip(words).map(|(&n, &w)| n.min(w)).collect();
    let min_total: usize = min.iter().sum();
    let widths = if min_total <= room {
        let want: Vec<usize> = natural.iter().zip(&min).map(|(n, m)| n - m).collect();
        let extra = share(&want, room - min_total);
        min.iter().zip(extra).map(|(m, e)| m + e).collect()
    } else {
        share(natural, room)
    };
    widths.into_iter().map(|w| w.max(1)).collect()
}

/// Splits `room` among claims of `want`: small claims are met in full, and
/// the rest split what's left evenly.
fn share(want: &[usize], mut room: usize) -> Vec<usize> {
    let mut got = vec![0; want.len()];
    let mut open: Vec<usize> = (0..want.len()).collect();
    open.sort_by_key(|&c| want[c]);
    while !open.is_empty() {
        let each = room / open.len();
        if want[open[0]] <= each {
            let c = open.remove(0);
            got[c] = want[c];
            room -= want[c];
            continue;
        }
        let extra = room % open.len();
        for (i, &c) in open.iter().enumerate() {
            got[c] = each + usize::from(i < extra);
        }
        break;
    }
    got
}

/// Drops trailing whitespace from a line.
fn trim_end(spans: &mut Vec<Span<'static>>) {
    while let Some(last) = spans.last_mut() {
        let trimmed = last.content.trim_end();
        if trimmed.is_empty() {
            spans.pop();
        } else {
            if trimmed.len() != last.content.len() {
                last.content = trimmed.to_string().into();
            }
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(md: &str, width: usize) -> String {
        render(md, width, &Theme::plain())
            .iter()
            .map(|l| l.text() + "\n")
            .collect()
    }

    #[test]
    fn fits_columns() {
        assert_eq!(fit_columns(&[3, 4], &[3, 4], 10), [3, 4]);
        assert_eq!(fit_columns(&[3, 40, 40], &[3, 5, 5], 23), [3, 10, 10]);
        // The long-worded column keeps its word whole.
        assert_eq!(fit_columns(&[20, 20], &[12, 3], 20), [15, 5]);
        assert_eq!(fit_columns(&[5, 5], &[5, 5], 1), [1, 1]);
    }

    #[test]
    fn records_source_lines() {
        let lines = render("# Title\n\npara one\nstill one\n\npara two\n", 80, &Theme::plain());
        let src: Vec<_> = lines.iter().map(|l| l.src).collect();
        assert_eq!(
            src,
            [Some((1, 1)), Some((1, 1)), None, Some((3, 4)), None, Some((6, 6))]
        );
    }

    #[test]
    fn corpus() {
        let md = include_str!("../tests/corpus/everything.md");
        insta::assert_snapshot!("everything_60", plain(md, 60));
        insta::assert_snapshot!("everything_24", plain(md, 24));
    }
}
