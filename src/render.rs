//! Renders Markdown to styled lines wrapped to a given width.

use crate::highlight::{self, expand_tabs};
use crate::theme::Theme;
use crate::wrap::{self, LinkSpan, Piece, WLine};
use comrak::nodes::{AlertType, ListDelimType, ListType, NodeValue, TableAlignment};
use comrak::{Anchorizer, Arena, Node, Options, parse_document};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use std::cell::RefCell;
use std::path::Path;

/// One rendered line.
#[derive(Clone, Debug)]
pub struct RLine {
    pub spans: Vec<Span<'static>>,
    /// The 1-based source lines (first, last) of the top-level block this
    /// line was rendered from. `None` for blank lines between blocks.
    pub src: Option<(usize, usize)>,
    /// For unwrapped code: the column where the code starts, after any
    /// quote bars or list indents. Everything from there scrolls sideways.
    pub scroll_from: Option<usize>,
    /// The links on this line, by column. Their ids index [`Rendered::links`].
    pub links: Vec<LinkSpan>,
}

/// A rendered document.
pub struct Rendered {
    pub lines: Vec<RLine>,
    pub headings: Vec<Heading>,
    /// Link targets, as written.
    pub links: Vec<String>,
    pub code_blocks: Vec<CodeBlock>,
}

pub struct CodeBlock {
    /// The rendered lines it covers.
    pub lines: std::ops::Range<usize>,
    pub lang: String,
    /// The code, as written.
    pub code: String,
}

pub struct Heading {
    /// The rendered line it starts on.
    pub line: usize,
    pub level: u8,
    pub text: String,
    /// Its anchor, as GitHub makes them: `#getting-started`.
    pub slug: String,
}

impl RLine {
    #[cfg(test)]
    pub fn text(&self) -> String {
        self.spans.iter().map(|s| s.content.as_ref()).collect()
    }
}

/// How deeply blocks may nest before the rest is shown as plain text.
const MAX_DEPTH: usize = 64;

/// GitHub-flavored Markdown, plus the extensions GitHub renders.
pub(crate) fn options() -> Options<'static> {
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
/// width is too small for a table's borders or a list's indent). Without
/// `wrap_code`, long lines of code run past `width`, to be scrolled to.
/// Relative links are checked against the files in `base`, the document's
/// directory.
pub fn render(
    md: &str,
    width: usize,
    theme: &Theme,
    wrap_code: bool,
    base: Option<&Path>,
    site: Option<&Path>,
) -> Rendered {
    let arena = Arena::new();
    let root = parse_document(&arena, md, &options());
    let mut r = Renderer {
        theme,
        width: width.max(1),
        wrap_code,
        out: Vec::new(),
        prefix: Vec::new(),
        gap: false,
        src: None,
        list_depth: 0,
        depth: 0,
        footnotes: false,
        base,
        site,
        links: RefCell::new(Vec::new()),
        inline_depth: std::cell::Cell::new(0),
        headings: Vec::new(),
        code_blocks: Vec::new(),
        anchors: Anchorizer::new(),
    };
    for child in root.children() {
        let pos = child.data().sourcepos;
        r.src = Some((pos.start.line, pos.end.line));
        r.block(child, false);
    }
    Rendered {
        lines: r.out,
        headings: r.headings,
        links: r.links.into_inner(),
        code_blocks: r.code_blocks,
    }
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
    wrap_code: bool,
    out: Vec<RLine>,
    prefix: Vec<Prefix>,
    /// A blank line is due before the next line.
    gap: bool,
    src: Option<(usize, usize)>,
    list_depth: usize,
    /// How many blocks the current one is nested in.
    depth: usize,
    footnotes: bool,
    base: Option<&'t Path>,
    /// Where links starting with `/` start from: see `files::site_root`.
    site: Option<&'t Path>,
    /// Nesting of the inline being collected. A Cell for the same reason.
    inline_depth: std::cell::Cell<usize>,
    /// Link targets. A RefCell because inlines are collected through `&self`.
    links: RefCell<Vec<String>>,
    headings: Vec<Heading>,
    code_blocks: Vec<CodeBlock>,
    anchors: Anchorizer,
}

