//! Syntax highlighting for code blocks.

use crate::theme::Theme;
use omarchy_theme::{Palette, Rgb};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use std::sync::OnceLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::{
    Color, FontStyle, ScopeSelectors, StyleModifier, Theme as SyntaxTheme, ThemeItem, ThemeSettings,
};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;
use two_face::theme::EmbeddedThemeName;

/// The syntax definitions take a few milliseconds to load, so they're only
/// loaded for documents that have code in them.
fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(two_face::syntax::extra_newlines)
}

fn syntax_theme(theme: &Theme) -> &SyntaxTheme {
    theme
        .syntax
        .as_ref()
        .unwrap_or_else(|| builtin_theme(theme.dark))
}

fn builtin_theme(dark: bool) -> &'static SyntaxTheme {
    static DARK: OnceLock<SyntaxTheme> = OnceLock::new();
    static LIGHT: OnceLock<SyntaxTheme> = OnceLock::new();
    let (cell, name) = if dark {
        (&DARK, EmbeddedThemeName::OneHalfDark)
    } else {
        (&LIGHT, EmbeddedThemeName::OneHalfLight)
    };
    cell.get_or_init(|| two_face::theme::extra().get(name).clone())
}

/// A syntax theme in an Omarchy palette's colors, scope for scope what
/// Omarchy's own Helix template picks.
pub fn palette_theme(p: &Palette) -> SyntaxTheme {
    let italic = FontStyle::ITALIC;
    let bold = FontStyle::BOLD;
    let none = FontStyle::empty();
    let rules: [(&str, Rgb, FontStyle); 29] = [
        ("comment", p.ansi(8), italic),
        // Quotes and comment markers go with what they mark.
        (
            "punctuation - punctuation.definition.string - punctuation.definition.comment",
            p.ansi(8),
            none,
        ),
        ("keyword, storage", p.magenta(), none),
        ("keyword.control", p.magenta(), italic),
        ("keyword.operator", p.cyan(), none),
        ("string", p.green(), none),
        ("string.regexp", p.magenta(), none),
        ("constant", p.yellow(), none),
        ("constant.character", p.cyan(), none),
        ("constant.character.escape", p.magenta(), none),
        (
            "entity.name.function, support.function, meta.function-call",
            p.blue(),
            none,
        ),
        (
            "entity.name.type, entity.name.class, entity.name.struct, entity.name.enum",
            p.yellow(),
            none,
        ),
        ("support.type, support.class", p.yellow(), none),
        (
            "entity.name.namespace, entity.name.module",
            p.yellow(),
            italic,
        ),
        ("variable.parameter", p.magenta(), italic),
        ("variable.language", p.red(), none),
        ("variable.other.member", p.blue(), none),
        ("entity.name.tag", p.blue(), none),
        ("entity.other.attribute-name", p.yellow(), none),
        ("markup.heading", p.red(), bold),
        (
            "punctuation.definition.list_item, punctuation.definition.list",
            p.cyan(),
            none,
        ),
        ("markup.bold", p.red(), bold),
        ("markup.italic", p.red(), italic),
        ("markup.raw", p.green(), none),
        ("markup.quote", p.magenta(), none),
        ("markup.underline.link", p.blue(), italic),
        ("markup.inserted", p.green(), none),
        ("markup.deleted", p.red(), none),
        ("markup.changed", p.blue(), none),
    ];
    let color = |c: Rgb| Color {
        r: c.r,
        g: c.g,
        b: c.b,
        a: 0xff,
    };
    SyntaxTheme {
        name: Some("omarchy".to_string()),
        settings: ThemeSettings {
            foreground: Some(color(p.foreground())),
            ..ThemeSettings::default()
        },
        scopes: rules
            .into_iter()
            .map(|(scope, fg, font_style)| ThemeItem {
                scope: scope.parse::<ScopeSelectors>().expect("valid selector"),
                style: StyleModifier {
                    foreground: Some(color(fg)),
                    background: None,
                    font_style: Some(font_style),
                },
            })
            .collect(),
        ..SyntaxTheme::default()
    }
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
    let mut hl = HighlightLines::new(syntax, syntax_theme(theme));
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
    fn colors_code_from_an_omarchy_palette() {
        let palette = Palette::parse(
            "background = \"#1a1b26\"\nforeground = \"#a9b1d6\"\nred = \"#f7768e\"\n\
             green = \"#9ece6a\"\nyellow = \"#e0af68\"\nblue = \"#7aa2f7\"\n\
             magenta = \"#ad8ee6\"\ncyan = \"#449dab\"\nlighter_background = \"#24283b\"\n",
        )
        .unwrap();
        let theme = Theme::new(crate::theme::Mode::Auto, true, Some(&palette));
        assert!(theme.dark);
        let rgb = |c: Rgb| theme.rgb(c.r, c.g, c.b);
        assert_eq!(theme.code_bg, Some(rgb(palette.lighter_background())));

        let line = &highlight(
            "let s = \"hi\"; // note\n",
            "rust",
            &theme,
            Style::default(),
        )[0];
        let fg = |text: &str| {
            line.iter()
                .find(|s| s.content.contains(text))
                .and_then(|s| s.style.fg)
        };
        assert_eq!(fg("let"), Some(rgb(palette.magenta())));
        assert_eq!(fg("hi"), Some(rgb(palette.green())));
        assert_eq!(fg("note"), Some(rgb(palette.ansi(8))));
    }

    #[test]
    fn expands_tabs_to_stops() {
        assert_eq!(expand_tabs("a\tb"), "a   b");
        assert_eq!(expand_tabs("\tx"), "    x");
    }
}
