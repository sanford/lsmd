//! Colors and styles.
//!
//! Document chrome (headings, links, quotes) uses the terminal's own 16
//! colors, so it follows the user's palette. Code uses a syntax theme, which
//! needs exact colors; those fall back to the 256-color palette when the
//! terminal doesn't advertise 24-bit color. On Omarchy, the syntax theme and
//! the code background come from the desktop theme's palette.
//!
//! A theme picked by name paints everything in its palette's colors instead:
//! the terminal's 16 colors, its default foreground and its background are
//! swapped for the palette's as the screen is drawn.

use crate::palettes;
use omarchy_theme::Palette;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier, Style};
use std::str::FromStr;
use syntect::highlighting::Theme as SyntaxTheme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Auto,
    Dark,
    Light,
}

/// What `--theme` and `theme =` pick: dark or light code colors in the
/// terminal's own palette, or one of the bundled themes by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Mode(Mode),
    Named(&'static str),
}

impl FromStr for Choice {
    type Err = String;

    fn from_str(s: &str) -> Result<Choice, String> {
        Ok(match s {
            "auto" => Choice::Mode(Mode::Auto),
            "dark" => Choice::Mode(Mode::Dark),
            "light" => Choice::Mode(Mode::Light),
            _ => Choice::Named(palettes::find(s).ok_or_else(|| {
                let names: Vec<_> = palettes::names().collect();
                format!(
                    "unknown theme {s:?}: use auto, dark, light or one of {}",
                    names.join(", ")
                )
            })?),
        })
    }
}

impl Choice {
    /// As it's written in the config file.
    pub fn name(self) -> &'static str {
        match self {
            Choice::Mode(Mode::Auto) => "auto",
            Choice::Mode(Mode::Dark) => "dark",
            Choice::Mode(Mode::Light) => "light",
            Choice::Named(name) => name,
        }
    }
}

impl<'de> serde::Deserialize<'de> for Choice {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Choice, D::Error> {
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// A palette's colors for the terminal's: its 16 colors, and its default
/// foreground and background.
struct Paint {
    ansi: [Color; 16],
    fg: Color,
    bg: Color,
}

pub struct Theme {
    pub dark: bool,
    /// False for `--plain` and `NO_COLOR`: every style is empty.
    pub color: bool,
    truecolor: bool,
    pub code_bg: Option<Color>,
    /// The border of the pane with the keyboard: a bundled theme's accent.
    pub frame: Option<Color>,
    /// Code colors made from an Omarchy palette, instead of the built-in
    /// syntax theme.
    pub syntax: Option<SyntaxTheme>,
    paint: Option<Paint>,
}

impl Theme {
    /// With a `palette`, code takes its colors from it, and in `Auto` mode
    /// it says whether the theme is dark.
    pub fn new(mode: Mode, color: bool, palette: Option<&Palette>) -> Theme {
        let palette = palette.filter(|_| color);
        let dark = match (mode, palette) {
            (Mode::Dark, _) => true,
            (Mode::Light, _) => false,
            (Mode::Auto, Some(p)) => p.is_dark(),
            (Mode::Auto, None) => color && detect_dark(),
        };
        let truecolor = std::env::var("COLORTERM")
            .is_ok_and(|v| v.eq_ignore_ascii_case("truecolor") || v.eq_ignore_ascii_case("24bit"));
        let mut theme = Theme {
            dark,
            color,
            truecolor,
            code_bg: None,
            frame: None,
            syntax: None,
            paint: None,
        };
        if let Some(p) = palette {
            let bg = p.lighter_background();
            theme.code_bg = Some(theme.rgb(bg.r, bg.g, bg.b));
            theme.syntax = Some(crate::highlight::palette_theme(p));
        } else if color {
            theme.code_bg = Some(if dark {
                theme.rgb(0x2b, 0x2f, 0x37)
            } else {
                theme.rgb(0xee, 0xee, 0xee)
            });
        }
        theme
    }

    /// A theme with no colors or attributes, for tests and `--plain`.
    #[cfg(test)]
    pub fn plain() -> Theme {
        Theme {
            dark: true,
            color: false,
            truecolor: false,
            code_bg: None,
            frame: None,
            syntax: None,
            paint: None,
        }
    }

