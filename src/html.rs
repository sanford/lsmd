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
        // A tag starts with `<` and a letter, `/` or `!`; "2 < 3" is text.
        let tag_start = rest.starts_with('<')
            && rest[1..].starts_with(|c: char| c.is_ascii_alphabetic() || c == '/' || c == '!');
        if tag_start && let Some(end) = rest.find('>') {
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
        // Text runs to the next tag. (Skip the first character, which may
        // be a `<` that didn't start one, and may be more than one byte.)
        let first = rest.chars().next().map_or(1, char::len_utf8);
        let end = rest[first..].find('<').map_or(rest.len(), |i| i + first);
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

/// An image in HTML: where it is, its alt text, and the size asked for.
#[derive(Debug, PartialEq, Eq)]
pub struct Img {
    pub src: String,
    pub alt: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// The images a block of HTML starts with, before any text (in paragraphs,
/// links, headings and the like), and the rest of it without them: a logo
/// over a project's name. `None` if it doesn't start with any. In a
/// `<picture>`, the source for a `dark` (or light) screen wins.
pub fn leading_images(html: &str, dark: bool) -> Option<(Vec<Img>, String)> {
    let mut images = Vec::new();
    // Where the images' tags are, to leave out of the rest.
    let mut taken = Vec::new();
    let mut text = false;
    let mut stack: Vec<String> = Vec::new();
    // Inside a `<picture>`: the source for this screen, if it has one.
    let mut chosen: Option<String> = None;
    let mut rest = html;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("<!--") {
            rest = after.find("-->").map_or("", |i| &after[i + 3..]);
            continue;
        }
        let tag_start = rest.starts_with('<')
            && rest[1..].starts_with(|c: char| c.is_ascii_alphabetic() || c == '/' || c == '!');
        if tag_start && let Some(end) = rest.find('>') {
            let at = html.len() - rest.len();
            let tag = &rest[1..end];
            rest = &rest[end + 1..];
            let name = tag_name(tag);
            if tag.starts_with('/') {
                if name == "picture" {
                    chosen = None;
                }
                if let Some(i) = stack.iter().rposition(|t| *t == name) {
                    stack.truncate(i);
                }
                continue;
            }
            match name.as_str() {
                "img" if !text => {
                    let mut img = img(tag);
                    if let Some(src) = chosen.take() {
                        img.src = src;
                    }
                    images.push(img);
                    taken.push(at..at + end + 1);
                }
                "source" if stack.iter().any(|t| t == "picture") => {
                    let media = attr(tag, "media").unwrap_or_default().to_ascii_lowercase();
                    let fits = if dark {
                        media.contains("dark")
                    } else {
                        media.contains("light")
                    };
                    let src = attr(tag, "srcset").and_then(|s| {
                        let first = s.split(',').next()?.split_whitespace().next()?;
                        Some(decode(first))
                    });
                    if fits && src.is_some() {
                        chosen = src;
                    }
                }
                _ if !tag.ends_with('/') && !is_void(&name) => stack.push(name),
                _ => {}
            }
            continue;
        }
        let first = rest.chars().next().map_or(1, char::len_utf8);
        let end = rest[first..].find('<').map_or(rest.len(), |i| i + first);
        let run = &rest[..end];
        rest = &rest[end..];
        let hidden = stack.iter().any(|t| HIDDEN.contains(&t.as_str()));
        if !hidden && !decode(run).trim().is_empty() {
            if images.is_empty() {
                return None;
            }
            text = true;
        }
    }
    if images.is_empty() {
        return None;
    }
    let mut left = String::new();
    let mut from = 0;
    for range in taken {
        left.push_str(&html[from..range.start]);
        from = range.end;
    }
    left.push_str(&html[from..]);
    Some((images, left))
}

/// The image an inline `<img …>` tag shows, if that's what it is.
pub fn inline_img(tag: &str) -> Option<Img> {
    let inner = tag.trim().strip_prefix('<')?.strip_suffix('>')?;
    (tag_name(inner) == "img").then(|| img(inner))
}

/// Whether an inline tag shows nothing: `<a …>`, `</a>` and the like.
pub fn invisible(tag: &str) -> bool {
    let Some(inner) = tag
        .trim()
        .strip_prefix('<')
        .and_then(|t| t.strip_suffix('>'))
    else {
        return false;
    };
    !matches!(tag_name(inner).as_str(), "img" | "br" | "hr")
}

fn img(tag: &str) -> Img {
    // `200` or `200px`; not `50%`, which says nothing about the picture.
    let pixels = |name| {
        let value = attr(tag, name)?;
        value
            .trim()
            .trim_end_matches("px")
            .parse()
            .ok()
            .filter(|&n| n > 0)
    };
    Img {
        src: decode(&attr(tag, "src").unwrap_or_default()),
        alt: decode(&attr(tag, "alt").unwrap_or_default()),
        width: pixels("width"),
        height: pixels("height"),
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
    fn finds_blocks_of_nothing_but_images() {
        let logo = r#"<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="dark.png 2x, big.png 3x">
    <source media="(prefers-color-scheme: light)" srcset="light.png">
    <img alt="Logo &amp; name" src="plain.png" width="200px" height="50%">
  </picture>
</p>"#;
        let (dark, rest) = leading_images(logo, true).unwrap();
        assert!(!rest.contains("<img"));
        assert_eq!(
            dark,
            [Img {
                src: "dark.png".into(),
                alt: "Logo & name".into(),
                width: Some(200),
                height: None,
            }]
        );
        assert_eq!(leading_images(logo, false).unwrap().0[0].src, "light.png");
        let badges =
            r#"<p><a href="x"><img src="a.svg" alt="a"></a> <img src='b.png'><!-- c --></p>"#;
        let (images, _) = leading_images(badges, true).unwrap();
        let srcs: Vec<String> = images.into_iter().map(|i| i.src).collect();
        assert_eq!(srcs, ["a.svg", "b.png"]);
        // A logo over a name: the logo, then the name, without the logo.
        let logo = r#"<h1><img src="logo.svg" height="64"><br>Name <img src=x.png></h1>"#;
        let (images, rest) = leading_images(logo, true).unwrap();
        assert_eq!(images.len(), 1, "only the images before any text");
        assert_eq!(rest, "<h1><br>Name <img src=x.png></h1>");
        assert_eq!(text(&rest), "Name [image]");
        assert_eq!(leading_images("<p>Text <img src=a.png></p>", true), None);
        assert_eq!(leading_images("<div>no images</div>", true), None);
        assert!(leading_images("<p><img src=a.png><script>x()</script></p>", true).is_some());
        assert_eq!(
            inline_img("<img src=\"a.png\" width=90>").unwrap().width,
            Some(90)
        );
        assert!(inline_img("<br>").is_none());
        assert!(invisible("</a>") && invisible("<a href=x>") && !invisible("<br/>"));
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
    fn handles_text_starting_with_wide_characters() {
        assert_eq!(text("<div>é is first</div>"), "é is first");
        assert_eq!(text("<p>日本語</p><p>2 < 3</p>"), "日本語\n2 < 3");
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
