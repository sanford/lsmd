//! Just enough HTML to read the HTML people put in Markdown: READMEs'
//! centered logos, badge rows, `<details>` and `<br>`. Tags are dropped;
//! what they mean is kept where it's simple (breaks, headings, emphasis,
//! links, image alt text).

use crate::theme::Theme;
use crate::wrap::Piece;
use ratatui::style::{Modifier, Style};

/// Tags that start a new line.
const BLOCK: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "br",
    "center",
    "details",
    "dd",
    "div",
    "dl",
    "dt",
    "figcaption",
    "figure",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "li",
    "nav",
    "ol",
    "p",
    "pre",
    "section",
    "summary",
    "table",
    "tr",
    "ul",
];

/// Tags whose content isn't meant to be read.
const HIDDEN: &[&str] = &["script", "style", "template", "noscript"];

/// Converts a block of HTML to wrappable pieces.
pub fn block(html: &str, theme: &Theme) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut rest = html;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("<!--") {
            rest = after.find("-->").map_or("", |i| &after[i + 3..]);
            continue;
        }
        if rest.starts_with('<')
            && let Some(end) = rest.find('>')
        {
            let tag = &rest[1..end];
            rest = &rest[end + 1..];
            let closing = tag.starts_with('/');
            let name = tag_name(tag);
            if BLOCK.contains(&name.as_str()) {
                out.push(Piece::Break);
            }
            if closing {
                if let Some(i) = stack.iter().rposition(|t| *t == name) {
                    stack.truncate(i);
                }
            } else if name == "img" {
                out.push(image(tag, theme));
            } else if !tag.ends_with('/') && !is_void(&name) {
                stack.push(name);
            }
            continue;
        }
        let end = rest[1..].find(['<']).map_or(rest.len(), |i| i + 1);
        let text = &rest[..end];
        rest = &rest[end..];
        if stack.iter().any(|t| HIDDEN.contains(&t.as_str())) {
            continue;
        }
        let pre = stack.iter().any(|t| t == "pre");
        let text = decode(text);
        if pre {
            for (i, line) in text.split('\n').enumerate() {
                if i > 0 {
                    out.push(Piece::Break);
                }
                out.push(Piece::Text(line.to_string(), style(&stack, theme)));
            }
        } else {
            out.push(Piece::Text(collapse(&text), style(&stack, theme)));
        }
    }
    tidy(out)
}

/// The piece for one inline tag, if it has any visible effect.
pub fn inline(tag: &str, theme: &Theme) -> Option<Piece> {
    let inner = tag.trim().strip_prefix('<')?.strip_suffix('>')?;
    match tag_name(inner).as_str() {
        "br" => Some(Piece::Break),
        "img" => Some(image(inner, theme)),
        _ => None,
    }
}

fn image(tag: &str, theme: &Theme) -> Piece {
    let label = match attr(tag, "alt").filter(|a| !a.is_empty()) {
        Some(alt) => format!("[image: {}]", decode(&alt)),
        None => "[image]".into(),
    };
    Piece::Text(label, theme.key())
}

fn style(stack: &[String], theme: &Theme) -> Style {
    let mut style = Style::default();
    for tag in stack {
        style = match tag.as_str() {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                style.patch(theme.heading(tag.as_bytes()[1] - b'0'))
            }
            "b" | "strong" | "summary" | "th" => theme.modifier(style, Modifier::BOLD),
            "i" | "em" => theme.modifier(style, Modifier::ITALIC),
            "s" | "del" | "strike" => theme.modifier(style, Modifier::CROSSED_OUT),
            "u" | "ins" => theme.modifier(style, Modifier::UNDERLINED),
            "code" | "kbd" | "samp" | "tt" => style.patch(theme.inline_code()),
            "a" => style.patch(theme.link()),
            _ => style,
        };
    }
    style
}

/// Collapses runs of whitespace to one space, as browsers do.
fn collapse(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars() {
        if c.is_ascii_whitespace() {
            if !space {
                out.push(' ');
            }
            space = true;
        } else {
            out.push(c);
            space = false;
        }
    }
    out
}

