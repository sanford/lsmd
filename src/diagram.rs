//! Mermaid diagrams. A ```` ```mermaid ```` block is laid out by
//! mermaid-rs-renderer and drawn to pixels by resvg, on `picture`'s
//! background thread, then shown as a picture where the terminal can draw
//! one. Until it's ready, and where it can't, the block shows as code.
//! SVG image files are drawn to pixels here too.

use image::{DynamicImage, RgbaImage};
use resvg::{tiny_skia, usvg};
use std::sync::{Arc, OnceLock};

/// Pixels drawn for each of the SVG's, for sharp text on high-density
/// screens; it's scaled to the cells it's given.
const SCALE: f32 = 2.0;
/// The most pixels a side, however big the SVG.
const LARGEST: f32 = 4096.0;

/// The colors a diagram is drawn in, from the document's theme.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Look {
    pub dark: bool,
    /// Behind it: the code blocks' background, if the theme has one.
    pub background: Option<[u8; 3]>,
}

pub fn is_mermaid(info: &str) -> bool {
    info.split_whitespace()
        .next()
        .is_some_and(|lang| lang.eq_ignore_ascii_case("mermaid"))
}

/// The system's fonts, loaded once, for the diagrams' text.
fn fonts() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = usvg::fontdb::Database::new();
            db.load_system_fonts();
            Arc::new(db)
        })
        .clone()
}

/// Draws `source`: the picture, and its size in the renderer's pixels.
pub fn draw(source: &str, look: Look) -> Result<(DynamicImage, u32, u32), String> {
    use mermaid_rs_renderer::{RenderOptions, Theme};
    let mut theme = if look.dark {
        Theme::dark()
    } else {
        Theme::modern()
    };
    if let Some([r, g, b]) = look.background {
        theme.background = format!("#{r:02x}{g:02x}{b:02x}");
    }
    let background = hex(&theme.background);
    let font = theme
        .font_family
        .split(',')
        .map(|s| s.trim().trim_matches(['"', '\'']))
        .find(|s| !s.is_empty())
        .unwrap_or("sans-serif")
        .to_string();
    let options = RenderOptions {
        theme,
        ..RenderOptions::default()
    };
    let svg = mermaid_rs_renderer::render_with_options(source, options)
        .map_err(|e| first_line(&e.to_string()))?;
    rasterize(svg.as_bytes(), font, background)
}

/// Draws an SVG file, on a transparent background.
pub fn draw_svg(data: &[u8]) -> Result<(DynamicImage, u32, u32), String> {
    rasterize(data, "sans-serif".into(), None)
}

/// Draws SVG `data` at [`SCALE`], text in `font` unless it says otherwise,
/// over `background`: the picture, and the SVG's own size.
fn rasterize(
    data: &[u8],
    font: String,
    background: Option<tiny_skia::Color>,
) -> Result<(DynamicImage, u32, u32), String> {
    let options = usvg::Options {
        font_family: font,
        fontdb: fonts(),
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_data(data, &options).map_err(|e| e.to_string())?;
    let size = tree.size();
    let scale = SCALE.min(LARGEST / size.width().max(size.height()).max(1.0));
    let (w, h) = (
        (size.width() * scale).ceil() as u32,
        (size.height() * scale).ceil() as u32,
    );
    let mut pixmap = tiny_skia::Pixmap::new(w.max(1), h.max(1)).ok_or("too big to draw")?;
    if let Some(color) = background {
        pixmap.fill(color);
    }
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let pixels: Vec<u8> = pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    let picture = RgbaImage::from_raw(pixmap.width(), pixmap.height(), pixels)
        .ok_or("couldn't make the picture")?;
    Ok((
        DynamicImage::ImageRgba8(picture),
        size.width().ceil() as u32,
        size.height().ceil() as u32,
    ))
}

/// `#rrggbb` as a color.
fn hex(s: &str) -> Option<tiny_skia::Color> {
    let s = s.strip_prefix('#').filter(|s| s.len() == 6)?;
    let n = u32::from_str_radix(s, 16).ok()?;
    Some(tiny_skia::Color::from_rgba8(
        (n >> 16) as u8,
        (n >> 8) as u8,
        n as u8,
        255,
    ))
}

/// The first line of an error, which is what's worth showing.
fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or(s).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_a_diagram_at_twice_its_size() {
        let look = Look {
            dark: true,
            background: Some([0x20, 0x20, 0x20]),
        };
        let (picture, w, h) = draw("flowchart LR; A-->B", look).unwrap();
        assert!(w > 50 && h > 20, "{w}×{h}");
        // Its own size is rounded up; the picture's, from the exact size.
        assert!(picture.width().abs_diff((w as f32 * SCALE) as u32) <= 2);
        // The background is the one asked for.
        let corner = picture.to_rgba8().get_pixel(0, 0).0;
        assert_eq!(corner, [0x20, 0x20, 0x20, 255]);
    }

    #[test]
    fn says_what_went_wrong() {
        let look = Look {
            dark: false,
            background: None,
        };
        assert!(draw("not a diagram at all", look).is_err());
    }

    #[test]
    fn knows_mermaid_blocks() {
        assert!(is_mermaid("mermaid"));
        assert!(is_mermaid("Mermaid title"));
        assert!(!is_mermaid("rust"));
        assert!(!is_mermaid(""));
    }
}
