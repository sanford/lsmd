//! Big headings, in terminals that can draw text at twice its size: Kitty's
//! text sizing protocol (OSC 66).
//!
//! The renderer wraps a big heading at half the width, and leaves the line
//! under each of its lines blank for the bottom half. The heading is drawn
//! as ordinary text; then, at the end of the frame, each one that's wholly
//! on screen, with nothing drawn over it, is drawn again over those two
//! lines at twice the size. One that isn't (half scrolled off, under a
//! popup) stays as ordinary text.
//!
//! The terminal treats a big heading as one thing: writing any cell of it
//! clears all of it. So whenever the big headings on screen move, the
//! screen is drawn afresh rather than by what changed.

use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::Rect;
use std::cell::RefCell;
use std::io::{self, Write};
use std::num::NonZeroU16;
use std::sync::atomic::{AtomicBool, Ordering};
use unicode_width::UnicodeWidthStr;

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Lays out headings to be drawn big, from now on.
pub fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Whether the terminal can draw big text: after text sized to take two
/// columns, the cursor has moved two. Asked once the screen's ours, since
/// it prints. tmux doesn't pass the question on.
pub fn detect() -> bool {
    use ratatui::crossterm::cursor::position;
    if std::env::var_os("TMUX").is_some() || std::env::var("TERM").is_ok_and(|t| t == "dumb") {
        return false;
    }
    let probe = || -> io::Result<bool> {
        let mut out = io::stdout();
        write!(out, "\r")?;
        out.flush()?;
        let (before, _) = position()?;
        write!(out, "\x1b]66;w=2; \x1b\\")?;
        out.flush()?;
        let (after, _) = position()?;
        write!(out, "\r")?;
        out.flush()?;
        Ok(after == before + 2)
    };
    probe().unwrap_or(false)
}

/// A heading to draw big: where its text is, how many columns, and the
/// symbols of the two lines' cells it'll cover, as they were drawn.
struct Big {
    area: Rect,
    cols: u16,
    symbols: Vec<String>,
}

thread_local! {
    static PLACED: RefCell<Vec<Big>> = const { RefCell::new(Vec::new()) };
}

/// The two lines, twice `cols` wide, a big heading at (x, y) covers.
fn cover(x: u16, y: u16, cols: u16) -> Rect {
    Rect::new(x, y, cols.saturating_mul(2), 2)
}

fn symbols(buf: &Buffer, area: Rect) -> impl Iterator<Item = &str> {
    area.positions().map(move |p| buf[p].symbol())
}

/// Notes a heading drawn at (x, y), `cols` wide, with a blank line below
/// it, to draw big at the end of the frame.
pub fn place(buf: &Buffer, x: u16, y: u16, cols: u16) {
    if enabled() {
        note(buf, x, y, cols);
    }
}

fn note(buf: &Buffer, x: u16, y: u16, cols: u16) {
    let area = cover(x, y, cols);
    if cols == 0 || buf.area.intersection(area) != area {
        return;
    }
    let symbols = symbols(buf, area).map(str::to_string).collect();
    PLACED.with_borrow_mut(|p| {
        p.push(Big {
            area,
            cols,
            symbols,
        })
    });
}

/// Draws the headings noted this frame big, where nothing's been drawn
/// over them since. Returns where they are.
pub fn apply(buf: &mut Buffer) -> Vec<Rect> {
    let mut drawn = Vec::new();
    for big in PLACED.take() {
        let area = big.area;
        if buf.area.intersection(area) != area || !symbols(buf, area).eq(big.symbols.iter()) {
            continue;
        }
        let (x, y) = (area.x, area.y);
        // Each run of text in one style, sized, in that style. The
        // terminal's colors are left as the first cell's, which is what
        // ratatui thinks they are after drawing it.
        let mut esc = String::new();
        let mut run: Option<(ratatui::style::Style, String)> = None;
        let mut covered = 0;
        for i in 0..big.cols {
            let cell = &buf[(x + i, y)];
            if covered > 0 {
                // The right half of a wide character.
                covered -= 1;
                continue;
            }
            covered = cell.symbol().width().saturating_sub(1);
            match &mut run {
                Some((style, text)) if *style == cell.style() => text.push_str(cell.symbol()),
                _ => {
                    if let Some((style, text)) = run.take() {
                        sized(&mut esc, style, &text);
                    }
                    run = Some((cell.style(), cell.symbol().to_string()));
                }
            }
        }
        if let Some((style, text)) = run {
            sized(&mut esc, style, &text);
        }
        esc.push_str(&format!(
            "\x1b[0;{}m",
            crate::ansi::sgr(buf[(x, y)].style())
        ));
        for p in area.positions() {
            buf[p].set_diff_option(CellDiffOption::Skip);
        }
        buf[(x, y)]
            .set_symbol(&esc)
            .set_diff_option(CellDiffOption::ForcedWidth(NonZeroU16::MIN));
        drawn.push(area);
    }
    drawn
}

/// `text` twice the size, in `style`.
fn sized(esc: &mut String, style: ratatui::style::Style, text: &str) {
    let sgr = crate::ansi::sgr(style);
    esc.push_str(&format!("\x1b[0;{sgr}m\x1b]66;s=2;{text}\x1b\\"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Style;

    #[test]
    fn draws_what_is_left_of_it() {
        // Not enable(): the renderer's tests would lay out big headings.
        let mut buf = Buffer::empty(Rect::new(0, 0, 12, 3));
        buf.set_string(0, 0, "Big", Style::new().bold());
        buf.set_string(3, 0, "!", Style::new().red());
        note(&buf, 0, 0, 4);
        // Too near the bottom for its lower half: left alone.
        note(&buf, 0, 2, 4);
        let drawn = apply(&mut buf);
        assert_eq!(drawn, [Rect::new(0, 0, 8, 2)]);
        assert_eq!(
            buf[(0, 0)].symbol(),
            "\x1b[0;1m\x1b]66;s=2;Big\x1b\\\x1b[0;31m\x1b]66;s=2;!\x1b\\\x1b[0;1m"
        );
        assert_eq!(buf[(7, 1)].diff_option, CellDiffOption::Skip);
        assert_eq!(buf[(8, 1)].diff_option, CellDiffOption::None);

        // Covered by the time the frame's done: left as text.
        buf = Buffer::empty(Rect::new(0, 0, 12, 3));
        buf.set_string(0, 0, "Big", Style::new());
        note(&buf, 0, 0, 3);
        buf.set_string(4, 1, "│", Style::new());
        assert!(apply(&mut buf).is_empty());
        assert_eq!(buf[(0, 0)].symbol(), "B");
    }
}
