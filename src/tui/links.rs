//! What a document links to, and what links to it: the counts in the
//! header, and the links panel (`L`).

use super::picker::{Picker, Row, Target};
use super::{App, Focus};
use crate::{files, index, render, wrap};
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use std::path::{Component, Path, PathBuf};

impl App<'_> {
    /// `path` relative to the root, if it's under it.
    pub(super) fn rel_of(&self, path: &Path) -> Option<String> {
        let rest = path.strip_prefix(self.root.as_ref()?).ok()?;
        let parts: Vec<_> = rest
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect();
        Some(parts.join("/"))
    }

    /// The file of the document on screen (`None` for standard input).
    fn current_path(&self) -> Option<PathBuf> {
        match self.focus {
            Focus::Reader => self.reading.clone(),
            Focus::List => self.selected().map(|e| e.path.clone()),
        }
    }

    /// How many Markdown files the document on screen links to, and how
    /// many link to it (once the index is ready).
    pub(super) fn link_counts(&mut self) -> (usize, Option<usize>) {
        let rel = self.current_path().and_then(|p| self.rel_of(&p));
        let incoming = self
            .index
            .as_ref()
            .map(|ix| rel.map_or(0, |r| ix.linked_from(&r).len()));
        let out = self.current().map_or(0, |doc| {
            let base = doc.base.clone().unwrap_or_default();
            let mut targets: Vec<PathBuf> = doc
                .links()
                .iter()
                .filter_map(|url| markdown_target(&base, url))
                .filter(|p| p.is_file())
                .collect();
            targets.sort();
            targets.dedup();
            targets.len()
        });
        (out, incoming)
    }

    /// The header's note about links: "3 linked docs · linked from 2".
    pub(super) fn link_note(&mut self) -> Option<Line<'static>> {
        let (out, incoming) = self.link_counts();
        let plural =
            |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        let mut parts = Vec::new();
        if out > 0 {
            parts.push(plural(out, "linked doc", "linked docs"));
        }
        if let Some(n) = incoming.filter(|&n| n > 0) {
            parts.push(format!("linked from {n}"));
        }
        if parts.is_empty() {
            return None;
        }
        Some(Line::from(vec![
            Span::raw(parts.join(" · ")).dim(),
            "  L".bold(),
            " links ".dim(),
        ]))
    }

    /// The links panel for the document on screen, or `None` if it has no
    /// links either way.
    pub(super) fn links_picker(&mut self) -> Option<Picker> {
        let rel = self.current_path().and_then(|p| self.rel_of(&p));
        let (urls, base, headings) = {
            let doc = self.current()?;
            let headings: Vec<(String, String)> = doc
                .headings()
                .iter()
                .map(|h| (h.slug.clone(), h.text.clone()))
                .collect();
            (
                doc.links().to_vec(),
                doc.base.clone().unwrap_or_default(),
                headings,
            )
        };

        // Links out, grouped by kind, each target once.
        let mut docs: Vec<(String, Option<String>, bool, String)> = Vec::new();
        let (mut sections, mut others, mut web) = (Vec::new(), Vec::new(), Vec::new());
        let mut seen = Vec::new();
        for url in urls {
            let Some(path) = render::local_path(&url) else {
                if !seen.contains(&url) {
                    seen.push(url.clone());
                    web.push(Row::item(
                        url.clone(),
                        Line::from(format!("   {url}")),
                        Target::Link(url),
                    ));
                }
                continue;
            };
            if path.is_empty() {
                let slug = url.trim_start_matches('#').to_lowercase();
                if seen.contains(&slug) {
                    continue;
                }
                seen.push(slug.clone());
                let text = headings
                    .iter()
                    .find(|(s, _)| *s == slug)
                    .map_or(url.clone(), |(_, t)| t.clone());
                sections.push(Row::item(
                    text.clone(),
                    Line::from(format!("   § {text}")),
                    Target::Link(url),
                ));
                continue;
            }
            let target = clean(&base.join(&path));
            let key = target.to_string_lossy().into_owned();
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            if files::is_markdown(&target) {
                let shown = self.rel_of(&target).unwrap_or(path);
                docs.push((shown, self.title_of(&target), target.is_file(), url));
            } else {
                others.push(Row::item(
                    path.clone(),
                    Line::from(format!("   {path}")),
                    Target::Link(url),
                ));
            }
        }

        // Links in, from the index.
        let incoming: Option<Vec<(String, Option<String>)>> = self.index.as_ref().map(|ix| {
            rel.as_deref()
                .map(|r| ix.linked_from(r))
                .unwrap_or_default()
                .iter()
                .map(|from| (from.clone(), ix.title(from).map(str::to_string)))
                .collect()
        });

        // Paths and titles in two columns.
        let col = docs
            .iter()
            .map(|d| &d.0)
            .chain(incoming.iter().flatten().map(|d| &d.0))
            .map(|p| wrap::width(p))
            .max()
            .unwrap_or(0);
        let doc_line = |path: &str, title: Option<&str>, exists: bool| {
            let pad = " ".repeat(col - wrap::width(path) + 2);
            let tail = if exists {
                Span::raw(title.unwrap_or("").to_string()).dim()
            } else {
                "✗ missing".red()
            };
            Line::from(vec![
                "   ".into(),
                Span::raw(path.to_string()).bold(),
                Span::raw(pad),
                tail,
            ])
        };

        let mut rows = Vec::new();
        if !docs.is_empty() {
            rows.push(Row::heading(&format!("Links to ({})", docs.len())));
            for (path, title, exists, url) in docs {
                let text = format!("{path} {}", title.as_deref().unwrap_or(""));
                rows.push(Row::item(
                    text,
                    doc_line(&path, title.as_deref(), exists),
                    Target::Link(url),
                ));
            }
        }
        for (name, group) in [
            ("Sections", sections),
            ("Other files", others),
            ("Web", web),
        ] {
            if !group.is_empty() {
                rows.push(Row::heading(&format!("{name} ({})", group.len())));
                rows.extend(group);
            }
        }
        match incoming {
            Some(from) if !from.is_empty() => {
                rows.push(Row::heading(&format!("Linked from ({})", from.len())));
                let root = self.root.clone().unwrap_or_default();
                for (path, title) in from {
                    let text = format!("{path} {}", title.as_deref().unwrap_or(""));
                    let line = doc_line(&path, title.as_deref(), true);
                    rows.push(Row::item(text, line, Target::File(root.join(&path))));
                }
            }
            None if self.indexing.is_some() => {
                rows.push(Row::heading("Linked from: still indexing…"))
            }
            _ => {}
        }
        if rows.iter().all(|r| !r.choosable()) {
            return None;
        }
        Some(Picker::new("Links".into(), rows, None))
    }

    /// A document's first heading, from the index if it's there.
    fn title_of(&self, path: &Path) -> Option<String> {
        if let (Some(ix), Some(rel)) = (&self.index, self.rel_of(path))
            && let Some(title) = ix.title(&rel)
        {
            return Some(title.to_string());
        }
        let md = std::fs::read_to_string(path).ok()?;
        index::extract(&md, "").0
    }
}

/// The Markdown file `url` points to, from a document in `base`.
fn markdown_target(base: &Path, url: &str) -> Option<PathBuf> {
    let path = render::local_path(url).filter(|p| !p.is_empty())?;
    let target = clean(&base.join(path));
    files::is_markdown(&target).then_some(target)
}

/// Resolves `.` and `..` without touching the file system.
fn clean(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_paths() {
        assert_eq!(
            clean(Path::new("/a/b/../c/./d.md")),
            PathBuf::from("/a/c/d.md")
        );
    }
}