    /// The theme for `choice`, outside Omarchy.
    pub fn chosen(choice: Choice, color: bool) -> Theme {
        match choice {
            Choice::Named(name) => Theme::painted(&palettes::palette(name), color),
            Choice::Mode(mode) => Theme::new(mode, color, None),
        }
    }

    /// A bundled theme's colors for everything, not only code.
    pub fn painted(palette: &Palette, color: bool) -> Theme {
        let mut theme = Theme::new(Mode::Auto, color, Some(palette));
        if color {
            let accent = palette.accent();
            theme.frame = Some(theme.rgb(accent.r, accent.g, accent.b));
            let rgb = |c: omarchy_theme::Rgb| theme.rgb(c.r, c.g, c.b);
            theme.paint = Some(Paint {
                ansi: std::array::from_fn(|i| rgb(palette.ansi(i as u8))),
                fg: rgb(palette.foreground()),
                bg: rgb(palette.background()),
            });
        }
        theme
    }

    /// Swaps the terminal's colors on screen for a painted theme's.
    pub fn paint(&self, buf: &mut Buffer) {
        let Some(paint) = &self.paint else { return };
        for cell in &mut buf.content {
            cell.fg = paint.color(cell.fg, paint.fg);
            cell.bg = paint.color(cell.bg, paint.bg);
        }
    }

    /// Swaps the terminal's colors in `style` for a painted theme's, leaving
    /// the default foreground and background alone: for printing.
    pub fn recolor(&self, style: Style) -> Style {
        let Some(paint) = &self.paint else {
            return style;
        };
        Style {
            fg: style.fg.map(|c| paint.color(c, Color::Reset)),
            bg: style.bg.map(|c| paint.color(c, Color::Reset)),
            ..style
        }
    }

    fn s(&self, style: Style) -> Style {
        if self.color { style } else { Style::default() }
    }

    pub fn rgb(&self, r: u8, g: u8, b: u8) -> Color {
        if self.truecolor {
            Color::Rgb(r, g, b)
        } else {
            Color::Indexed(ansi256(r, g, b))
        }
    }

    pub fn heading(&self, level: u8) -> Style {
        let bold = Style::new().add_modifier(Modifier::BOLD);
        self.s(match level {
            1 | 2 => bold.fg(Color::Magenta),
            3 => bold.fg(Color::Cyan),
            _ => bold,
        })
    }

    pub fn dim(&self) -> Style {
        self.s(Style::new().add_modifier(Modifier::DIM))
    }

    pub fn link(&self) -> Style {
        self.s(Style::new()
            .fg(Color::Blue)
            .add_modifier(Modifier::UNDERLINED))
    }

    pub fn broken_link(&self) -> Style {
        self.s(Style::new()
            .fg(Color::Red)
            .add_modifier(Modifier::CROSSED_OUT))
    }

    pub fn inline_code(&self) -> Style {
        let fg = if self.dark { Color::Yellow } else { Color::Red };
        let style = Style::new().fg(fg);
        self.s(match self.code_bg {
            Some(bg) => style.bg(bg),
            None => style,
        })
    }

    pub fn code_block(&self) -> Style {
        self.s(match self.code_bg {
            Some(bg) => Style::new().bg(bg),
            None => Style::new(),
        })
    }

    pub fn quote(&self) -> Style {
        self.s(Style::new().fg(Color::Green))
    }

    pub fn bullet(&self) -> Style {
        self.s(Style::new().fg(Color::Cyan))
    }

    pub fn task_done(&self) -> Style {
        self.s(Style::new().fg(Color::Green))
    }

    pub fn border(&self) -> Style {
        self.dim()
    }

    pub fn table_header(&self) -> Style {
        self.s(Style::new().add_modifier(Modifier::BOLD))
    }

    pub fn key(&self) -> Style {
        self.s(Style::new().fg(Color::Cyan))
    }

    pub fn alert(&self, kind: comrak::nodes::AlertType) -> Style {
        use comrak::nodes::AlertType::*;
        let fg = match kind {
            Note => Color::Blue,
            Tip => Color::Green,
            Important => Color::Magenta,
            Warning => Color::Yellow,
            Caution => Color::Red,
        };
        self.s(Style::new().fg(fg))
    }

