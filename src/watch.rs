//! Watching files for changes, so documents reload as they're edited.

use notify_debouncer_mini::notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{DebounceEventResult, Debouncer, new_debouncer};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

pub struct Watch {
    debouncer: Debouncer<RecommendedWatcher>,
    rx: Receiver<DebounceEventResult>,
    /// What's watched: folders, and whether their subfolders are too.
    watched: Vec<(PathBuf, bool)>,
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
    ///
    /// A file is watched through its folder. Watching the file itself
    /// breaks on Linux the first time an editor saves by writing a new file
    /// and renaming it over the old one, as vim does, because inotify
    /// watches the old file, which is gone.
    pub fn add(&mut self, path: &Path) {
        let (dir, recursive) = if path.is_dir() {
            (path, true)
        } else {
            match path.parent() {
                Some(dir) => (dir, false),
                None => return,
            }
        };
        if self.covers(dir, recursive) {
            return;
        }
        let mode = if recursive {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };
        if self.debouncer.watcher().watch(dir, mode).is_ok() {
            self.watched.push((dir.to_path_buf(), recursive));
        }
    }

    /// Whether watching `dir` (and its subfolders, if `recursive`) would
    /// add nothing.
    fn covers(&self, dir: &Path, recursive: bool) -> bool {
        self.watched.iter().any(|(w, all)| {
            if *all {
                dir.starts_with(w)
            } else {
                !recursive && dir == w
            }
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_what_it_covers() {
        let mut w = Watch::new().unwrap();
        w.watched = vec![(PathBuf::from("/a"), false), (PathBuf::from("/r"), true)];
        assert!(w.covers(Path::new("/a"), false));
        assert!(
            !w.covers(Path::new("/a/b"), false),
            "a folder watched for one file"
        );
        assert!(!w.covers(Path::new("/a"), true));
        assert!(w.covers(Path::new("/r/x/y"), false));
        assert!(w.covers(Path::new("/r/x"), true));
    }

    #[test]
    fn sees_saves_by_rename() {
        let dir = std::env::temp_dir().join(format!("lsmd-watch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = std::fs::canonicalize(&dir).unwrap();
        let file = dir.join("doc.md");
        std::fs::write(&file, "one").unwrap();
        let mut w = Watch::new().unwrap();
        w.add(&file);
        std::thread::sleep(Duration::from_millis(300));
        // Save twice the way vim does: write a new file, rename it over.
        for text in ["two", "three"] {
            let tmp = dir.join("doc.md.swp");
            std::fs::write(&tmp, text).unwrap();
            std::fs::rename(&tmp, &file).unwrap();
            let saw = (0..40).any(|_| {
                std::thread::sleep(Duration::from_millis(50));
                w.changed().contains(&file)
            });
            assert!(saw, "missed the save of {text:?}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
