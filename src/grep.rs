//! Searching the text of every file.

use std::path::PathBuf;
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
    use super::find;

    #[test]
    fn finds_with_smart_case() {
        assert_eq!(find("Hello World", "world"), Some(6..11));
        assert_eq!(find("Hello World", "World"), Some(6..11));
        assert_eq!(find("Hello world", "World"), None);
        assert_eq!(find("Ünïcode ÜBER", "über"), Some(10..15));
        assert_eq!(find("short", "longer than it"), None);
    }
}
