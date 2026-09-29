//! Pictures to show in documents: Mermaid diagrams (see `diagram`) and
//! image files the document points to. Each is made ready once, on a
//! background thread, and kept; documents ask for them as they're laid
//! out, show text until they're ready, and are laid out again when they
//! are. None of this happens unless the terminal can draw pictures.

use crate::diagram::Look;
use image::DynamicImage;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};

/// Image files are shrunk to fit this, so drawing them doesn't wait on
/// scaling a photo from a camera.
const LARGEST: u32 = 2048;

/// A picture ready to draw.
pub struct Picture {
    /// Which it is, for keeping its drawn versions.
    pub key: u64,
    pub image: DynamicImage,
    /// Its own size in pixels, which may be more than `image`'s.
    pub width: u32,
    pub height: u32,
}

/// What's known of a picture.
#[derive(Clone)]
pub enum State {
    Making,
    Ready(Arc<Picture>),
    Failed(String),
}

enum Job {
    Diagram(String, Look),
    File(PathBuf),
}

/// Whether pictures are made at all: only once the terminal's said it can
/// show them.
static ENABLED: AtomicBool = AtomicBool::new(false);
static PICTURES: LazyLock<Mutex<HashMap<u64, State>>> = LazyLock::new(Default::default);
static WORKER: OnceLock<Mutex<Sender<(u64, Job)>>> = OnceLock::new();
static MAKING: AtomicUsize = AtomicUsize::new(0);
/// A picture has been made (or failed) since [`take_news`] last asked.
static NEWS: AtomicBool = AtomicBool::new(false);

pub fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
}

/// The Mermaid diagram `source` drawn in `look`, or `None` if pictures
/// aren't shown. Asking for one not yet made starts making it.
pub fn diagram(source: &str, look: Look) -> Option<State> {
    let key = key(&(0u8, source, look));
    get(key, || Job::Diagram(source.to_string(), look))
}

/// The image file at `path`, as it is now, or `None` if pictures aren't
/// shown.
pub fn file(path: &Path) -> Option<State> {
    // Changed since, it's a new picture.
    let meta = std::fs::metadata(path).ok();
    let stamp = meta.as_ref().map(|m| (m.len(), m.modified().ok()));
    let key = key(&(1u8, path, stamp));
    get(key, || Job::File(path.to_path_buf()))
}

fn key(what: &impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    what.hash(&mut hasher);
    hasher.finish()
}

fn get(key: u64, job: impl FnOnce() -> Job) -> Option<State> {
    if !ENABLED.load(Ordering::Relaxed) {
        return None;
    }
    let mut pictures = PICTURES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(state) = pictures.get(&key) {
        return Some(state.clone());
    }
    pictures.insert(key, State::Making);
    MAKING.fetch_add(1, Ordering::Relaxed);
    let worker = WORKER.get_or_init(|| Mutex::new(start()));
    let _ = worker
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .send((key, job()));
    Some(State::Making)
}

/// Whether any picture is still being made.
pub fn making() -> bool {
    MAKING.load(Ordering::Relaxed) > 0
}

/// Whether pictures have been made since last asked, so documents with
/// them should be laid out again.
pub fn take_news() -> bool {
    NEWS.swap(false, Ordering::Relaxed)
}

/// The thread that makes pictures, one at a time.
fn start() -> Sender<(u64, Job)> {
    let (tx, rx) = mpsc::channel::<(u64, Job)>();
    std::thread::spawn(move || {
        for (key, job) in rx {
            let made = match job {
                Job::Diagram(source, look) => crate::diagram::draw(&source, look),
                Job::File(path) => load(&path),
            };
            let state = match made {
                Ok((image, width, height)) => State::Ready(Arc::new(Picture {
                    key,
                    image,
                    width,
                    height,
                })),
                Err(e) => State::Failed(e),
            };
            PICTURES
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(key, state);
            MAKING.fetch_sub(1, Ordering::Relaxed);
            NEWS.store(true, Ordering::Relaxed);
        }
    });
    tx
}

/// Reads and decodes an image file: the picture, and its size in pixels.
/// SVGs are drawn as diagrams are; GIFs show their first frame.
fn load(path: &Path) -> Result<(DynamicImage, u32, u32), String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() > crate::files::BACKGROUND_LIMIT {
        return Err("too big".into());
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let svg = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("svg") || e.eq_ignore_ascii_case("svgz"));
    if svg {
        return crate::diagram::draw_svg(&bytes);
    }
    let image = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
    let (width, height) = (image.width(), image.height());
    let image = if width > LARGEST || height > LARGEST {
        image.thumbnail(LARGEST, LARGEST)
    } else {
        image
    };
    Ok((image, width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_image_files_and_says_what_went_wrong() {
        let dir = std::env::temp_dir().join(format!("lsmd-picture-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("a.png");
        image::RgbaImage::from_pixel(300, 100, image::Rgba([200, 0, 0, 255]))
            .save(&png)
            .unwrap();
        let (image, w, h) = load(&png).unwrap();
        assert_eq!((w, h), (300, 100));
        assert_eq!(image.width(), 300);

        // Big ones are shrunk, keeping their own size.
        let big = dir.join("big.png");
        image::RgbaImage::new(4096, 1024).save(&big).unwrap();
        let (image, w, _) = load(&big).unwrap();
        assert_eq!(w, 4096);
        assert_eq!(image.width(), LARGEST);

        let svg = dir.join("c.svg");
        std::fs::write(
            &svg,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="blue"/></svg>"#,
        )
        .unwrap();
        let (image, w, h) = load(&svg).unwrap();
        assert_eq!((w, h), (40, 20));
        assert_eq!(image.width(), 80, "drawn at twice its size");

        let text = dir.join("not.png");
        std::fs::write(&text, "not a picture").unwrap();
        assert!(load(&text).is_err());
        assert!(load(&dir.join("missing.png")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
