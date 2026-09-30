//! Pictures in a document, drawn in whatever way the terminal can:
//! Kitty's and iTerm2's images here, Sixel by ratatui-image. Mermaid
//! diagrams and image files (see `picture`).
//!
//! The renderer leaves blank lines for each picture, sized with [`cells`];
//! the app draws it over them.

use image::DynamicImage;
use image::imageops::FilterType;
use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::{Rect, Size};
use ratatui::style::Color;
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
    use ratatui_image::picker::Capability;
    use ratatui_image::picker::cap_parser::QueryStdioOptions;
    if std::env::var_os("TMUX").is_some() {
        return None;
    }
    let mut picker = Picker::from_query_stdio_with_options(QueryStdioOptions {
        timeout: Duration::from_millis(250),
        ..QueryStdioOptions::default()
    })
    .ok()?;
    // A terminal that names itself in TERM is that one: Kitty, say,
    // started from iTerm2 inherits iTerm2's TERM_PROGRAM and LC_TERMINAL.
    // What it said it can do stands.
    if own_term() {
        return ready(picker);
    }
    // Which terminal this is. iTerm2's variables outlive it: a terminal
    // started from iTerm2 (Terminal.app, say) inherits its LC_TERMINAL, so
    // that only counts when nothing else says (over ssh, say).
    let program = std::env::var("TERM_PROGRAM").ok();
    let iterm = match &program {
        Some(p) => p.contains("iTerm"),
        None => std::env::var("LC_TERMINAL").is_ok_and(|v| v.contains("iTerm")),
    };
    if iterm {
        // iTerm2 says it can do Sixel too, and that's what gets picked. Newer
        // ones draw Kitty's, sent once and then only placed, so they move
        // with the text; in older ones, its own are what it draws best.
        let kitty = picker.capabilities().contains(&Capability::Kitty);
        picker.set_protocol_type(if kitty {
            ProtocolType::Kitty
        } else {
            ProtocolType::Iterm2
        });
    } else if picker.protocol_type() == ProtocolType::Iterm2
        && program.is_some_and(|p| !ITERM_LIKE.iter().any(|t| p.contains(t)))
    {
        // Guessed from iTerm2's leftover variables, in a terminal that says
        // it's something else.
        return None;
    }
    ready(picker)
}

/// The picker, unless it'd draw in half blocks, which can't draw a
/// diagram's labels legibly: better the text.
fn ready(picker: Picker) -> Option<Picker> {
    if picker.protocol_type() == ProtocolType::Halfblocks {
        return None;
    }
    let size = picker.font_size();
    set_cell(size.width, size.height);
    Some(picker)
}

/// Whether the terminal says which it is in TERM, or as Kitty does, a
/// variable of its own: then TERM_PROGRAM may be another's, inherited.
fn own_term() -> bool {
    let term = std::env::var("TERM").unwrap_or_default();
    ["kitty", "ghostty", "wezterm"]
        .iter()
        .any(|t| term.contains(t))
        || std::env::var_os("KITTY_WINDOW_ID").is_some()
}

/// Terminals that draw iTerm2's pictures, by what they call themselves in
/// `TERM_PROGRAM`: those ratatui-image counts.
const ITERM_LIKE: &[&str] = &[
    "iTerm",
    "WezTerm",
    "mintty",
    "vscode",
    "Tabby",
    "Hyper",
    "rio",
    "Bobcat",
    "WarpTerminal",
];

