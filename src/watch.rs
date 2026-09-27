//! Watching files for changes, so documents reload as they're edited.

use notify_debouncer_mini::notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{DebounceEventResult, Debouncer, new_debouncer};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

pub struct Watch {
    debouncer: Debouncer<RecommendedWatcher>,
    rx: Receiver<DebounceEventResult>,
    watched: Vec<PathBuf>,
}

impl Watch {
    /// Starts watching nothing. `None` if the system won't let us watch.
    pub fn new() -> Option<Watch> {
        let (tx, rx) = mpsc::channel();
        // Editors save in bursts (write, rename, touch): wait for quiet.
        let debouncer = new_debouncer(Duration::from_millis(100), tx).ok()?;
        Some(Watch {
            debouncer,
            rx,
            watched: Vec::new(),
        })
    }

    /// Watches `path`: a directory and everything in it, or one file.
    /// Watching something already covered does nothing.
    pub fn add(&mut self, path: &Path) {
        if self.watched.iter().any(|w| path.starts_with(w)) {
            return;
        }
        let mode = if path.is_dir() {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };
        if self.debouncer.watcher().watch(path, mode).is_ok() {
            self.watched.push(path.to_path_buf());
        }
    }

    /// The paths that have changed since last asked, each once.
    pub fn changed(&self) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = self
            .rx
            .try_iter()
            .filter_map(Result::ok)
            .flatten()
            .map(|e| e.path)
            .collect();
        paths.sort();
        paths.dedup();
        paths
    }
}
