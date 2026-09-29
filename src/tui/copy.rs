//! Copying (`c`): a menu of a code block, the section, the whole document,
//! a link to the section, the file's path, or pieces of the document picked
//! out with the keyboard or the mouse, each saying just what it'll copy.
//! Everything but a path is copied as Markdown, as written.

use super::App;
use super::menu::{Item, Menu};
use super::nav::{Prompt, hint_labels};
use crate::clipboard;
use crate::doc::Doc;
use crate::render::CodeBlock;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use std::path::{Path, PathBuf};

/// What a menu item does.
#[derive(Clone)]
pub(super) enum Action {
    /// Copy this, and say "Copied `what`".
    Copy { text: String, what: String },
    /// Copy a code block: the one on screen, or with more, the one whose
    /// letters are typed.
    ChooseCode,
    /// Select pieces of the document to copy.
    Select,
    /// Open a link outside lsmd.
    Open(crate::open::Target),
}

/// "1 line", "42 lines".
fn lines(n: usize) -> String {
    format!("{n} line{}", if n == 1 { "" } else { "s" })
}

impl App {
    /// `c`: what to copy, and what each choice will copy. Sections and
    /// links are of `heading`, the outline's selection, or else the section
    /// on screen.
    pub(super) fn open_copy(&mut self, heading: Option<usize>) {
        let path = self.path_to_copy();
        let root = self.root.clone();
        let doc_path = self.current_path();
        let Some(doc) = self.current() else { return };
        let heading = heading.or_else(|| doc.current_heading());
        let mut items = Vec::new();

        let code = doc.code_on_screen();
        items.push(match code[..] {
            [] => Item::off('c', "Code block", "none on screen"),
            [(i, _)] => {
                let what = doc.code_block(i).map_or(String::new(), code_what);
                Item::new('c', "Code block", what, Action::ChooseCode)
            }
            _ => Item::new(
                'c',
                "Code block",
                format!("{} on screen: pick one next", code.len()),
                Action::ChooseCode,
            ),
        });

        let section = heading.and_then(|i| {
            let title = doc.headings().get(i)?.text.clone();
            Some((title, doc.section_source(i)?))
        });
        items.push(match section {
            Some((title, text)) => {
                let what = format!("\"{title}\", {}", lines(text.lines().count()));
                Item::new('s', "Section", what.clone(), Action::Copy { text, what })
            }
            None => Item::off('s', "Section", "none here: before the first heading"),
        });

        let text = doc.markdown().to_string();
        let name = doc_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map_or("standard input".into(), |n| {
                n.to_string_lossy().into_owned()
            });
        let what = format!("{name}, {}", lines(text.lines().count()));
        items.push(Item::new(
            'a',
            "Whole document",
            what.clone(),
            Action::Copy { text, what },
        ));

        items.push(Item::new(
            'v',
            "Select…",
            "paragraphs, list items or lines: j k, then c",
            Action::Select,
        ));

        let link = link(doc, heading, doc_path.as_deref(), root.as_deref());
        items.push(match link {
            Some(link) => Item::new(
                'l',
                "Link here",
                link.clone(),
                Action::Copy {
                    text: link.clone(),
                    what: link,
                },
            ),
            None => Item::off('l', "Link here", "nothing to link to"),
        });

        match path {
            Some(full) => {
                let rel = root
                    .as_ref()
                    .and_then(|r| full.strip_prefix(r).ok())
                    .filter(|rel| !rel.as_os_str().is_empty())
                    .unwrap_or(&full)
                    .display()
                    .to_string();
                let full = full.display().to_string();
                for (key, label, path) in [('p', "Path", rel), ('P', "Full path", full)] {
                    let action = Action::Copy {
                        text: path.clone(),
                        what: path.clone(),
                    };
                    items.push(Item::new(key, label, path, action));
                }
            }
            None => {
                items.push(Item::off('p', "Path", "standard input has none"));
                items.push(Item::off('P', "Full path", "standard input has none"));
            }
        }
        self.prompt = Some(Prompt::Menu(Menu::new("Copy", items)));
    }

    /// Asks before opening a link outside lsmd, offering to copy it instead.
    pub(super) fn ask_open(&mut self, target: crate::open::Target) {
        let items = vec![
            Item::new(
                'o',
                "Open",
                target.what.clone(),
                Action::Open(target.clone()),
            ),
            Item::new(
                'c',
                "Copy",
                target.target.clone(),
                Action::Copy {
                    text: target.target.clone(),
                    what: target.target.clone(),
                },
            ),
        ];
        self.prompt = Some(Prompt::Menu(Menu::new("Open outside lsmd?", items)));
    }

    pub(super) fn act(&mut self, action: Action) {
        match action {
            Action::Copy { text, what } => self.put(&text, &what),
            Action::ChooseCode => self.choose_code(),
            Action::Select => self.start_select(),
            Action::Open(target) => {
                self.flash = Some(match crate::open::open(&target) {
                    Ok(()) => format!("Opened {}", target.what),
                    Err(e) => format!("Couldn't open {}: {e}", target.what),
                });
            }
        }
    }

    /// Copies `text`, and says so: "Copied `what`".
    fn put(&mut self, text: &str, what: &str) {
        self.flash = Some(match clipboard::copy(text) {
            Ok(how) => format!("Copied {what} {how}"),
            Err(e) => format!("Couldn't copy: {e}"),
        });
    }

