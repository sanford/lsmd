//! Getting around documents: search, headings and the outline, and
//! following links, with a history to go back through.

use super::picker::{Outcome, Picker, Row, Target};
use super::{App, Focus};
use crate::files;
use crate::render;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use std::path::PathBuf;

/// Something in the reader that takes the keyboard until it's done.
pub enum Prompt {
    /// Typing a search. `from` is where the search started.
    Search { query: String, from: usize },
    /// Choosing a link by its hint letters.
    Hints { typed: String },
    /// The outline or the links panel.
    Pick(Picker),
    /// Confirming opening something outside lsmd.
    Open(crate::open::Target),
    /// Typing a search of every file's text.
    Grep { query: String },
}

/// A place to go back to: a document (`None` for standard input) and the
/// line at the top of the screen.
pub struct Place {
    path: Option<PathBuf>,
    top: usize,
}

/// Letters for link hints, easiest to reach first.
const HINT_KEYS: &str = "asdfjklghqwertyuiopzxcvbnm";

impl App<'_> {
    /// Reader keys for getting around. Returns true if the key was one.
    pub(super) fn nav_key(&mut self, key: KeyEvent) -> bool {
        let Some(doc) = self.current() else {
            return false;
        };
        match key.code {
            KeyCode::Char('/') => {
                let from = doc.top();
                doc.clear_search();
                self.prompt = Some(Prompt::Search {
                    query: String::new(),
                    from,
                });
            }
            KeyCode::Char(c @ ('n' | 'N')) => {
                if !doc.search_next(c == 'n') {
                    self.flash = Some("No search (/ to search)".into());
                }
            }
            KeyCode::Char(c @ (']' | '[')) => {
                if !doc.jump_heading(c == ']') {
                    let which = if c == ']' { "below" } else { "above" };
                    self.flash = Some(format!("No heading {which}"));
                }
            }
            KeyCode::Char('o') => {
                if doc.headings().is_empty() {
                    self.flash = Some("No headings".into());
                } else {
                    let top = doc.top();
                    let current = doc.headings().iter().rposition(|h| h.line <= top);
                    let rows = doc
                        .headings()
                        .iter()
                        .map(|h| {
                            let indent = "  ".repeat(usize::from(h.level.saturating_sub(1)));
                            let text = Span::raw(h.text.clone());
                            let text = if h.level <= 2 { text.bold() } else { text };
                            let line = Line::from(vec![Span::raw(format!(" {indent}")), text]);
                            Row::item(h.text.clone(), line, Target::Line(h.line))
                        })
                        .collect();
                    self.prompt = Some(Prompt::Pick(Picker::new("Outline".into(), rows, current)));
                }
            }
            KeyCode::Char('L') => self.open_links(),
            KeyCode::Char('f') => self.show_hints(),
            KeyCode::Esc if doc.search_status().is_some() => doc.clear_search(),
            KeyCode::Esc | KeyCode::Backspace if !self.history.is_empty() => self.go_back(),
            _ => return false,
        }
        true
    }

    /// Handles a key while `prompt` is up. The prompt stays up unless this
    /// puts it back.
    pub(super) fn prompt_key(&mut self, prompt: Prompt, key: KeyEvent, ctrl: bool) {
        match prompt {
            Prompt::Search { mut query, from } => {
                match key.code {
                    KeyCode::Enter => {
                        if let Some(doc) = self.current()
                            && doc.search_status().is_some_and(|s| s.starts_with("0/"))
                        {
                            doc.clear_search();
                            self.flash = Some(format!("Not found: {query}"));
                        }
                        return;
                    }
                    KeyCode::Esc => {
                        if let Some(doc) = self.current() {
                            doc.clear_search();
                            doc.jump_to(from);
                        }
                        return;
                    }
                    KeyCode::Backspace => {
                        query.pop();
                    }
                    KeyCode::Char(c) if !ctrl => query.push(c),
                    _ => {}
                }
                if let Some(doc) = self.current() {
                    doc.search(&query, from);
                }
                self.prompt = Some(Prompt::Search { query, from });
            }
            Prompt::Hints { mut typed } => {
                let KeyCode::Char(c) = key.code else {
                    return self.clear_hints();
                };
                typed.push(c);
                let Some(doc) = self.current() else { return };
                let url = doc
                    .hints
                    .iter()
                    .find(|h| h.label == typed)
                    .map(|h| h.url.clone());
                let partial = doc.hints.iter().any(|h| h.label.starts_with(&typed));
                if let Some(url) = url {
                    self.clear_hints();
                    self.follow(&url);
                } else if partial {
                    self.prompt = Some(Prompt::Hints { typed });
                } else {
                    self.clear_hints();
                }
            }
            Prompt::Pick(mut picker) => match picker.key(key, ctrl) {
                Outcome::Stay => self.prompt = Some(Prompt::Pick(picker)),
                Outcome::Close => {}
                Outcome::Choose(target) => self.go(target),
            },
            Prompt::Grep { mut query } => match key.code {
                KeyCode::Esc => {}
                KeyCode::Enter => self.start_grep(query),
                KeyCode::Backspace => {
                    query.pop();
                    self.prompt = Some(Prompt::Grep { query });
                }
                KeyCode::Char(c) if !ctrl => {
                    query.push(c);
                    self.prompt = Some(Prompt::Grep { query });
                }
                _ => self.prompt = Some(Prompt::Grep { query }),
            },
            Prompt::Open(target) => {
                if matches!(key.code, KeyCode::Char('y' | 'Y') | KeyCode::Enter) {
                    self.flash = Some(match crate::open::open(&target) {
                        Ok(()) => format!("Opened {}", target.what),
                        Err(e) => format!("Couldn't open {}: {e}", target.what),
                    });
                }
            }
        }
    }

    /// Labels the links on screen so one can be followed by typing.
    fn show_hints(&mut self) {
        let Some(doc) = self.current() else { return };
        let links = doc.visible_links();
        if links.is_empty() {
            self.flash = Some("No links on screen".into());
            return;
        }
        let labels = hint_labels(links.len());
        let hints = links
            .into_iter()
            .zip(labels)
            .map(|((line, col, url), label)| (label, line, col, url))
            .collect();
        doc.set_hints(hints);
        self.prompt = Some(Prompt::Hints {
            typed: String::new(),
        });
    }

    fn clear_hints(&mut self) {
        if let Some(doc) = self.current() {
            doc.set_hints(Vec::new());
        }
    }

    /// Where the reader is now, to come back to.
    fn here(&mut self) -> Option<Place> {
        let path = self.reading.clone();
        let top = self.current()?.top();
        Some(Place { path, top })
    }

    /// Where Esc goes from the reader, for the footer.
    pub(super) fn back_label(&self) -> String {
        match self.history.last() {
            Some(Place {
                path: Some(path), ..
            }) if Some(path) == self.reading.as_ref() => "back".into(),
            Some(Place {
                path: Some(path), ..
            }) => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
                format!("back to {}", name.unwrap_or_default())
            }
            Some(Place { path: None, .. }) => "back to stdin".into(),
            None if self.text.is_none() => "list".into(),
            None => "quit".into(),
        }
    }

    fn go_back(&mut self) {
        let Some(place) = self.history.pop() else {
            return;
        };
        self.reading = place.path;
        if let Some(doc) = self.current() {
            doc.jump_to(place.top);
        }
    }

    /// Follows a link: to a heading, another Markdown file, or (after
    /// asking) a web page or other file opened by the system.
    pub(super) fn follow(&mut self, url: &str) {
        let here = self.here();
        let Some(doc) = self.current() else { return };
        let Some(path) = render::local_path(url) else {
            match crate::open::web(url) {
                Ok(target) => self.prompt = Some(Prompt::Open(target)),
                Err(why) => self.flash = Some(why),
            }
            return;
        };
        let anchor = url.split_once('#').map(|(_, a)| a.to_string());
        if path.is_empty() {
            // A heading in this document.
            let Some(anchor) = anchor else { return };
            match doc.anchor(&anchor) {
                Some(line) => {
                    doc.jump_to(line);
                    self.history.extend(here);
                }
                None => self.flash = Some(format!("No heading #{anchor}")),
            }
            return;
        }
        let base = doc.base.clone().unwrap_or_default();
        let target = render::local_target(&base, doc.site.as_deref(), &path);
        if !target.exists() {
            self.flash = Some(format!("Not found: {path}"));
            return;
        }
        if !target.is_file() || !files::is_markdown(&target) {
            match crate::open::file(&target) {
                Ok(target) => self.prompt = Some(Prompt::Open(target)),
                Err(why) => self.flash = Some(why),
            }
            return;
        }
        self.open_path(target, anchor);
    }

    pub(super) fn draw_picker(&mut self, f: &mut Frame) {
        if let Some(Prompt::Pick(picker)) = &mut self.prompt {
            picker.draw(f);
        }
    }

    /// Opens the links panel for the document on screen.
    pub(super) fn open_links(&mut self) {
        match self.links_picker() {
            Some(picker) => self.prompt = Some(Prompt::Pick(picker)),
            None => self.flash = Some("No links to or from this document".into()),
        }
    }

    /// Goes where a picker row points.
    fn go(&mut self, target: Target) {
        // Chosen from the list: read that document, with Esc going back to
        // the list rather than to the document that was being previewed.
        let from_list = self.focus == Focus::List;
        if from_list {
            self.reading = self.selected().map(|e| e.path.clone());
            self.focus = Focus::Reader;
        }
        match target {
            Target::Line(line) => {
                let here = self.here();
                if let Some(doc) = self.current() {
                    doc.jump_to(line);
                    self.history.extend(here);
                }
            }
            Target::Link(url) => self.follow(&url),
            Target::File(path) => self.open_path(path, None),
            Target::Match(path, line, query) => {
                self.open_path(path.clone(), None);
                self.doc(&path).go_to_match(line, &query);
            }
        }
        if from_list {
            self.history.clear();
        }
    }

    /// Opens a Markdown file in the reader, remembering where we were.
    fn open_path(&mut self, target: PathBuf, anchor: Option<String>) {
        let here = self.here();
        let target = std::fs::canonicalize(&target).unwrap_or(target);
        self.history.extend(here);
        self.reading = Some(target.clone());
        self.focus = Focus::Reader;
        let doc = self.doc(&target);
        doc.jump_to(0);
        if let Some(anchor) = anchor {
            doc.go_to_anchor(&anchor);
        }
    }

    /// The footer while a prompt is up.
    pub(super) fn prompt_footer(&mut self) -> Option<Line<'static>> {
        let line = match self.prompt.as_ref()? {
            Prompt::Search { query, .. } => {
                let query = query.clone();
                let status = self
                    .current()
                    .and_then(|d| d.search_status())
                    .unwrap_or_default();
                Line::from(vec![
                    " /".bold(),
                    Span::raw(query),
                    "▏".slow_blink(),
                    Span::raw(format!("  {status}")).dim(),
                ])
            }
            Prompt::Hints { typed } => Line::from(vec![
                " Follow link: ".bold(),
                Span::raw(format!("type its letters {typed}")),
                "  esc cancels".dim(),
            ]),
            Prompt::Pick(_) => Line::from(" ↑↓ choose  ⏎ go  esc close".dim()),
            Prompt::Grep { query } => Line::from(vec![
                " Search all files: ".bold(),
                Span::raw(query.clone()),
                "▏".slow_blink(),
                "  ⏎ search  esc cancel".dim(),
            ]),
            Prompt::Open(target) => Line::from(vec![
                " Open ".bold(),
                Span::raw(target.what.clone()),
                "? ".bold(),
                "y/n  ".dim(),
                Span::raw(target.target.clone()).dim(),
            ]),
        };
        Some(line)
    }
}

/// `n` distinct labels, all the same length, from [`HINT_KEYS`].
fn hint_labels(n: usize) -> Vec<String> {
    let keys: Vec<char> = HINT_KEYS.chars().collect();
    let mut len = 1;
    while keys.len().pow(len) < n {
        len += 1;
    }
    (0..n)
        .map(|mut i| {
            let mut label = Vec::new();
            for _ in 0..len {
                label.push(keys[i % keys.len()]);
                i /= keys.len();
            }
            label.iter().rev().collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_unique_and_even() {
        assert_eq!(hint_labels(3), ["a", "s", "d"]);
        let many = hint_labels(30);
        assert!(many.iter().all(|l| l.len() == 2));
        let mut sorted = many.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 30);
    }
}
