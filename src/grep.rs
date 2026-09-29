//! Searching the text of every file.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;

/// Stop after this many matches: past it, the query needs narrowing anyway.
pub const MAX_HITS: usize = 5000;

pub struct Hit {
    pub path: PathBuf,
    pub rel: String,
    /// 1-based.
    pub line: usize,
    pub text: String,
    /// Byte range of the match in `text`.
    pub range: std::ops::Range<usize>,
}

/// Searches `files` (path, relative path) for `query` on a background
/// thread, sending all the hits at once when done.
pub fn search(files: Vec<(PathBuf, String)>, query: String) -> Receiver<Vec<Hit>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut hits = Vec::new();
        'files: for (path, rel) in files {
            let too_big =
                std::fs::metadata(&path).map_or(true, |m| m.len() > crate::files::BACKGROUND_LIMIT);
            if too_big {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            let text = String::from_utf8_lossy(&bytes);
            for (i, line) in text.lines().enumerate() {
                if let Some(range) = find(line, &query) {
                    hits.push(Hit {
                        path: path.clone(),
                        rel: rel.clone(),
                        line: i + 1,
                        text: line.to_string(),
                        range,
                    });
                    if hits.len() >= MAX_HITS {
                        break 'files;
                    }
                }
            }
        }
        let _ = tx.send(hits);
    });
    rx
}

/// Counts the lines of each of `files` that have `query` in them, on
/// background threads, sending the files with any all at once when done,
/// unless `stop` is set first.
pub fn count(
    files: Vec<PathBuf>,
    query: String,
    stop: Arc<AtomicBool>,
) -> Receiver<HashMap<PathBuf, usize>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let threads = thread::available_parallelism().map_or(4, |n| n.get().min(8));
        let chunk = files.len().div_ceil(threads).max(1);
        let found: HashMap<PathBuf, usize> = thread::scope(|s| {
            let workers: Vec<_> = files
                .chunks(chunk)
                .map(|paths| {
                    let (query, stop) = (&query, &stop);
                    s.spawn(move || {
                        let mut found = Vec::new();
                        for path in paths {
                            if stop.load(Ordering::Relaxed) {
                                break;
                            }
                            let n = count_in(path, query);
                            if n > 0 {
                                found.push((path.clone(), n));
                            }
                        }
                        found
                    })
                })
                .collect();
            workers
                .into_iter()
                .flat_map(|w| w.join().unwrap_or_default())
                .collect()
        });
        if !stop.load(Ordering::Relaxed) {
            let _ = tx.send(found);
        }
    });
    rx
}

/// How many lines of the file at `path` have `query` in them, with smart
/// case as [`find`] has it.
fn count_in(path: &std::path::Path, query: &str) -> usize {
    let too_big =
        std::fs::metadata(path).map_or(true, |m| m.len() > crate::files::BACKGROUND_LIMIT);
    if too_big {
        return 0;
    }
    let Ok(bytes) = std::fs::read(path) else {
        return 0;
    };
    let text = String::from_utf8_lossy(&bytes);
    if query.chars().any(char::is_uppercase) {
        text.lines().filter(|l| l.contains(query)).count()
    } else {
        let query = query.to_lowercase();
        text.to_lowercase()
            .lines()
            .filter(|l| l.contains(&query))
            .count()
    }
}

/// The first match of `query` in `line`. Smart case: case matters only if
/// the query has capitals.
pub fn find(line: &str, query: &str) -> Option<std::ops::Range<usize>> {
    if query.is_empty() {
        return None;
    }
    if query.chars().any(char::is_uppercase) {
        return line.find(query).map(|i| i..i + query.len());
    }
    let needle: Vec<char> = query.chars().collect();
    let starts: Vec<usize> = line.char_indices().map(|(i, _)| i).collect();
    let chars: Vec<char> = line.chars().collect();
    let fold = |c: char| c.to_lowercase().next().unwrap_or(c);
    (0..chars.len().saturating_sub(needle.len() - 1)).find_map(|at| {
        let hit = chars[at..at + needle.len()]
            .iter()
            .zip(&needle)
            .all(|(&a, &b)| fold(a) == b);
        hit.then(|| {
            let end = starts.get(at + needle.len()).copied().unwrap_or(line.len());
            starts[at]..end
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_with_smart_case() {
        assert_eq!(find("Hello World", "world"), Some(6..11));
        assert_eq!(find("Hello World", "World"), Some(6..11));
        assert_eq!(find("Hello world", "World"), None);
        assert_eq!(find("Ünïcode ÜBER", "über"), Some(10..15));
        assert_eq!(find("short", "longer than it"), None);
    }

    #[test]
    fn counts_matching_lines_per_file() {
        let dir = std::env::temp_dir().join(format!("lsmd-count-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.md"), dir.join("b.md"));
        std::fs::write(&a, "Needle\nno\nneedle needle\n").unwrap();
        std::fs::write(&b, "nothing\n").unwrap();
        let files = vec![a.clone(), b];
        let stop = Arc::new(AtomicBool::new(false));
        let found = count(files.clone(), "needle".into(), stop).recv().unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[&a], 2);
        let stop = Arc::new(AtomicBool::new(false));
        let found = count(files, "Needle".into(), stop).recv().unwrap();
        assert_eq!(found[&a], 1, "a capital makes case matter");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
