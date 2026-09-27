//! Getting around documents: search, headings and the outline, and
//! following links, with a history to go back through.

use super::{App, Focus};
use crate::files;
use crate::render;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Padding, Paragraph};
use std::path::PathBuf;

/// Something in the reader that takes the keyboard until it's done.
pub enum Prompt {
    /// Typing a search. `from` is where the search started.
    Search { query: String, from: usize },
    /// Choosing a link by its hint letters.
    Hints { typed: String },
    /// The outline: headings, filtered by `filter`.
    Outline { filter: String, list: ListState },
    /// Confirming opening something outside lsmd.
    Open { target: String },
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
                    let current = doc
                        .headings()
                        .iter()
                        .rposition(|h| h.line <= top)
                        .unwrap_or(0);
                    let list = ListState::default().with_selected(Some(current));
                    self.prompt = Some(Prompt::Outline {
                        filter: String::new(),
                        list,
                    });
                }
            }
            KeyCode::Char('f') => self.show_hints(),
            KeyCode::Esc if doc.search_status().is_some() => doc.clear_search(),
            KeyCode::Backspace if !self.history.is_empty() => self.go_back(),
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
            Prompt::Outline {
                mut filter,
                mut list,
            } => {
                let shown = self.outline_items(&filter).len();
                match key.code {
                    KeyCode::Esc => return,
                    KeyCode::Enter => {
                        let items = self.outline_items(&filter);
                        if let Some(&(i, _)) = list.selected().and_then(|s| items.get(s)) {
                            let here = self.here();
                            if let Some(doc) = self.current() {
                                let line = doc.headings()[i].line;
                                doc.jump_to(line);
                                self.history.extend(here);
                            }
                        }
                        return;
                    }
                    KeyCode::Down => list.select_next(),
                    KeyCode::Up => list.select_previous(),
                    KeyCode::Char('n') if ctrl => list.select_next(),
                    KeyCode::Char('p') if ctrl => list.select_previous(),
                    KeyCode::Backspace => {
                        filter.pop();
                        list.select(Some(0));
                    }
                    KeyCode::Char(c) if !ctrl => {
                        filter.push(c);
                        list.select(Some(0));
                    }
                    _ => {}
                }
                if list.selected().is_some_and(|s| s >= shown) {
                    list.select(Some(shown.saturating_sub(1)));
                }
                self.prompt = Some(Prompt::Outline { filter, list });
            }
            Prompt::Open { target } => {
                if matches!(key.code, KeyCode::Char('y' | 'Y') | KeyCode::Enter) {
                    self.flash = Some(match open_externally(&target) {
                        Ok(()) => format!("Opened {target}"),
                        Err(e) => format!("Couldn't open {target}: {e}"),
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
            self.prompt = Some(Prompt::Open {
                target: url.to_string(),
            });
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
        let target = base.join(&path);
        if !target.exists() {
            self.flash = Some(format!("Not found: {path}"));
            return;
        }
        if !target.is_file() || !files::is_markdown(&target) {
            self.prompt = Some(Prompt::Open {
                target: target.display().to_string(),
            });
            return;
        }
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

    /// Headings that pass the outline's filter: their index and how they
    /// show in the list.
    fn outline_items(&mut self, filter: &str) -> Vec<(usize, Line<'static>)> {
        let Some(doc) = self.current() else {
            return Vec::new();
        };
        let pattern = Pattern::parse(filter, CaseMatching::Smart, Normalization::Smart);
        let mut matcher = Matcher::new(Config::DEFAULT);
        let mut buf = Vec::new();
        doc.headings()
            .iter()
            .enumerate()
            .filter(|(_, h)| {
                filter.is_empty()
                    || pattern
                        .score(Utf32Str::new(&h.text, &mut buf), &mut matcher)
                        .is_some()
            })
            .map(|(i, h)| {
                let indent = "  ".repeat(usize::from(h.level.saturating_sub(1)));
                let style = if h.level <= 2 {
                    Style::new().bold()
                } else {
                    Style::new()
                };
                (
                    i,
                    Line::from(vec![
                        Span::raw(format!(" {indent}")),
                        Span::styled(h.text.clone(), style),
                    ]),
                )
            })
            .collect()
    }

    pub(super) fn draw_outline(&mut self, f: &mut Frame) {
        if !matches!(self.prompt, Some(Prompt::Outline { .. })) {
            return;
        }
        let Some(Prompt::Outline { filter, list }) = self.prompt.take() else {
            return;
        };
        let items = self.outline_items(&filter);
        let area = f.area();
        let widest = items.iter().map(|(_, l)| l.width()).max().unwrap_or(0);
        let width = (widest as u16 + 4)
            .clamp(30, area.width.saturating_sub(4).max(1))
            .min(area.width);
        let height = (items.len() as u16 + 3)
            .min(area.height.saturating_sub(2))
            .max(3)
            .min(area.height);
        let rect = Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + (area.height - height) / 2,
            width,
            height,
        };
        let block = Block::bordered()
            .title(" Outline ")
            .padding(Padding::horizontal(0));
        let inner = block.inner(rect);
        f.render_widget(Clear, rect);
        f.render_widget(block, rect);
        let list_area = Rect {
            height: inner.height.saturating_sub(1),
            ..inner
        };
        let filter_area = Rect {
            y: inner.y + list_area.height,
            height: 1,
            ..inner
        };
        let mut state = list;
        let widget = List::new(
            items
                .into_iter()
                .map(|(_, l)| ListItem::new(l))
                .collect::<Vec<_>>(),
        )
        .highlight_style(Style::new().add_modifier(Modifier::REVERSED));
        f.render_stateful_widget(widget, list_area, &mut state);
        let prompt = if filter.is_empty() {
            Line::from(" type to filter, ⏎ to jump".dim())
        } else {
            Line::from(vec![
                " ".into(),
                Span::raw(filter.clone()),
                "▏".slow_blink(),
            ])
        };
        f.render_widget(Paragraph::new(prompt), filter_area);
        self.prompt = Some(Prompt::Outline {
            filter,
            list: state,
        });
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
            Prompt::Outline { .. } => Line::from(" ↑↓ choose  ⏎ jump  esc close".dim()),
            Prompt::Open { target } => Line::from(vec![
                " Open ".bold(),
                Span::raw(target.clone()),
                "? ".bold(),
                "y/n".dim(),
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

/// Opens a URL or file with the system's default app.
fn open_externally(target: &str) -> std::io::Result<()> {
    use std::process::{Command, Stdio};
    let mut cmd = if cfg!(target_os = "macos") {
        Command::new("open")
    } else if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", ""]);
        c
    } else {
        Command::new("xdg-open")
    };
    cmd.arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
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
