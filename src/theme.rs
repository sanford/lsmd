//! Colors and styles.
//!
//! Document chrome (headings, links, quotes) uses the terminal's own 16
//! colors, so it follows the user's palette. Code uses a syntax theme, which
//! needs exact colors; those fall back to the 256-color palette when the
//! terminal doesn't advertise 24-bit color.

use ratatui::style::{Color, Modifier, Style};

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Mode {
    Auto,
    Dark,
    Light,
}

pub struct Theme {
    pub dark: bool,
    /// False for `--plain` and `NO_COLOR`: every style is empty.
    pub color: bool,
    truecolor: bool,
    pub code_bg: Option<Color>,
}

impl Theme {
    pub fn new(mode: Mode, color: bool) -> Theme {
        let dark = match mode {
            Mode::Dark => true,
            Mode::Light => false,
            Mode::Auto => color && detect_dark(),
        };
        let truecolor = std::env::var("COLORTERM")
            .is_ok_and(|v| v.eq_ignore_ascii_case("truecolor") || v.eq_ignore_ascii_case("24bit"));
        let mut theme = Theme {
            dark,
            color,
            truecolor,
            code_bg: None,
        };
        if color {
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
        self.s(Style::new().fg(Color::Blue).add_modifier(Modifier::UNDERLINED))
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
        if self.color { style.add_modifier(modifier) } else { style }
    }
}

/// Asks the terminal for its background color. Assumes dark if it can't tell.
fn detect_dark() -> bool {
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
    let cube_err = dist((r, g, b), (LEVELS[ri as usize], LEVELS[gi as usize], LEVELS[bi as usize]));

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
    use super::ansi256;

    #[test]
    fn maps_to_256_colors() {
        assert_eq!(ansi256(0, 0, 0), 16);
        assert_eq!(ansi256(255, 0, 0), 196);
        assert_eq!(ansi256(0x2b, 0x2f, 0x37), 236);
    }
}
