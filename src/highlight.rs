//! Syntax highlighting for code blocks.

use crate::theme::Theme;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use std::sync::OnceLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Theme as SyntaxTheme};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;
use two_face::theme::EmbeddedThemeName;

/// The syntax definitions take a few milliseconds to load, so they're only
/// loaded for documents that have code in them.
fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(two_face::syntax::extra_newlines)
}

fn syntax_theme(dark: bool) -> &'static SyntaxTheme {
    static DARK: OnceLock<SyntaxTheme> = OnceLock::new();
    static LIGHT: OnceLock<SyntaxTheme> = OnceLock::new();
    let (cell, name) = if dark {
        (&DARK, EmbeddedThemeName::OneHalfDark)
    } else {
        (&LIGHT, EmbeddedThemeName::OneHalfLight)
    };
    cell.get_or_init(|| two_face::theme::extra().get(name).clone())
}

/// The language named by a fence's info string: `rust` in "rust,ignore" or
/// "rust {.class}".
pub fn language(info: &str) -> &str {
    info.split([' ', ',', '{', '\t']).next().unwrap_or("")
}

/// Highlights `code`, returning one line of spans per source line, with
/// tabs expanded. Unknown languages come back unstyled (apart from `base`).
pub fn highlight(code: &str, lang: &str, theme: &Theme, base: Style) -> Vec<Vec<Span<'static>>> {
    let code = code.strip_suffix('\n').unwrap_or(code);
    let plain = |line: &str| vec![Span::styled(expand_tabs(line), base)];

    if !theme.color || lang.is_empty() {
        return code.split('\n').map(plain).collect();
    }
    let set = syntaxes();
    let Some(syntax) = set.find_syntax_by_token(lang) else {
        return code.split('\n').map(plain).collect();
    };
    let mut hl = HighlightLines::new(syntax, syntax_theme(theme.dark));
    let mut out = Vec::new();
    for line in LinesWithEndings::from(code) {
        let Ok(ranges) = hl.highlight_line(line, set) else {
            out.push(plain(line.trim_end_matches('\n')));
            continue;
        };
        let spans = ranges
            .into_iter()
            .filter_map(|(style, text)| {
                let text = expand_tabs(text.trim_end_matches(['\n', '\r']));
                if text.is_empty() {
                    return None;
                }
                let fg = style.foreground;
                let mut s = base.fg(theme.rgb(fg.r, fg.g, fg.b));
                if style.font_style.contains(FontStyle::BOLD) {
                    s = s.add_modifier(Modifier::BOLD);
                }
                if style.font_style.contains(FontStyle::ITALIC) {
                    s = s.add_modifier(Modifier::ITALIC);
                }
                Some(Span::styled(text, s))
            })
            .collect();
        out.push(spans);
    }
    if out.is_empty() {
        out.push(Vec::new());
    }
    out
}

/// Replaces tabs with spaces up to the next multiple of 4 columns. Terminals
/// would do this themselves, but we need to know the width to wrap and pad.
pub fn expand_tabs(s: &str) -> String {
    if !s.contains('\t') {
        return s.to_string();
    }
    let mut out = String::new();
    let mut col = 0;
    for c in s.chars() {
        if c == '\t' {
            let n = 4 - col % 4;
            out.extend(std::iter::repeat_n(' ', n));
            col += n;
        } else {
            out.push(c);
            col += crate::wrap::width(c.encode_utf8(&mut [0; 4]));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_info_strings() {
        assert_eq!(language("rust,ignore"), "rust");
        assert_eq!(language("python {.numberLines}"), "python");
        assert_eq!(language(""), "");
    }

    #[test]
    fn expands_tabs_to_stops() {
        assert_eq!(expand_tabs("a\tb"), "a   b");
        assert_eq!(expand_tabs("\tx"), "    x");
    }
}