/// A picture made ready to draw at one size.
pub enum Drawn {
    /// Kitty's (newer iTerm2s draw them too): sent once, then shown by a
    /// character in each cell it covers, so it moves with the text like any
    /// other. Only a row's first cell says which row it is, the rest follow
    /// on from it, so a step of a scroll rewrites a cell a row.
    Placed {
        id: u32,
        cols: usize,
        rows: usize,
        /// The picture, till it's been sent.
        send: Option<String>,
    },
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
        let cell = picker.font_size();
        let (cw, ch) = (cell.width.into(), cell.height.into());
        if picker.protocol_type() == ProtocolType::Kitty && rows <= DIACRITICS.len() {
            let id = kitty_id();
            return Some(Drawn::Placed {
                id,
                cols,
                rows,
                send: Some(kitty_send(picture, cols, rows, cw, ch, id)?),
            });
        }
        if picker.protocol_type() == ProtocolType::Iterm2 {
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

    /// What's to be written to the terminal before it can be shown, once.
    pub fn take_send(&mut self) -> Option<String> {
        match self {
            Drawn::Placed { send, .. } => send.take(),
            _ => None,
        }
    }

    /// What frees it in the terminal, once it's no longer wanted.
    pub fn forget(&self) -> Option<String> {
        match self {
            Drawn::Placed { id, .. } => Some(format!("\x1b_Ga=d,d=I,i={id},q=2\x1b\\")),
            _ => None,
        }
    }

    /// Draws it `x` columns into `area` and `y` rows down, which is above
    /// the top once it's scrolled partly off; `soft` while it's moving.
    pub fn draw(&self, buf: &mut Buffer, area: Rect, x: u16, y: i32, soft: bool) {
        match self {
            Drawn::Placed { id, cols, rows, .. } => {
                let [_, r, g, b] = id.to_be_bytes();
                let left = area.x + x;
                let cols = (*cols as u16).min(area.right().saturating_sub(left));
                for (row, &mark) in DIACRITICS[..*rows].iter().enumerate() {
                    let at = y + row as i32;
                    if at < 0 || at >= i32::from(area.height) {
                        continue;
                    }
                    let top = area.y + at as u16;
                    for c in 0..cols {
                        let cell = &mut buf[(left + c, top)];
                        if c == 0 {
                            let mut first = String::from(PLACEHOLDER);
                            first.extend([mark, DIACRITICS[0]]);
                            cell.set_symbol(&first);
                        } else {
                            cell.set_char(PLACEHOLDER);
                        }
                        cell.set_fg(Color::Rgb(r, g, b))
                            .set_diff_option(CellDiffOption::ForcedWidth(NonZeroU16::MIN));
                    }
                }
            }
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

/// Stands for a cell of a Kitty picture.
const PLACEHOLDER: char = '\u{10EEEE}';

/// Marks that follow it to say which row and column of the picture it is:
/// the first of Kitty's, enough for the most rows a picture takes.
const DIACRITICS: [char; 48] = [
    '\u{305}', '\u{30D}', '\u{30E}', '\u{310}', '\u{312}', '\u{33D}', '\u{33E}', '\u{33F}',
    '\u{346}', '\u{34A}', '\u{34B}', '\u{34C}', '\u{350}', '\u{351}', '\u{352}', '\u{357}',
    '\u{35B}', '\u{363}', '\u{364}', '\u{365}', '\u{366}', '\u{367}', '\u{368}', '\u{369}',
    '\u{36A}', '\u{36B}', '\u{36C}', '\u{36D}', '\u{36E}', '\u{36F}', '\u{483}', '\u{484}',
    '\u{485}', '\u{486}', '\u{487}', '\u{592}', '\u{593}', '\u{594}', '\u{595}', '\u{597}',
    '\u{598}', '\u{599}', '\u{59C}', '\u{59D}', '\u{59E}', '\u{59F}', '\u{5A0}', '\u{5A1}',
];
const _: () = assert!(MAX_ROWS <= DIACRITICS.len());

/// A new number for a Kitty picture, which its cells carry as their
/// colour: never 0, and 24 bits, so that's all it takes. Starting from the
/// process's, so pictures left from another run aren't taken for these.
fn kitty_id() -> u32 {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    (std::process::id().wrapping_mul(7919).wrapping_add(n) & 0xFF_FFFF).max(1)
}

/// Kitty's escapes sending the picture at `cols`×`rows` cells, as `id`,
/// to be shown wherever its cells are. That size is given, not left to be
/// worked out from the picture's as Kitty does, since iTerm2 doesn't.
fn kitty_send(
    picture: &DynamicImage,
    cols: usize,
    rows: usize,
    cw: u32,
    ch: u32,
    id: u32,
) -> Option<String> {
    use base64::Engine;
    let scaled = picture.resize(cols as u32 * cw, rows as u32 * ch, FilterType::Triangle);
    let mut bytes = Vec::new();
    scaled
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .ok()?;
    let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
    // In pieces of at most 4096 characters, each saying if more follow.
    let pieces: Vec<&[u8]> = data.as_bytes().chunks(4096).collect();
    let mut out = String::with_capacity(data.len() + pieces.len() * 16 + 64);
    for (i, piece) in pieces.iter().enumerate() {
        let more = u8::from(i + 1 < pieces.len());
        out.push_str("\x1b_G");
        if i == 0 {
            out.push_str(&format!("a=T,U=1,f=100,i={id},c={cols},r={rows},"));
        }
        out.push_str(&format!("q=2,m={more};"));
        out.push_str(std::str::from_utf8(piece).ok()?);
        out.push_str("\x1b\\");
    }
    Some(out)
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
