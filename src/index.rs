//! An index of the links between the Markdown files under a directory, so a
//! document can show what links to it as well as what it links to.
//!
//! It's kept in `~/.lsmd/index/`, one file per directory browsed, with each
//! file's size and modification time, so later runs only re-read the files
//! that changed.

use crate::files::{self, Entry};
use crate::render;
use comrak::nodes::NodeValue;
use comrak::{Arena, parse_document};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::SystemTime;

/// Bump when what's stored changes, to rebuild old indexes.
const VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct FileLinks {
    /// Modification time, as seconds and nanoseconds since 1970.
    modified: Option<(u64, u32)>,
    size: u64,
    /// The first heading.
    title: Option<String>,
    /// The Markdown files it links to, relative to the root.
    links: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct Stored {
    version: u32,
    root: String,
    files: HashMap<String, FileLinks>,
}

pub struct Index {
    files: HashMap<String, FileLinks>,
    /// For each file, the files that link to it, sorted.
    backlinks: HashMap<String, Vec<String>>,
}

impl Index {
    /// The first heading of the file at `rel`, if it's indexed and has one.
    pub fn title(&self, rel: &str) -> Option<&str> {
        self.files.get(rel)?.title.as_deref()
    }

    /// The files that link to `rel`.
    pub fn linked_from(&self, rel: &str) -> &[String] {
        self.backlinks.get(rel).map_or(&[], Vec::as_slice)
    }
}

/// Indexes `entries` (files under `root`) on a background thread.
pub fn build(root: &Path, entries: Vec<Entry>) -> Receiver<Index> {
    let (tx, rx) = mpsc::channel();
    let root = root.to_path_buf();
    thread::spawn(move || {
        let _ = tx.send(update(&root, &entries, cache_file(&root)));
    });
    rx
}

/// Indexes `entries`, reusing and then updating what's stored in `cache`.
fn update(root: &Path, entries: &[Entry], cache: Option<PathBuf>) -> Index {
    let root_str = root.to_string_lossy().into_owned();
    let mut old = cache
        .as_deref()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice::<Stored>(&b).ok())
        .filter(|s| s.version == VERSION && s.root == root_str)
        .map(|s| s.files)
        .unwrap_or_default();

    let mut changed = old.len() != entries.len();
    let mut files = HashMap::with_capacity(entries.len());
    for e in entries {
        let meta = std::fs::metadata(&e.path).ok();
        let size = meta.as_ref().map_or(0, |m| m.len());
        let modified = meta.and_then(|m| m.modified().ok()).and_then(stamp);
        let links = match old.remove(&e.rel) {
            Some(f) if f.size == size && f.modified == modified && modified.is_some() => f,
            _ => {
                changed = true;
                let md = std::fs::read(&e.path)
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
                    .unwrap_or_default();
                let (title, links) = extract(&md, &e.rel);
                FileLinks {
                    modified,
                    size,
                    title,
                    links,
                }
            }
        };
        files.insert(e.rel.clone(), links);
    }

    if changed && let Some(cache) = cache {
        save(
            &cache,
            &Stored {
                version: VERSION,
                root: root_str,
                files: files.clone(),
            },
        );
    }

    let mut backlinks: HashMap<String, Vec<String>> = HashMap::new();
    for (from, f) in &files {
        for to in &f.links {
            if to != from {
                backlinks.entry(to.clone()).or_default().push(from.clone());
            }
        }
    }
    for list in backlinks.values_mut() {
        list.sort();
    }
    Index { files, backlinks }
}

/// The first heading of a document, and the Markdown files it links to,
/// relative to the root. `rel` is the document's own path from the root.
pub fn extract(md: &str, rel: &str) -> (Option<String>, Vec<String>) {
    let arena = Arena::new();
    let root = parse_document(&arena, md, &render::options());
    let mut title = None;
    let mut links: Vec<String> = Vec::new();
    for node in root.descendants() {
        match &node.data().value {
            NodeValue::Heading(_) if title.is_none() => {
                title = Some(render::plain_text(node)).filter(|t| !t.is_empty());
            }
            NodeValue::Link(link) => {
                if let Some(to) = resolve(rel, &link.url)
                    && !links.contains(&to)
                {
                    links.push(to);
                }
            }
            _ => {}
        }
    }
    (title, links)
}

