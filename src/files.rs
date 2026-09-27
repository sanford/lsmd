//! Finding the Markdown files under a directory.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

pub const EXTENSIONS: &[&str] = &["md", "markdown", "mdown", "mkd", "mdx"];

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    /// `path` relative to the root, with `/` separators on every platform.
    pub rel: String,
    pub modified: Option<SystemTime>,
}

pub fn is_markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// Walks `root` on a background thread, sending files in batches as it
/// finds them. The channel closes when the walk is done. With `all`, hidden
/// and ignored files are included.
pub fn scan(root: &Path, all: bool) -> Receiver<Vec<Entry>> {
    let (tx, rx) = mpsc::channel();
    let root = root.to_path_buf();
    thread::spawn(move || {
        let walk = ignore::WalkBuilder::new(&root)
            .standard_filters(!all)
            .require_git(false)
            .build();
        let mut batch = Vec::new();
        let mut sent = Instant::now();
        for dent in walk.flatten() {
            if !dent.file_type().is_some_and(|t| t.is_file()) || !is_markdown(dent.path()) {
                continue;
            }
            let rel = dent.path().strip_prefix(&root).unwrap_or(dent.path());
            let rel = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            batch.push(Entry {
                modified: dent.metadata().ok().and_then(|m| m.modified().ok()),
                path: dent.into_path(),
                rel,
            });
            // Send early and often at first, so the list appears at once.
            if sent.elapsed() > Duration::from_millis(30) {
                if tx.send(std::mem::take(&mut batch)).is_err() {
                    return;
                }
                sent = Instant::now();
            }
        }
        if !batch.is_empty() {
            let _ = tx.send(batch);
        }
    });
    rx
}

/// Directory order: a directory's own files before its subdirectories',
/// READMEs first, then names case-insensitively.
pub fn by_path(a: &Entry, b: &Entry) -> Ordering {
    let key = |e: &Entry| {
        let (dir, name) = e.rel.rsplit_once('/').unwrap_or(("", &e.rel));
        let dirs: Vec<String> = dir
            .split('/')
            .filter(|s| !s.is_empty())
            .map(str::to_lowercase)
            .collect();
        let readme = name.to_lowercase().starts_with("readme.");
        (dirs, !readme, name.to_lowercase())
    };
    key(a).cmp(&key(b)).then_with(|| a.rel.cmp(&b.rel))
}

/// Newest first.
pub fn by_modified(a: &Entry, b: &Entry) -> Ordering {
    b.modified.cmp(&a.modified).then_with(|| by_path(a, b))
}

/// "5m ago", "3d ago", ...: short enough for a column.
pub fn ago(time: Option<SystemTime>, now: SystemTime) -> String {
    let Some(secs) = time
        .and_then(|t| now.duration_since(t).ok())
        .map(|d| d.as_secs())
    else {
        return String::new();
    };
    const MIN: u64 = 60;
    const HOUR: u64 = 60 * MIN;
    const DAY: u64 = 24 * HOUR;
    match secs {
        s if s < MIN => "now".into(),
        s if s < HOUR => format!("{}m ago", s / MIN),
        s if s < DAY => format!("{}h ago", s / HOUR),
        s if s < 14 * DAY => format!("{}d ago", s / DAY),
        s if s < 60 * DAY => format!("{}w ago", s / (7 * DAY)),
        s if s < 365 * DAY => format!("{}mo ago", s / (30 * DAY)),
        s => format!("{}y ago", s / (365 * DAY)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(rel: &str) -> Entry {
        Entry {
            path: PathBuf::from(rel),
            rel: rel.into(),
            modified: None,
        }
    }

    #[test]
    fn sorts_readme_first_and_files_before_subdirectories() {
        let mut v: Vec<_> = [
            "docs/b.md",
            "zeta.md",
            "docs/README.md",
            "Alpha.md",
            "README.md",
            "docs/deep/a.md",
        ]
        .into_iter()
        .map(entry)
        .collect();
        v.sort_by(by_path);
        let order: Vec<_> = v.iter().map(|e| e.rel.as_str()).collect();
        assert_eq!(
            order,
            [
                "README.md",
                "Alpha.md",
                "zeta.md",
                "docs/README.md",
                "docs/b.md",
                "docs/deep/a.md"
            ]
        );
    }

    #[test]
    fn formats_ages() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000_000);
        let ago_secs = |s| ago(Some(now - Duration::from_secs(s)), now);
        assert_eq!(ago_secs(5), "now");
        assert_eq!(ago_secs(300), "5m ago");
        assert_eq!(ago_secs(7200), "2h ago");
        assert_eq!(ago_secs(3 * 86400), "3d ago");
        assert_eq!(ago(None, now), "");
    }

    #[test]
    fn recognizes_markdown() {
        assert!(is_markdown(Path::new("a/B.MD")));
        assert!(is_markdown(Path::new("x.markdown")));
        assert!(!is_markdown(Path::new("x.txt")));
        assert!(!is_markdown(Path::new("md")));
    }
}