/// Drops whitespace at the start and end of lines, and blank lines.
fn tidy(pieces: Vec<Piece>) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::new();
    // Whether the current line has anything visible on it yet.
    let mut content = false;
    let trim_line_end = |out: &mut Vec<Piece>| {
        while let Some(Piece::Text(t, _)) = out.last_mut() {
            let trimmed = t.trim_end().len();
            if trimmed > 0 {
                t.truncate(trimmed);
                break;
            }
            out.pop();
        }
    };
    for piece in pieces {
        match piece {
            Piece::Break => {
                trim_line_end(&mut out);
                if content {
                    out.push(Piece::Break);
                    content = false;
                }
            }
            Piece::Text(text, style) => {
                let text = if content {
                    text
                } else {
                    text.trim_start().to_string()
                };
                if text.is_empty() {
                    continue;
                }
                content = true;
                out.push(Piece::Text(text, style));
            }
            other => out.push(other),
        }
    }
    trim_line_end(&mut out);
    if matches!(out.last(), Some(Piece::Break)) {
        out.pop();
    }
    out
}

fn tag_name(tag: &str) -> String {
    tag.trim_start_matches('/')
        .split(|c: char| c.is_whitespace() || c == '/' || c == '>')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn is_void(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "source"
            | "track"
            | "wbr"
    )
}

/// The value of attribute `name` in the inside of a tag.
fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name) {
        let start = from + i;
        from = start + name.len();
        let before_ok = start > 0 && lower.as_bytes()[start - 1].is_ascii_whitespace();
        let rest = tag[from..].trim_start();
        if !before_ok || !rest.starts_with('=') {
            continue;
        }
        let value = rest[1..].trim_start();
        return Some(match value.chars().next() {
            Some(q @ ('"' | '\'')) => value[1..].split(q).next().unwrap_or("").to_string(),
            _ => value
                .split(|c: char| c.is_whitespace() || c == '>')
                .next()?
                .to_string(),
        });
    }
    None
}

/// Decodes the common character references.
fn decode(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';').filter(|&e| e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let c = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            "mdash" => Some('—'),
            "ndash" => Some('–'),
            "hellip" => Some('…'),
            "copy" => Some('©'),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(html: &str) -> String {
        block(html, &Theme::plain())
            .iter()
            .map(|p| match p {
                Piece::Text(t, _) => t.as_str(),
                Piece::Break => "\n",
                _ => "",
            })
            .collect()
    }

    #[test]
    fn readme_header() {
        let html = r#"<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="dark.svg">
    <img alt="HyperFrames" src="light.svg" width="300">
  </picture>
</p>
<p align="center"><a href="x"><img src="b.svg" alt="npm version"></a> <a href="y"><img alt='CI'></a></p>"#;
        assert_eq!(
            text(html),
            "[image: HyperFrames]\n[image: npm version] [image: CI]"
        );
    }

    #[test]
    fn keeps_text_and_breaks() {
        assert_eq!(
            text("<div>one<br>two &amp; three</div>"),
            "one\ntwo & three"
        );
        assert_eq!(
            text("<details><summary>More</summary>\n\nHidden<!-- no --></details>"),
            "More\nHidden"
        );
        assert_eq!(text("<script>alert(1)</script>ok"), "ok");
    }

    #[test]
    fn reads_attributes() {
        assert_eq!(
            attr(r#"img src="a" alt="b c""#, "alt").as_deref(),
            Some("b c")
        );
        assert_eq!(attr("img alt=plain src=x", "alt").as_deref(), Some("plain"));
        assert_eq!(attr(r#"img data-alt="no""#, "alt"), None);
    }

    #[test]
    fn decodes_entities() {
        assert_eq!(
            decode("&lt;a&gt; &#39;x&#x27; &bogus; &"),
            "<a> 'x' &bogus; &"
        );
    }
}