/// The Markdown file a link in the document at `from` points to, relative
/// to the root. `None` for web links, anchors, other kinds of file, and
/// paths that leave the root. A leading `/` means the root, as on GitHub.
pub fn resolve(from: &str, url: &str) -> Option<String> {
    let path = render::local_path(url)?;
    if path.is_empty() || !files::is_markdown(Path::new(&path)) {
        return None;
    }
    let mut parts: Vec<&str> = if path.starts_with('/') {
        Vec::new()
    } else {
        let mut dir: Vec<&str> = from.split('/').collect();
        dir.pop();
        dir
    };
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            _ => parts.push(part),
        }
    }
    Some(parts.join("/"))
}

fn stamp(t: SystemTime) -> Option<(u64, u32)> {
    let d = t.duration_since(SystemTime::UNIX_EPOCH).ok()?;
    Some((d.as_secs(), d.subsec_nanos()))
}

/// `~/.lsmd/index/<hash of root>.json`.
fn cache_file(root: &Path) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let name = format!("{:016x}.json", fnv1a(root.to_string_lossy().as_bytes()));
    Some(PathBuf::from(home).join(".lsmd").join("index").join(name))
}

/// A hash that stays the same between runs and Rust versions, unlike std's.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
    })
}

/// Writes the index, via a temporary file so a crash can't leave half of one.
/// Failing to save only costs speed next time, so errors are ignored.
fn save(path: &Path, stored: &Stored) {
    let Some(dir) = path.parent() else { return };
    let Ok(json) = serde_json::to_vec(stored) else {
        return;
    };
    let tmp = path.with_extension("tmp");
    if std::fs::create_dir_all(dir).is_ok() && std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_links() {
        assert_eq!(resolve("docs/a.md", "b.md").as_deref(), Some("docs/b.md"));
        assert_eq!(
            resolve("docs/a.md", "../README.md#x").as_deref(),
            Some("README.md")
        );
        assert_eq!(
            resolve("docs/a.md", "./sub/c.md").as_deref(),
            Some("docs/sub/c.md")
        );
        assert_eq!(resolve("docs/a.md", "/top.md").as_deref(), Some("top.md"));
        assert_eq!(resolve("a.md", "../outside.md"), None);
        assert_eq!(resolve("a.md", "https://x.io/b.md"), None);
        assert_eq!(resolve("a.md", "#anchor"), None);
        assert_eq!(resolve("a.md", "image.png"), None);
    }

    #[test]
    fn extracts_title_and_links_but_not_code() {
        let md = "Intro\n\n# The Title\n\n[b](b.md) [b again](b.md#x) [web](https://x.io)\n\n```\n[not](c.md)\n```\n";
        let (title, links) = extract(md, "a.md");
        assert_eq!(title.as_deref(), Some("The Title"));
        assert_eq!(links, ["b.md"]);
    }

    #[test]
    fn builds_backlinks_and_caches() {
        let dir = std::env::temp_dir().join(format!("lsmd-index-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::write(
            dir.join("README.md"),
            "# Readme\n\n[guide](docs/guide.md)\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("docs/guide.md"),
            "# Guide\n\n[home](../README.md)\n",
        )
        .unwrap();
        std::fs::write(dir.join("docs/other.md"), "[guide](guide.md)\n").unwrap();
        let entries: Vec<Entry> = ["README.md", "docs/guide.md", "docs/other.md"]
            .iter()
            .map(|rel| Entry {
                path: dir.join(rel),
                rel: rel.to_string(),
                modified: None,
            })
            .collect();
        let cache = dir.join("home/.lsmd/index/test.json");
        let index = update(&dir, &entries, Some(cache.clone()));
        assert_eq!(
            index.linked_from("docs/guide.md"),
            ["README.md", "docs/other.md"]
        );
        assert_eq!(index.linked_from("README.md"), ["docs/guide.md"]);
        assert_eq!(index.title("docs/guide.md"), Some("Guide"));
        assert!(cache.exists());

        // A second run reads the cache and gets the same answer.
        let again = update(&dir, &entries, Some(cache));
        assert_eq!(again.files, index.files);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