    /// Applies `modifier` (bold, italic, ...) only when styling is on.
    pub fn modifier(&self, style: Style, modifier: Modifier) -> Style {
        if self.color {
            style.add_modifier(modifier)
        } else {
            style
        }
    }
}

impl Paint {
    /// The palette's color for `c`, with `default` for the terminal's
    /// default color.
    fn color(&self, c: Color, default: Color) -> Color {
        let i = match c {
            Color::Reset => return default,
            Color::Black => 0,
            Color::Red => 1,
            Color::Green => 2,
            Color::Yellow => 3,
            Color::Blue => 4,
            Color::Magenta => 5,
            Color::Cyan => 6,
            Color::Gray => 7,
            Color::DarkGray => 8,
            Color::LightRed => 9,
            Color::LightGreen => 10,
            Color::LightYellow => 11,
            Color::LightBlue => 12,
            Color::LightMagenta => 13,
            Color::LightCyan => 14,
            Color::White => 15,
            Color::Indexed(i) if i < 16 => i,
            other => return other,
        };
        self.ansi[usize::from(i)]
    }
}

/// Asks the terminal for its background color. Assumes dark if it can't tell.
pub fn detect_dark() -> bool {
    use terminal_colorsaurus::{QueryOptions, ThemeMode, theme_mode};
    let mut options = QueryOptions::default();
    options.timeout = std::time::Duration::from_millis(150);
    !matches!(theme_mode(options), Ok(ThemeMode::Light))
}

/// Nearest color in the xterm 256-color cube or gray ramp.
fn ansi256(r: u8, g: u8, b: u8) -> u8 {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let nearest = |v: u8| {
        (0..6)
            .min_by_key(|&i| (LEVELS[i] as i32 - v as i32).abs())
            .unwrap() as u8
    };
    let (ri, gi, bi) = (nearest(r), nearest(g), nearest(b));
    let cube = 16 + 36 * ri + 6 * gi + bi;
    let cube_err = dist(
        (r, g, b),
        (
            LEVELS[ri as usize],
            LEVELS[gi as usize],
            LEVELS[bi as usize],
        ),
    );

    let avg = (r as u32 + g as u32 + b as u32) / 3;
    let gi = (avg.saturating_sub(8) / 10).min(23) as u8;
    let gv = 8 + 10 * gi;
    let gray_err = dist((r, g, b), (gv, gv, gv));

    if gray_err < cube_err { 232 + gi } else { cube }
}

fn dist(a: (u8, u8, u8), b: (u8, u8, u8)) -> i32 {
    let d = |x: u8, y: u8| (x as i32 - y as i32).pow(2);
    d(a.0, b.0) + d(a.1, b.1) + d(a.2, b.2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_modes_and_bundled_themes() {
        assert_eq!("light".parse(), Ok(Choice::Mode(Mode::Light)));
        assert_eq!("nord".parse(), Ok(Choice::Named("nord")));
        let e = "blue".parse::<Choice>().unwrap_err();
        assert!(e.contains("tokyo-night"), "{e}");
    }

    #[test]
    fn paints_the_terminal_colors() {
        let palette = palettes::palette("gruvbox");
        let theme = Theme::painted(&palette, true);
        let mut buf = Buffer::empty(ratatui::layout::Rect::new(0, 0, 2, 1));
        buf[(1, 0)]
            .set_fg(Color::Magenta)
            .set_bg(Color::Rgb(1, 2, 3));
        theme.paint(&mut buf);
        let rgb = |c: omarchy_theme::Rgb| theme.rgb(c.r, c.g, c.b);
        assert_eq!(buf[(0, 0)].fg, rgb(palette.foreground()));
        assert_eq!(buf[(0, 0)].bg, rgb(palette.background()));
        assert_eq!(buf[(1, 0)].fg, rgb(palette.magenta()));
        assert_eq!(buf[(1, 0)].bg, Color::Rgb(1, 2, 3), "exact colors stay");

        let plain = Theme::painted(&palette, false);
        assert_eq!(plain.recolor(Style::new().red()), Style::new().red());
    }

    #[test]
    fn maps_to_256_colors() {
        assert_eq!(ansi256(0, 0, 0), 16);
        assert_eq!(ansi256(255, 0, 0), 196);
        assert_eq!(ansi256(0x2b, 0x2f, 0x37), 236);
    }
}