impl Renderer<'_> {
    /// Columns left for content inside the current prefixes.
    fn avail(&self) -> usize {
        self.width.saturating_sub(self.prefix_width()).max(1)
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
            let mut blank: Vec<Span<'static>> = self
                .prefix
                .iter()
                .flat_map(|p| p.rest.iter().cloned())
                .collect();
            trim_end(&mut blank);
            self.out.push(RLine {
                spans: blank,
                src: None,
                scroll_from: None,
                links: Vec::new(),
            });
        }
    }

    fn emit(&mut self, spans: Vec<Span<'static>>) {
        self.flush_gap();
        let mut line = Vec::new();
        for p in &mut self.prefix {
            line.extend(if p.used {
                p.rest.clone()
            } else {
                p.first.clone()
            });
            p.used = true;
        }
        // Every rendered line comes through here: the one place to make
        // sure nothing in a document can act on the terminal.
        line.extend(spans.into_iter().map(|mut span| {
            if let std::borrow::Cow::Owned(text) = crate::safe::printable(&span.content) {
                span.content = text.into();
            }
            span
        }));
        self.out.push(RLine {
            spans: line,
            src: self.src,
            scroll_from: None,
            links: Vec::new(),
        });
    }

    /// Emits a line of unwrapped code, which scrolls sideways.
    fn emit_code(&mut self, spans: Vec<Span<'static>>) {
        let prefix_w = self.prefix_width();
        self.emit(spans);
        if let Some(line) = self.out.last_mut() {
            line.scroll_from = Some(prefix_w);
        }
    }

    fn prefix_width(&self) -> usize {
        self.prefix.iter().map(|p| wrap::spans_width(&p.rest)).sum()
    }

    /// Emits a wrapped line, keeping track of its links.
    fn emit_wrapped(&mut self, line: WLine) {
        let shift = self.prefix_width();
        self.emit(line.spans);
        if let Some(out) = self.out.last_mut() {
            out.links = line
                .links
                .into_iter()
                .map(|l| LinkSpan {
                    start: l.start + shift,
                    end: l.end + shift,
                    ..l
                })
                .collect();
        }
    }

    fn para(&mut self, pieces: &[Piece]) {
        for line in wrap::wrap_links(pieces, self.avail()) {
            self.emit_wrapped(line);
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
        // Hostile files can nest quotes or lists thousands deep, which would
        // overflow the stack. Nothing real comes near this, so past it,
        // show the text.
        if self.depth >= MAX_DEPTH {
            let text = plain_text(node);
            self.para(&[Piece::Text(text, self.theme.dim())]);
            return;
        }
        self.depth += 1;
        self.block_inner(node, tight);
        self.depth -= 1;
    }

    fn block_inner(&mut self, node: Node<'_>, tight: bool) {
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
            pieces.push(Piece::Text(
                format!("{} ", "#".repeat(level.into())),
                self.theme.dim(),
            ));
        }
        pieces.extend(self.inlines(node, style));
        let lines = wrap::wrap_links(&pieces, self.avail());
        let w = lines
            .iter()
            .map(|l| wrap::spans_width(&l.spans))
            .max()
            .unwrap_or(0);
        self.flush_gap();
        let text = plain_text(node);
        self.headings.push(Heading {
            line: self.out.len(),
            level,
            slug: self.anchors.anchorize(&text),
            text,
        });
        for line in lines {
            self.emit_wrapped(line);
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
        let delim = if list.delimiter == ListDelimType::Paren {
            ')'
        } else {
            '.'
        };
        let bullet = ["•", "◦", "▪"][self.list_depth % 3];
        self.list_depth += 1;
        for (i, item) in items.into_iter().enumerate() {
            let (marker, style) = match &item.data().value {
                NodeValue::TaskItem(task) if task.symbol.is_some() => {
                    ("☑".to_string(), self.theme.task_done())
                }
                NodeValue::TaskItem(_) => ("☐".to_string(), self.theme.bullet()),
                _ if list.list_type == ListType::Ordered => (
                    format!("{:>num_w$}{delim}", list.start + i),
                    self.theme.bullet(),
                ),
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
        self.flush_gap();
        let start = self.out.len();
        self.code_block_lines(info, literal);
        self.code_blocks.push(CodeBlock {
            lines: start..self.out.len(),
            lang: highlight::language(info).to_string(),
            code: literal.to_string(),
        });
    }

    fn code_block_lines(&mut self, info: &str, literal: &str) {
        let lang = highlight::language(info);
        let theme = self.theme;
        let base = theme.code_block();
        let lines = highlight::highlight(literal, lang, theme, base);
        // Unwrapped, every line is one chunk, however long.
        let wrap_code = self.wrap_code;
        let chunks = |line: Vec<Span<'static>>, width: usize| {
            if wrap_code {
                wrap::hard_wrap(line, width)
            } else {
                vec![line]
            }
        };
        if !theme.color {
            // No background to set the code apart, so indent it instead.
            let avail = self.avail().saturating_sub(4);
            for line in lines {
                for chunk in chunks(line, avail) {
                    let mut spans = vec![Span::raw("    ")];
                    spans.extend(chunk);
                    trim_end(&mut spans);
                    self.emit_code(spans);
                }
            }
            return;
        }
        // A block of background color the full width (or as wide as the
        // widest line, unwrapped), with a column of padding on each side and
        // the language in the top right corner.
        let inner = self.avail().saturating_sub(2).max(1);
        let widest = lines
            .iter()
            .map(|l| wrap::spans_width(l))
            .max()
            .unwrap_or(0);
        let block_w = if wrap_code { inner } else { inner.max(widest) };
        let mut first = true;
        for line in lines {
            for chunk in chunks(line, inner) {
                let mut pad = block_w.saturating_sub(wrap::spans_width(&chunk)) + 1;
                let mut spans = vec![Span::styled(" ", base)];
                spans.extend(chunk);
                let label_w = wrap::width(lang);
                if first && label_w > 0 && pad > label_w + 2 && block_w == inner {
                    spans.push(Span::styled(" ".repeat(pad - label_w - 1), base));
                    spans.push(Span::styled(lang.to_string(), base.patch(theme.dim())));
                    pad = 1;
                }
                spans.push(Span::styled(" ".repeat(pad), base));
                self.emit_code(spans);
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
                let base = if header {
                    header_style
                } else {
                    Style::default()
                };
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
            .map(|c| {
                rows.iter()
                    .map(|(_, cells)| wrap::longest_word(&cells[c]))
                    .max()
                    .unwrap_or(0)
            })
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
            let mut wrapped: Vec<_> = cells
                .iter()
                .zip(&widths)
                .map(|(cell, &w)| wrap::wrap_links(cell, w))
                .collect();
            let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);
            for i in 0..height {
                let mut row = WLine {
                    spans: vec![Span::styled("│", border)],
                    links: Vec::new(),
                };
                let mut col = 1;
                for (c, lines) in wrapped.iter_mut().enumerate() {
                    let line = lines.get_mut(i).map(std::mem::take).unwrap_or_default();
                    let line_w = wrap::spans_width(&line.spans);
                    let slack = widths[c].saturating_sub(line_w);
                    let left = match aligns[c] {
                        TableAlignment::Center => slack / 2,
                        TableAlignment::Right => slack,
                        _ => 0,
                    };
                    let start = col + left + 1;
                    row.spans.push(Span::raw(" ".repeat(left + 1)));
                    row.spans.extend(line.spans);
                    row.links.extend(line.links.into_iter().map(|l| LinkSpan {
                        start: l.start + start,
                        end: l.end + start,
                        ..l
                    }));
                    row.spans.push(Span::raw(" ".repeat(slack - left + 1)));
                    row.spans.push(Span::styled("│", border));
                    col += widths[c] + 3;
                }
                self.emit_wrapped(row);
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
        // As for blocks: deep enough nesting could overflow the stack.
        if self.inline_depth.get() >= MAX_DEPTH {
            out.push(Piece::Text(plain_text(node), style));
            return;
        }
        self.inline_depth.set(self.inline_depth.get() + 1);
        self.inline_inner(node, style, out);
        self.inline_depth.set(self.inline_depth.get() - 1);
    }

    fn inline_inner(&self, node: Node<'_>, style: Style, out: &mut Vec<Piece>) {
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
                let url = link.url.as_str();
                let link_style = if self.broken(url) {
                    theme.broken_link()
                } else {
                    theme.link()
                };
                let text_start = out.len();
                out.push(Piece::LinkStart(self.add_link(url)));
                self.inline_children(node, style.patch(link_style), out);
                out.push(Piece::LinkEnd);
                let text: String = out[text_start..]
                    .iter()
                    .filter_map(|p| match p {
                        Piece::Text(t, _) => Some(t.as_str()),
                        _ => None,
                    })
                    .collect();
                let shown = url.strip_prefix("mailto:").unwrap_or(url);
                if !url.is_empty() && !url.starts_with('#') && text != shown {
                    out.push(Piece::Text(format!(" ({url})"), theme.dim()));
                }
            }
            NodeValue::WikiLink(link) => {
                out.push(Piece::LinkStart(self.add_link(&link.url)));
                self.inline_children(node, style.patch(theme.link()), out);
                out.push(Piece::LinkEnd);
            }
            NodeValue::Image(link) => {
                let alt = plain_text(node);
                let label = if alt.is_empty() {
                    "[image]".into()
                } else {
                    format!("[image: {alt}]")
                };
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

    fn add_link(&self, url: &str) -> u32 {
        let mut links = self.links.borrow_mut();
        links.push(url.to_string());
        (links.len() - 1) as u32
    }

    /// Whether `url` is a relative link to a file that isn't there.
    fn broken(&self, url: &str) -> bool {
        let Some(base) = self.base else { return false };
        let Some(path) = local_path(url) else {
            return false;
        };
        !path.is_empty() && !local_target(base, self.site, &path).exists()
    }

    fn inline_children(&self, node: Node<'_>, style: Style, out: &mut Vec<Piece>) {
        for child in node.children() {
            self.inline(child, style, out);
        }
    }
}

/// The file part of a link that points into the file system rather than
/// the web: `docs/a b.md` for `docs/a%20b.md#usage`.
pub fn local_path(url: &str) -> Option<String> {
    let scheme = url.find(':').is_some_and(|i| {
        url[..i]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
            && i > 1
    });
    if scheme || url.starts_with("//") {
        return None;
    }
    let path = url.split(['#', '?']).next().unwrap_or("");
    Some(percent_decode(path))
}

/// The file a link's path (from [`local_path`]) points to, from a document
/// in `base`. A leading `/` means `site`, the top of the repository, as on
/// GitHub, rather than the top of the disk.
pub fn local_target(base: &Path, site: Option<&Path>, path: &str) -> std::path::PathBuf {
    match (path.strip_prefix('/'), site) {
        (Some(rest), Some(site)) => site.join(rest),
        _ => base.join(path),
    }
}

/// Decodes `%20` and friends. Invalid escapes are kept as they are.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = s.get(i + 1..i + 3)
            && let Ok(b) = u8::from_str_radix(hex, 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The text of a node's descendants with formatting dropped (image alt text).
pub(crate) fn plain_text(node: Node<'_>) -> String {
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
        render(md, width, &Theme::plain(), true, None, None)
            .lines
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
        let lines = render(
            "# Title\n\npara one\nstill one\n\npara two\n",
            80,
            &Theme::plain(),
            true,
            None,
            None,
        )
        .lines;
        let src: Vec<_> = lines.iter().map(|l| l.src).collect();
        assert_eq!(
            src,
            [
                Some((1, 1)),
                Some((1, 1)),
                None,
                Some((3, 4)),
                None,
                Some((6, 6))
            ]
        );
    }

    #[test]
    fn leaves_code_unwrapped_for_scrolling() {
        let md = "> ```\n> a long line of code\n> ```\n";
        let lines = render(md, 12, &Theme::plain(), false, None, None).lines;
        let code = lines.iter().find(|l| l.text().contains("long")).unwrap();
        assert_eq!(code.text(), "│     a long line of code");
        assert_eq!(code.scroll_from, Some(2));
    }

    #[test]
    fn resolves_slash_links_from_the_site_root() {
        let dir = std::env::temp_dir().join(format!("lsmd-slash-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::write(dir.join("docs/guide.md"), "").unwrap();
        let base = dir.join("docs");
        assert_eq!(
            local_target(&base, Some(&dir), "/docs/guide.md"),
            dir.join("docs/guide.md")
        );
        assert_eq!(
            local_target(&base, Some(&dir), "guide.md"),
            dir.join("docs/guide.md")
        );
        // Broken only when the file really isn't there.
        let md = "[ok](/docs/guide.md) [gone](/docs/nope.md)\n";
        let r = render(md, 80, &Theme::plain(), true, Some(&base), Some(&dir));
        let r_plain = render(
            md,
            80,
            &Theme::new(crate::theme::Mode::Dark, true),
            true,
            Some(&base),
            Some(&dir),
        );
        let styles: Vec<_> = r_plain.lines[0]
            .spans
            .iter()
            .filter(|s| s.content == "ok" || s.content == "gone")
            .map(|s| {
                (
                    s.content.to_string(),
                    s.style.add_modifier.contains(Modifier::CROSSED_OUT),
                )
            })
            .collect();
        assert_eq!(
            styles,
            [("ok".to_string(), false), ("gone".to_string(), true)]
        );
        assert_eq!(r.links, ["/docs/guide.md", "/docs/nope.md"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn records_headings_and_links() {
        let md = "# Intro\n\nSee [the guide](guide.md#setup) and [web](https://x.io).\n\n## Intro\n\n| a |\n|---|\n| [t](#intro) |\n";
        let r = render(md, 80, &Theme::plain(), true, None, None);
        let slugs: Vec<_> = r
            .headings
            .iter()
            .map(|h| (h.line, h.slug.as_str()))
            .collect();
        assert_eq!(slugs, [(0, "intro"), (5, "intro-1")]);
        assert_eq!(r.links, ["guide.md#setup", "https://x.io", "#intro"]);
        let para = &r.lines[3];
        assert_eq!(
            para.links[0],
            LinkSpan {
                start: 4,
                end: 13,
                id: 0
            }
        );
        let cell = r
            .lines
            .iter()
            .find(|l| l.text().starts_with("│ t"))
            .unwrap();
        assert_eq!(
            cell.links,
            [LinkSpan {
                start: 2,
                end: 3,
                id: 2
            }]
        );
    }

    #[test]
    fn finds_local_paths() {
        assert_eq!(
            local_path("docs/a%20b.md#x").as_deref(),
            Some("docs/a b.md")
        );
        assert_eq!(local_path("#top").as_deref(), Some(""));
        assert_eq!(local_path("https://x.io/a.md"), None);
        assert_eq!(local_path("mailto:me@x.io"), None);
        assert_eq!(local_path("C:/x.md").as_deref(), Some("C:/x.md"));
    }

    #[test]
    fn neutralizes_escape_sequences() {
        let md = "Title \u{1b}]0;x\u{7} and &#27;]52;c;eA==&#7; and `\u{1b}[2J`\n\n[l](https://x/\u{1b}) ![\u{1b}](i.png)\n";
        let text = plain(md, 80);
        assert!(
            text.lines().all(|l| !l.chars().any(crate::safe::is_unsafe)),
            "{text:?}"
        );
        assert!(text.contains('\u{fffd}'));
    }

    #[test]
    fn survives_deep_nesting() {
        // Wide enough for 64 levels of quote bars, so the text fits.
        let quotes = format!("{} deep\n", ">".repeat(10_000));
        assert!(plain(&quotes, 300).contains("deep"));
        let lists: String = (0..2000)
            .map(|i| format!("{}- x\n", "  ".repeat(i)))
            .collect();
        plain(&lists, 80);
        let links = format!("{}x{}", "[".repeat(5000), "](u)".repeat(5000));
        plain(&links, 80);
    }

    #[test]
    fn corpus() {
        let md = include_str!("../tests/corpus/everything.md");
        insta::assert_snapshot!("everything_60", plain(md, 60));
        insta::assert_snapshot!("everything_24", plain(md, 24));
    }
}