    /// The code block on screen, or with more than one, the one whose
    /// letters are typed.
    fn choose_code(&mut self) {
        let Some(doc) = self.current() else { return };
        let blocks = doc.code_on_screen();
        match blocks[..] {
            [] => self.flash = Some("No code block on screen".into()),
            [(i, _)] => self.copy_code(i),
            _ => {
                let hints = blocks
                    .iter()
                    .zip(hint_labels(blocks.len()))
                    .map(|(&(i, line), label)| (label, line, 0, i.to_string()))
                    .collect();
                doc.set_hints(hints);
                self.prompt = Some(Prompt::Hints {
                    typed: String::new(),
                    code: true,
                });
            }
        }
    }

    /// Copies code block `i`.
    pub(super) fn copy_code(&mut self, i: usize) {
        let Some(block) = self.current().and_then(|d| d.code_block(i)) else {
            return;
        };
        let (code, what) = (block.code.clone(), code_what(block));
        self.put(&code, &what);
    }

    /// The selected folder's path in the list, else the document's.
    fn path_to_copy(&self) -> Option<PathBuf> {
        let folder = self.selected_folder().zip(self.root.as_ref());
        folder
            .map(|(dir, root)| {
                dir.split('/')
                    .filter(|c| !c.is_empty())
                    .fold(root.clone(), |p, c| p.join(c))
            })
            .or_else(|| self.current_path())
    }

    /// Starts selecting at the top of the screen.
    fn start_select(&mut self) {
        let Some(doc) = self.current() else { return };
        if doc.pieces().is_empty() {
            self.flash = Some("Nothing to select".into());
            return;
        }
        let at = doc.piece_on_screen();
        self.select(at, at, false);
    }

    /// Shows pieces `anchor` to `cursor` selected.
    pub(super) fn select(&mut self, anchor: usize, cursor: usize, drag: bool) {
        let Some(doc) = self.current() else { return };
        doc.select_pieces(anchor, cursor);
        self.prompt = Some(Prompt::Select {
            anchor,
            cursor,
            drag,
        });
    }

    pub(super) fn end_select(&mut self) {
        if let Some(doc) = self.current() {
            doc.selection = None;
        }
    }

    pub(super) fn select_key(&mut self, key: KeyEvent, anchor: usize, cursor: usize) {
        let Some(doc) = self.current() else { return };
        let last = doc.pieces().len().saturating_sub(1);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let page = 10;
        let to = match key.code {
            KeyCode::Char('j') | KeyCode::Down if !shift => cursor + 1,
            KeyCode::Char('k') | KeyCode::Up if !shift => cursor.saturating_sub(1),
            KeyCode::Char('J') | KeyCode::Down | KeyCode::PageDown => cursor + page,
            KeyCode::Char('K') | KeyCode::Up | KeyCode::PageUp => cursor.saturating_sub(page),
            KeyCode::Char('g') | KeyCode::Home => 0,
            KeyCode::Char('G') | KeyCode::End => last,
            KeyCode::Char('c' | 'y') | KeyCode::Enter => {
                self.copy_selection(anchor, cursor);
                return;
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                self.end_select();
                return;
            }
            _ => cursor,
        };
        let mut to = to.min(last);
        // From outside a code block, all of it is taken in: step over it
        // in one go, not a line at a time with nothing changing.
        let block = |i| doc.code_block_of(i);
        let from = block(cursor);
        if from.is_some() && from != block(anchor) && to.abs_diff(cursor) == 1 {
            while block(to) == from && to > 0 && to < last {
                to = if to > cursor { to + 1 } else { to - 1 };
            }
        }
        self.select(anchor, to, false);
    }

    pub(super) fn copy_selection(&mut self, anchor: usize, cursor: usize) {
        self.end_select();
        let Some(doc) = self.current() else { return };
        let (text, n) = doc.pieces_source(anchor, cursor);
        self.put(&text, &lines(n));
    }

    pub(super) fn select_footer(&mut self, anchor: usize, cursor: usize) -> Line<'static> {
        let n = self
            .current()
            .map_or(0, |d| d.pieces_source(anchor, cursor).1);
        Line::from(vec![
            " Select: ".bold(),
            Span::raw("↑↓ j k").bold(),
            Span::raw(" more or less  "),
            Span::raw("c ⏎").bold(),
            Span::raw(format!(" copy {}  ", lines(n))),
            "esc cancel".dim(),
        ])
    }
}

/// "3 lines of sh".
fn code_what(block: &CodeBlock) -> String {
    let n = block.code.lines().count();
    match block.lang.as_str() {
        "" => lines(n),
        lang => format!("{} of {lang}", lines(n)),
    }
}

/// A Markdown link to `heading`, with the path from `root`:
/// `[Install](docs/setup.md#install)`. Without a heading, to the document.
fn link(
    doc: &Doc,
    heading: Option<usize>,
    path: Option<&Path>,
    root: Option<&Path>,
) -> Option<String> {
    let path = path.map(|p| {
        let rel = root.and_then(|r| p.strip_prefix(r).ok()).unwrap_or(p);
        rel.components()
            .map(|c| c.as_os_str().to_string_lossy().replace(' ', "%20"))
            .collect::<Vec<_>>()
            .join("/")
    });
    let heading = heading.and_then(|i| doc.headings().get(i));
    let text = match (heading, &path) {
        (Some(h), _) => h.text.clone(),
        (None, Some(p)) => p.rsplit('/').next().unwrap_or(p).replace("%20", " "),
        (None, None) => return None,
    };
    let target = format!(
        "{}{}",
        path.as_deref().unwrap_or(""),
        heading.map_or(String::new(), |h| format!("#{}", h.slug))
    );
    Some(format!(
        "[{}]({target})",
        text.replace('[', "\\[").replace(']', "\\]")
    ))
}
