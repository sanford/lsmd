//! Printing rendered lines with ANSI escape codes, for non-interactive use.

use crate::render::RLine;
use ratatui::style::{Color, Modifier, Style};
use std::io::{self, Write};

pub fn print(lines: &[RLine], out: &mut impl Write) -> io::Result<()> {
    for line in lines {
        for span in &line.spans {
            // Rendering already made text printable; this is the last line
            // of defense before the terminal.
            let text = crate::safe::printable(&span.content);
            let sgr = sgr(span.style);
            if sgr.is_empty() {
                out.write_all(text.as_bytes())?;
            } else {
                write!(out, "\x1b[{sgr}m{text}\x1b[0m")?;
            }
        }
        out.write_all(b"\n")?;
    }
    out.flush()
}

/// The SGR parameters for `style`, e.g. "1;35".
pub(crate) fn sgr(style: Style) -> String {
    let mut codes: Vec<String> = Vec::new();
    let m = style.add_modifier;
    for (flag, code) in [
        (Modifier::BOLD, "1"),
        (Modifier::DIM, "2"),
        (Modifier::ITALIC, "3"),
        (Modifier::UNDERLINED, "4"),
        (Modifier::REVERSED, "7"),
        (Modifier::CROSSED_OUT, "9"),
    ] {
        if m.contains(flag) {
            codes.push(code.into());
        }
    }
    if let Some(fg) = style.fg.and_then(|c| color(c, 30)) {
        codes.push(fg);
    }
    if let Some(bg) = style.bg.and_then(|c| color(c, 40)) {
        codes.push(bg);
    }
    codes.join(";")
}

/// `base` is 30 for foreground and 40 for background.
fn color(c: Color, base: u8) -> Option<String> {
    let named = |i: u8| Some((base + i).to_string());
    let bright = |i: u8| Some((base + 60 + i).to_string());
    match c {
        Color::Reset => None,
        Color::Black => named(0),
        Color::Red => named(1),
        Color::Green => named(2),
        Color::Yellow => named(3),
        Color::Blue => named(4),
        Color::Magenta => named(5),
        Color::Cyan => named(6),
        Color::Gray => named(7),
        Color::DarkGray => bright(0),
        Color::LightRed => bright(1),
        Color::LightGreen => bright(2),
        Color::LightYellow => bright(3),
        Color::LightBlue => bright(4),
        Color::LightMagenta => bright(5),
        Color::LightCyan => bright(6),
        Color::White => bright(7),
        Color::Indexed(i) => Some(format!("{};5;{i}", base + 8)),
        Color::Rgb(r, g, b) => Some(format!("{};2;{r};{g};{b}", base + 8)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_sgr_codes() {
        assert_eq!(sgr(Style::new()), "");
        assert_eq!(sgr(Style::new().bold().magenta()), "1;35");
        assert_eq!(sgr(Style::new().bg(Color::Indexed(236))), "48;5;236");
        assert_eq!(sgr(Style::new().fg(Color::Rgb(1, 2, 3))), "38;2;1;2;3");
    }
}
