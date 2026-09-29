//! Pictures in a document, drawn by ratatui-image in whatever way the
//! terminal can: iTerm2's images, Kitty's, or Sixel: Mermaid diagrams and
//! image files (see `picture`).
//!
//! The renderer leaves blank lines for each picture, sized with [`cells`];
//! the app draws it over them.

use image::DynamicImage;
use image::imageops::FilterType;
use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::{Rect, Size};
use ratatui::widgets::Widget;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::sliced::{SignedPosition, SlicedImage, SlicedProtocol};
use std::num::NonZeroU16;
use std::sync::OnceLock;
use std::time::Duration;

/// The most rows a picture may take.
pub const MAX_ROWS: usize = 40;

/// The terminal's character cell, in pixels, once it's been asked.
static CELL: OnceLock<(u16, u16)> = OnceLock::new();

pub fn set_cell(width: u16, height: u16) {
    if width > 0 && height > 0 {
        let _ = CELL.set((width, height));
    }
}

fn cell() -> (u16, u16) {
    CELL.get().copied().unwrap_or((10, 20))
}

/// How the terminal can draw pictures, and its cell size in pixels: `None`
/// if it can't (or won't say), when they're left as text. Asked after the
/// screen's ours and before reading keys, and briefly: a terminal that
/// doesn't answer holds up the start, and whatever's typed meanwhile is
/// lost. tmux passes on neither the question nor pictures unless set up
/// to, so there it isn't asked.
pub fn picker() -> Option<Picker> {
    use ratatui_image::picker::cap_parser::QueryStdioOptions;
    if std::env::var_os("TMUX").is_some() {
        return None;
    }
    let mut picker = Picker::from_query_stdio_with_options(QueryStdioOptions {
        timeout: Duration::from_millis(250),
        ..QueryStdioOptions::default()
    })
    .ok()?;
    // iTerm2 says it can do Sixel too, and that's what gets picked, but
    // its own pictures are what it draws best.
    let iterm = |var| std::env::var(var).is_ok_and(|v| v.contains("iTerm"));
    if iterm("TERM_PROGRAM") || iterm("LC_TERMINAL") {
        picker.set_protocol_type(ProtocolType::Iterm2);
    }
    // Half blocks can't draw a diagram's labels legibly: better the text.
    if picker.protocol_type() == ProtocolType::Halfblocks {
        return None;
    }
    let size = picker.font_size();
    set_cell(size.width, size.height);
    Some(picker)
}

/// A picture made ready to draw at one size.
pub enum Drawn {
    /// iTerm2: a picture per row, so it can scroll partly off screen.
    /// `soft` is the same at a sixteenth of the resolution, for while it's
    /// moving: the rows are sent again at each step of a scroll.
    Rows {
        cols: usize,
        rows: Vec<String>,
        soft: Vec<String>,
    },
    /// Anything else, as ratatui-image does it.
    Sliced(SlicedProtocol),
}

impl Drawn {
    pub fn new(picker: &Picker, picture: &DynamicImage, cols: usize, rows: usize) -> Option<Drawn> {
        if picker.protocol_type() == ProtocolType::Iterm2 {
            let cell = picker.font_size();
            let (cw, ch) = (cell.width.into(), cell.height.into());
            return Some(Drawn::Rows {
                cols,
                rows: iterm_rows(picture, cols, rows, cw, ch, 1)?,
                soft: iterm_rows(picture, cols, rows, cw, ch, SOFTER)?,
            });
        }
        let size = Size::new(cols as u16, rows as u16);
        SlicedProtocol::new(picker, picture.clone(), Some(size))
            .ok()
            .map(Drawn::Sliced)
    }

    /// Whether there's a lighter version to show while it moves.
    pub fn has_soft(&self) -> bool {
        matches!(self, Drawn::Rows { .. })
    }

