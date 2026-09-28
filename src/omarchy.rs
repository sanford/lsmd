//! Following the Omarchy desktop's theme.
//!
//! Omarchy keeps the active theme's `colors.toml` in
//! `~/.local/state/omarchy/current/theme/` and swaps that directory when the
//! theme changes. Code blocks take their colors from it; everything else
//! already follows it through the terminal's palette, which Omarchy sets.

use omarchy_theme::{Omarchy, Palette, Watcher};
use std::sync::mpsc::{self, Receiver};

/// The active theme's palette, when running on Omarchy.
pub fn palette() -> Option<Palette> {
    Omarchy::detect()?.palette().ok()
}

/// Watches for theme changes.
pub struct Follow {
    rx: Receiver<Palette>,
    _watcher: Watcher,
}

impl Follow {
    /// `None` when not running on Omarchy, or the system won't let us watch.
    pub fn start() -> Option<Follow> {
        let (tx, rx) = mpsc::channel();
        let watcher = Omarchy::detect()?
            .watch_palette(move |palette| {
                let _ = tx.send(palette);
            })
            .ok()?;
        Some(Follow {
            rx,
            _watcher: watcher,
        })
    }

    /// The newest palette, if the theme has changed since last asked.
    pub fn changed(&self) -> Option<Palette> {
        self.rx.try_iter().last()
    }
}