    /// Draws it `x` columns into `area` and `y` rows down, which is above
    /// the top once it's scrolled partly off; `soft` while it's moving.
    pub fn draw(&self, buf: &mut Buffer, area: Rect, x: u16, y: i32, soft: bool) {
        match self {
            Drawn::Rows {
                cols,
                rows,
                soft: light,
            } => {
                let rows = if soft { light } else { rows };
                let left = area.x + x;
                let cols = (*cols as u16).min(area.right().saturating_sub(left));
                for (i, row) in rows.iter().enumerate() {
                    let at = y + i as i32;
                    if at < 0 || at >= i32::from(area.height) || cols == 0 {
                        continue;
                    }
                    let top = area.y + at as u16;
                    buf[(left, top)]
                        .set_symbol(row)
                        .set_diff_option(CellDiffOption::ForcedWidth(NonZeroU16::MIN));
                    for c in 1..cols {
                        buf[(left + c, top)].set_diff_option(CellDiffOption::Skip);
                    }
                }
            }
            Drawn::Sliced(protocol) => {
                let area = Rect {
                    x: area.x + x,
                    width: area.width.saturating_sub(x),
                    ..area
                };
                let y = y.clamp(i16::MIN.into(), i16::MAX.into()) as i16;
                SlicedImage::new(protocol, SignedPosition::from((0, y))).render(area, buf);
            }
        }
    }
}

/// How much less detail the picture has while it moves, each way.
const SOFTER: u32 = 16;

/// iTerm2's escape for each row of the picture at `cols`×`rows` cells,
/// each clearing its row first. With `less` over 1, each row has that
/// much less detail each way, and iTerm2 stretches it to fit.
fn iterm_rows(
    picture: &DynamicImage,
    cols: usize,
    rows: usize,
    cw: u32,
    ch: u32,
    less: u32,
) -> Option<Vec<String>> {
    use base64::Engine;
    let scaled = picture.resize(cols as u32 * cw, rows as u32 * ch, FilterType::Triangle);
    let mut out = Vec::new();
    let mut y = 0;
    while y < scaled.height() {
        let h = ch.min(scaled.height() - y);
        let (w, row) = (scaled.width(), scaled.crop_imm(0, y, scaled.width(), h));
        let row = if less > 1 {
            row.resize_exact((w / less).max(1), (h / less).max(1), FilterType::Triangle)
        } else {
            row
        };
        // PNG: diagrams are flat colors and sharp edges, which it keeps
        // small and JPEG smears.
        let mut bytes = Vec::new();
        row.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .ok()?;
        out.push(format!(
            "\x1b[{cols}X\x1b]1337;File=inline=1;size={};width={w}px;height={h}px;preserveAspectRatio=0;doNotMoveCursor=1:{}\x07",
            bytes.len(),
            base64::engine::general_purpose::STANDARD.encode(&bytes),
        ));
        y += h;
    }
    Some(out)
}

/// How many columns and rows a `w`×`h` picture takes, at most `cols` wide
/// and `rows` high: its own size if that fits, never bigger.
pub fn cells(w: u32, h: u32, cols: usize, rows: usize) -> (usize, usize) {
    let (cw, ch) = cell();
    let (cw, ch) = (f64::from(cw), f64::from(ch));
    let (w, h) = (f64::from(w), f64::from(h));
    let mut c = (w / cw).ceil().min(cols as f64);
    let mut r = (c * cw * h / w / ch).ceil();
    if r > rows as f64 {
        r = rows as f64;
        c = (r * ch * w / h / cw).floor();
    }
    (c.max(1.0) as usize, r.max(1.0) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_fit_the_width_and_rows() {
        // 10×20 cells: a 1600×900 picture at 80 columns is 23 rows, over 12.
        assert_eq!(cells(1600, 900, 80, 12), (42, 12));
        assert_eq!(cells(1600, 900, 80, 30), (80, 23));
        // Small ones stay their size.
        assert_eq!(cells(200, 100, 80, 30), (20, 5));
    }
}
