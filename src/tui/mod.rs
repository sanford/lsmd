//! The interactive browser and reader.

mod links;
mod mouse;
mod nav;
mod picker;
mod search_all;

use crate::doc::{Doc, Side, SourceSide, Split};
use crate::files::{self, Entry};
use crate::index::{self, Index};
use crate::theme::Theme;
use crate::watch::Watch;
use crate::wrap;
use crate::{clipboard, editor};
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Padding, Paragraph};
use ratatui::{DefaultTerminal, Frame};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, SystemTime};

/// What to show.
pub enum Source {
    /// A document read from standard input: no file list.
    Text { title: String, md: String },
    /// The Markdown files under `root`, optionally opening `open` straight away.
    Browse {
        root: PathBuf,
        open: Option<PathBuf>,
        all: bool,
    },
}

/// How documents are shown.
pub struct Settings {
    /// The widest to wrap text.
    pub max_width: Option<usize>,
    /// Open documents with their source alongside.
    pub split: bool,
    pub source_side: SourceSide,
    /// Scroll and click with the mouse. (Selecting text then needs a
    /// modifier key in most terminals.)
    pub mouse: bool,
    /// Sort the list newest first.
    pub by_date: bool,
}

pub fn run(source: Source, theme: &Theme, settings: Settings) -> io::Result<()> {
    let mut app = App::new(source, theme, settings.max_width);
    app.split = settings.split;
    app.source_right = settings.source_side == SourceSide::Right;
    let mut terminal = ratatui::init();
    app.mouse_on = settings.mouse;
    app.by_time = settings.by_date;
    set_mouse(app.mouse_on, true);
    if app.mouse_on {
        // ratatui's panic hook restores the terminal, but doesn't know
        // about the mouse: turn it off first, or the shell gets mouse codes.
        let restore = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            set_mouse(true, false);
            restore(info);
        }));
    }
    let result = app.run(&mut terminal);
    set_mouse(app.mouse_on, false);
    ratatui::restore();
    result
}

/// Turns mouse reporting on or off, if lsmd uses the mouse.
fn set_mouse(used: bool, on: bool) {
    use ratatui::crossterm::event::{DisableMouseCapture, EnableMouseCapture};
    use ratatui::crossterm::execute;
    if used {
        let _ = if on {
            execute!(io::stdout(), EnableMouseCapture)
        } else {
            execute!(io::stdout(), DisableMouseCapture)
        };
    }
}

#[derive(PartialEq, Eq)]
enum Focus {
    List,
    Reader,
}

/// A file that passes the filter, with the positions of the matched
/// characters in its path.
struct Shown {
    file: usize,
    hits: Vec<u32>,
}

struct App<'t> {
    theme: &'t Theme,
    max_width: Option<usize>,

    /// Standard input's document, when there's no file list.
    text: Option<(String, Doc)>,
    root_label: String,
    files: Vec<Entry>,
    scan: Option<Receiver<Vec<Entry>>>,
    /// The directory being browsed.
    root: Option<PathBuf>,
    /// List hidden and ignored files too.
    all: bool,
    /// Watches the root and open documents for changes.
    watch: Option<Watch>,
    /// Who links to whom, once it's built.
    index: Option<Index>,
    indexing: Option<Receiver<Index>>,
    by_time: bool,
    filter: String,
    typing: bool,
    matcher: Matcher,
    shown: Vec<Shown>,
    list: ListState,
    list_height: usize,
    /// Where the list's rows were drawn (empty when it isn't shown).
    list_area: Rect,
    /// The user has moved the selection since the list last changed order.
    moved: bool,
    /// A file to select as soon as the scan finds it.
    want: Option<PathBuf>,

    focus: Focus,
    /// The file open in the reader.
    reading: Option<PathBuf>,
    /// Keep the file list on screen while reading.
    list_in_reader: bool,
    help: bool,
    /// Show the source beside the rendered document.
    split: bool,
    /// In the split view, the source side has the keyboard.
    source_focus: bool,
    /// The source side's share of the split, in percent.
    ratio: u16,
    source_right: bool,
    /// A prompt or popup that has the keyboard, in the reader.
    prompt: Option<nav::Prompt>,
    /// Where following links has come from, to go back to.
    history: Vec<nav::Place>,
    /// A message for the footer, until the next key.
    flash: Option<String>,
    /// A file to open in the editor, at a line, once the key's handled.
    edit: Option<(PathBuf, usize)>,
    /// The mouse is in use.
    mouse_on: bool,
    /// A search of every file under way, and its query.
    grep: Option<(Receiver<Vec<crate::grep::Hit>>, String)>,
    docs: HashMap<PathBuf, Doc>,
}

impl<'t> App<'t> {
    fn new(source: Source, theme: &'t Theme, max_width: Option<usize>) -> App<'t> {
        let mut app = App {
            theme,
            max_width,
            text: None,
            root_label: String::new(),
            files: Vec::new(),
            scan: None,
            root: None,
            all: false,
            watch: None,
            index: None,
            indexing: None,
            by_time: false,
            filter: String::new(),
            typing: false,
            matcher: Matcher::new(Config::DEFAULT.match_paths()),
            shown: Vec::new(),
            list: ListState::default(),
            list_height: 0,
            list_area: Rect::default(),
            moved: false,
            want: None,
            focus: Focus::List,
            reading: None,
            list_in_reader: false,
            help: false,
            split: false,
            source_focus: false,
            ratio: 50,
            source_right: true,
            prompt: None,
            history: Vec::new(),
            flash: None,
            edit: None,
            grep: None,
            mouse_on: false,
            docs: HashMap::new(),
        };
        match source {
            Source::Text { title, md } => {
                app.text = Some((title, Doc::new(md)));
                app.focus = Focus::Reader;
                app.watch = Watch::new();
            }
            Source::Browse { root, open, all } => {
                app.root_label = display_path(&root);
                app.scan = Some(files::scan(&root, all));
                app.watch = Watch::new();
                if let Some(watch) = &mut app.watch {
                    watch.add(&root);
                }
                app.root = Some(root);
                app.all = all;
                if let Some(path) = open {
                    app.focus = Focus::Reader;
                    app.reading = Some(path.clone());
                    app.want = Some(path);
                }
            }
        }
        app
    }

    fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        loop {
            self.receive();
            terminal.draw(|f| self.draw(f))?;
            // While scanning or indexing, wake up now and then to show
            // what's new.
            let wait = if self.scan.is_some() || self.indexing.is_some() || self.grep.is_some() {
                Duration::from_millis(50)
            } else if self.watch.is_some() {
                Duration::from_millis(250)
            } else {
                Duration::from_secs(60)
            };
            if !event::poll(wait)? {
                continue;
            }
            let key = match event::read()? {
                Event::Key(key) => key,
                Event::Mouse(m) => {
                    self.flash = None;
                    self.mouse(m);
                    continue;
                }
                _ => continue, // Resizes redraw at the top of the loop.
            };
            if key.kind == KeyEventKind::Press && self.key(key) {
                return Ok(());
            }
            if let Some((path, line)) = self.edit.take() {
                // Hand the terminal to the editor until it's done.
                set_mouse(self.mouse_on, false);
                ratatui::restore();
                let result = editor::edit(&path, line);
                *terminal = ratatui::init();
                set_mouse(self.mouse_on, true);
                terminal.clear()?;
                if let Err(e) = result {
                    self.flash = Some(format!("Couldn't edit: {e}"));
                }
            }
        }
    }

    /// Takes in files the scan has found since last time, and the link
    /// index when it's ready.
    fn receive(&mut self) {
        self.reload();
        self.receive_grep();
        if let Some(rx) = &self.indexing
            && let Ok(index) = rx.try_recv()
        {
            self.index = Some(index);
            self.indexing = None;
        }
        let Some(rx) = &self.scan else { return };
        let mut changed = false;
        loop {
            match rx.try_recv() {
                Ok(batch) => {
                    self.files.extend(batch);
                    changed = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.scan = None;
                    changed = true;
                    if let Some(root) = &self.root {
                        self.indexing = Some(index::build(root, self.files.clone()));
                    }
                    break;
                }
            }
        }
        if changed {
            self.refresh();
        }
    }

    fn selected(&self) -> Option<&Entry> {
        let shown = self.shown.get(self.list.selected()?)?;
        Some(&self.files[shown.file])
    }

    /// Re-sorts and re-filters the list, keeping the same file selected.
    ///
    /// Until the user moves, the selection stays on the first file (or the
    /// file named on the command line), however the list changes.
    fn refresh(&mut self) {
        let chosen = self
            .moved
            .then(|| self.selected().map(|e| e.path.clone()))
            .flatten();
        let keep = self.want.clone().or(chosen);
        let mut order: Vec<usize> = (0..self.files.len()).collect();
        let cmp = if self.by_time {
            files::by_modified
        } else {
            files::by_path
        };
        order.sort_by(|&a, &b| cmp(&self.files[a], &self.files[b]));

        if self.filter.is_empty() {
            self.shown = order
                .into_iter()
                .map(|file| Shown {
                    file,
                    hits: Vec::new(),
                })
                .collect();
        } else {
            let pattern = Pattern::parse(&self.filter, CaseMatching::Smart, Normalization::Smart);
            let mut buf = Vec::new();
            let mut scored = Vec::new();
            for file in order {
                let mut hits = Vec::new();
                let hay = Utf32Str::new(&self.files[file].rel, &mut buf);
                if let Some(score) = pattern.indices(hay, &mut self.matcher, &mut hits) {
                    hits.sort_unstable();
                    hits.dedup();
                    scored.push((score, Shown { file, hits }));
                }
            }
            scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
            self.shown = scored.into_iter().map(|(_, s)| s).collect();
        }

        let index = keep.and_then(|p| self.shown.iter().position(|s| self.files[s.file].path == p));
        if index.is_some() && self.want.take().is_some() {
            // Found the file we were waiting for: from now on it counts as
            // chosen, and later batches mustn't move off it.
            self.moved = true;
        }
        self.list
            .select(index.or((!self.shown.is_empty()).then_some(0)));
    }

    /// The document for `path`, loading it the first time. (Changes to
    /// it are picked up by [`App::reload`].)
    fn doc(&mut self, path: &Path) -> &mut Doc {
        if !self.docs.contains_key(path) {
            // Files outside the root aren't watched yet.
            if let Some(watch) = &mut self.watch {
                watch.add(path);
            }
            self.docs.insert(path.to_path_buf(), Doc::load(path));
        }
        self.docs.get_mut(path).unwrap()
    }

    /// Catches up with files that have changed: open documents re-render
    /// in place, and the file list and link index follow.
    fn reload(&mut self) {
        let Some(watch) = &self.watch else { return };
        let changed = watch.changed();
        let mut list_changed = false;
        for path in changed.into_iter().filter(|p| files::is_markdown(p)) {
            let exists = path.is_file();
            if let Some(doc) = self.docs.get_mut(&path)
                && exists
            {
                doc.reload(&path);
            }
            let Some(rel) = self.rel_of(&path) else {
                continue;
            };
            if let Some(index) = &mut self.index {
                index.refresh(&rel, &path);
            }
            // New files in hidden directories stay out, as in the scan.
            if !self.all && rel.split('/').any(|part| part.starts_with('.')) {
                continue;
            }
            let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
            match self.files.iter().position(|e| e.path == path) {
                Some(i) if exists => self.files[i].modified = modified,
                Some(i) => {
                    self.files.remove(i);
                }
                None if exists => self.files.push(Entry {
                    path,
                    rel,
                    modified,
                }),
                None => continue,
            }
            list_changed = true;
        }
        if list_changed {
            self.refresh();
        }
    }

    /// The document on screen: the one being read, or else the selected one.
    fn current(&mut self) -> Option<&mut Doc> {
        let path = match self.focus {
            Focus::Reader => self.reading.clone(),
            Focus::List => self.selected().map(|e| e.path.clone()),
        };
        match path {
            Some(path) => Some(self.doc(&path)),
            None => self.text.as_mut().map(|(_, doc)| doc),
        }
    }

    fn select_by(&mut self, delta: isize) {
        if self.shown.is_empty() {
            return;
        }
        self.moved = true;
        let i = self
            .list
            .selected()
            .unwrap_or(0)
            .saturating_add_signed(delta);
        self.list.select(Some(i.min(self.shown.len() - 1)));
    }

    fn start_edit(&mut self) {
        let Some(path) = self.current_path() else {
            self.flash = Some("Standard input isn't a file to edit".into());
            return;
        };
        let line = self.current().map_or(1, |d| d.source_line());
        self.edit = Some((path, line));
    }

    fn copy_code(&mut self) {
        let Some(block) = self.current().and_then(|d| d.code_on_screen()) else {
            self.flash = Some("No code block on screen".into());
            return;
        };
        let lines = block.code.lines().count();
        let what = match block.lang.as_str() {
            "" => format!("{lines} line{}", if lines == 1 { "" } else { "s" }),
            lang => format!(
                "{lines} line{} of {lang}",
                if lines == 1 { "" } else { "s" }
            ),
        };
        let code = block.code.clone();
        self.flash = Some(match clipboard::copy(&code) {
            Ok(how) => format!("Copied {what} {how}"),
            Err(e) => format!("Couldn't copy: {e}"),
        });
    }

    fn copy_path(&mut self) {
        let Some(path) = self.current_path() else {
            self.flash = Some("Standard input has no path".into());
            return;
        };
        let path = path.display().to_string();
        self.flash = Some(match clipboard::copy(&path) {
            Ok(how) => format!("Copied {path} {how}"),
            Err(e) => format!("Couldn't copy: {e}"),
        });
    }

    /// The side of the document that scrolls.
    fn side(&self) -> Side {
        if self.split && self.source_focus {
            Side::Source
        } else {
            Side::Rendered
        }
    }

    /// Keys that work the same in the list and the reader.
    fn view_key(&mut self, key: KeyEvent, ctrl: bool) -> bool {
        match key.code {
            KeyCode::Char('e') if !ctrl => self.start_edit(),
            KeyCode::Char('y') if !ctrl => self.copy_code(),
            KeyCode::Char('Y') => self.copy_path(),
            KeyCode::Char('s') if !ctrl => {
                self.prompt = Some(nav::Prompt::Grep {
                    query: String::new(),
                });
            }
            KeyCode::Tab => self.split = !self.split,
            KeyCode::BackTab if self.split => self.source_focus = !self.source_focus,
            KeyCode::Char('w') if ctrl && self.split => self.source_focus = !self.source_focus,
            // Move the divider, whichever side the source is on.
            KeyCode::Char(c @ ('<' | '>')) if self.split => {
                let wider = (c == '>') != self.source_right;
                let ratio = if wider {
                    self.ratio + 5
                } else {
                    self.ratio - 5
                };
                self.ratio = ratio.clamp(20, 80);
            }
            _ => return false,
        }
        true
    }

    /// Handles a key. Returns true to quit.
    fn key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            return true;
        }
        self.flash = None;
        if self.help {
            self.help = false;
            return false;
        }
        if let Some(prompt) = self.prompt.take() {
            self.prompt_key(prompt, key, ctrl);
            return false;
        }
        if self.typing {
            self.filter_key(key, ctrl);
            return false;
        }
        match key.code {
            KeyCode::Char('Q') => return true,
            KeyCode::Char('?') => self.help = true,
            _ if self.view_key(key, ctrl) => {}
            _ if self.focus == Focus::List => return self.list_key(key, ctrl),
            _ => return self.reader_key(key, ctrl),
        }
        false
    }

    fn filter_key(&mut self, key: KeyEvent, ctrl: bool) {
        match key.code {
            KeyCode::Down => return self.select_by(1),
            KeyCode::Up => return self.select_by(-1),
            KeyCode::Char('n') if ctrl => return self.select_by(1),
            KeyCode::Char('p') if ctrl => return self.select_by(-1),
            KeyCode::Enter => return self.typing = false,
            KeyCode::Esc => {
                self.typing = false;
                self.filter.clear();
            }
            KeyCode::Backspace => {
                if self.filter.pop().is_none() {
                    self.typing = false;
                }
            }
            KeyCode::Char(c) if !ctrl => self.filter.push(c),
            _ => return,
        }
        // Jump to the best match.
        self.moved = false;
        self.refresh();
    }

    fn list_key(&mut self, key: KeyEvent, ctrl: bool) -> bool {
        let page = self.list_height.max(1) as isize;
        match key.code {
            KeyCode::Char('q') => return true,
            KeyCode::Char('d') if ctrl => self.select_by(page / 2),
            KeyCode::Char('u') if ctrl => self.select_by(-page / 2),
            KeyCode::Char('j') | KeyCode::Down => self.select_by(1),
            KeyCode::Char('k') | KeyCode::Up => self.select_by(-1),
            KeyCode::PageDown => self.select_by(page),
            KeyCode::PageUp => self.select_by(-page),
            KeyCode::Char('g') | KeyCode::Home => self.select_by(isize::MIN / 2),
            KeyCode::Char('G') | KeyCode::End => self.select_by(isize::MAX / 2),
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
                if let Some(e) = self.selected() {
                    self.reading = Some(e.path.clone());
                    self.focus = Focus::Reader;
                }
            }
            KeyCode::Char('/') => self.typing = true,
            KeyCode::Esc if !self.filter.is_empty() => {
                self.filter.clear();
                self.refresh();
            }
            KeyCode::Esc => return true,
            KeyCode::Char('L') => self.open_links(),
            KeyCode::Char('m') => {
                self.by_time = !self.by_time;
                self.refresh();
            }
            // Page through the preview without opening it.
            KeyCode::Char(' ') => {
                let side = self.side();
                if let Some(doc) = self.current() {
                    doc.scroll_by(doc.page(), side);
                }
            }
            KeyCode::Char('b') => {
                let side = self.side();
                if let Some(doc) = self.current() {
                    doc.scroll_by(-doc.page(), side);
                }
            }
            _ => {}
        }
        false
    }

    fn reader_key(&mut self, key: KeyEvent, ctrl: bool) -> bool {
        let browsing = self.text.is_none();
        if !ctrl && self.nav_key(key) {
            return false;
        }
        // Esc and Backspace with somewhere to go back to were handled above.
        match key.code {
            KeyCode::Char('q') => return true,
            KeyCode::Esc if !browsing => return true,
            KeyCode::Esc | KeyCode::Backspace if browsing => {
                self.focus = Focus::List;
                return false;
            }
            KeyCode::Char('\\') if browsing => {
                self.list_in_reader = !self.list_in_reader;
                return false;
            }
            _ => {}
        }
        let side = self.side();
        let Some(doc) = self.current() else {
            return false;
        };
        let page = doc.page();
        let half = (page / 2).max(1);
        match key.code {
            KeyCode::Char('d') if ctrl => doc.scroll_by(half, side),
            KeyCode::Char('u') if ctrl => doc.scroll_by(-half, side),
            KeyCode::Char('f') if ctrl => doc.scroll_by(page, side),
            KeyCode::Char('b') if ctrl => doc.scroll_by(-page, side),
            KeyCode::Char('j') | KeyCode::Down | KeyCode::Enter => doc.scroll_by(1, side),
            KeyCode::Char('k') | KeyCode::Up => doc.scroll_by(-1, side),
            KeyCode::Char('d') => doc.scroll_by(half, side),
            KeyCode::Char('u') => doc.scroll_by(-half, side),
            KeyCode::Char(' ') | KeyCode::PageDown => doc.scroll_by(page, side),
            KeyCode::Char('b') | KeyCode::PageUp => doc.scroll_by(-page, side),
            KeyCode::Char('g') | KeyCode::Home => doc.scroll_to_top(side),
            KeyCode::Char('G') | KeyCode::End => doc.scroll_to_bottom(side),
            KeyCode::Char('h') | KeyCode::Left => doc.scroll_sideways(-8),
            KeyCode::Char('l') | KeyCode::Right => doc.scroll_sideways(8),
            KeyCode::Char('0') => doc.scroll_sideways(isize::MIN / 2),
            _ => {}
        }
        false
    }

    fn draw(&mut self, f: &mut Frame) {
        let [header, body, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .areas(f.area());

        self.list_area = Rect::default();
        let reader_only =
            self.text.is_some() || (self.focus == Focus::Reader && !self.list_in_reader);
        let narrow = body.width < 80;
        if reader_only || (narrow && self.focus == Focus::Reader) {
            // One column of margin on each side.
            let area = Rect {
                x: body.x + 1,
                width: body.width.saturating_sub(2),
                ..body
            };
            self.draw_doc(f, area);
        } else if narrow {
            self.draw_list(f, body);
        } else {
            let widest = self
                .shown
                .iter()
                .map(|s| wrap::width(&self.files[s.file].rel))
                .max();
            let want = widest.unwrap_or(0) + 11; // time column, gaps, borders
            let cap = usize::from(body.width) * 2 / 5;
            let list_w = want.clamp(24, cap.max(24)) as u16;
            let [list_area, doc_area] =
                Layout::horizontal([Constraint::Length(list_w), Constraint::Min(0)]).areas(body);
            self.draw_list(f, list_area);
            self.draw_preview(f, doc_area);
        }

        // After the document, so it's laid out and its links are known.
        self.draw_header(f, header);
        self.draw_footer(f, footer);
        self.draw_picker(f);
        if self.help {
            draw_help(f);
        }
    }

    fn draw_header(&mut self, f: &mut Frame, area: Rect) {
        let summary = if let Some((title, _)) = &self.text {
            title.clone()
        } else {
            let n = self.files.len();
            let mut s = format!(
                "{n} file{} in {}",
                if n == 1 { "" } else { "s" },
                self.root_label
            );
            if self.scan.is_some() {
                s.push_str(" …");
            }
            if self.focus == Focus::Reader
                && let Some(path) = &self.reading
            {
                s = display_path(path);
            }
            s
        };
        let note = self.link_note();
        let note_w = note.as_ref().map_or(0, |n| n.width() as u16);
        let [title_area, note_area] =
            Layout::horizontal([Constraint::Min(0), Constraint::Length(note_w)]).areas(area);
        if let Some(note) = note {
            f.render_widget(Paragraph::new(note), note_area);
        }
        f.render_widget(
            Paragraph::new(Line::from(vec![
                " lsmd ".bold(),
                Span::raw(" "),
                summary.dim(),
            ])),
            title_area,
        );
    }

    fn pane(&self, title: String, focused: bool) -> Block<'static> {
        let block = Block::bordered().title(title);
        if focused {
            block
        } else {
            block.border_style(Style::new().dim())
        }
    }

    fn draw_list(&mut self, f: &mut Frame, area: Rect) {
        let title = if self.filter.is_empty() {
            format!(" Files ({}) ", self.files.len())
        } else {
            format!(" Files ({} of {}) ", self.shown.len(), self.files.len())
        };
        let block = self.pane(title, self.focus == Focus::List);
        let inner = block.inner(area);
        self.list_height = inner.height.into();
        self.list_area = inner;
        let now = SystemTime::now();
        let width = usize::from(inner.width);
        let items: Vec<ListItem> = self
            .shown
            .iter()
            .map(|s| {
                let e = &self.files[s.file];
                ListItem::new(list_line(
                    &e.rel,
                    &s.hits,
                    &files::ago(e.modified, now),
                    width,
                ))
            })
            .collect();
        let list = List::new(items)
            .block(block)
            .highlight_style(Style::new().add_modifier(Modifier::REVERSED));
        f.render_stateful_widget(list, area, &mut self.list);

        if self.shown.is_empty() && self.scan.is_none() {
            let msg = if self.files.is_empty() {
                "No Markdown files here"
            } else {
                "Nothing matches"
            };
            f.render_widget(Paragraph::new(format!(" {msg}")).dim(), inner);
        }
    }

    fn draw_preview(&mut self, f: &mut Frame, area: Rect) {
        let path = match self.focus {
            Focus::Reader => self.reading.clone(),
            Focus::List => self.selected().map(|e| e.path.clone()),
        };
        let title = path
            .as_deref()
            .map(|p| format!(" {} ", self.rel_label(p)))
            .unwrap_or_default();
        let block = self
            .pane(title, self.focus == Focus::Reader)
            .padding(Padding::horizontal(1));
        let inner = block.inner(area);
        f.render_widget(block, area);
        self.draw_doc(f, inner);
    }

    fn draw_doc(&mut self, f: &mut Frame, area: Rect) {
        let mut width = usize::from(area.width);
        if let Some(max) = self.max_width {
            width = width.min(max);
        }
        let theme = self.theme;
        let split = self.split.then(|| Split {
            ratio: self.ratio,
            focus: self.side(),
            source_right: self.source_right,
            max_width: self.max_width,
        });
        if let Some(doc) = self.current() {
            if let Some(split) = split {
                doc.draw_split(f, area, &split, theme);
            } else {
                doc.draw(f, area, width, theme);
            }
        }
    }

    /// `path` relative to the root when it's in the list.
    fn rel_label(&self, path: &Path) -> String {
        self.files
            .iter()
            .find(|e| e.path == path)
            .map_or_else(|| display_path(path), |e| e.rel.clone())
    }

    fn draw_footer(&mut self, f: &mut Frame, area: Rect) {
        if let Some(line) = self.prompt_footer() {
            f.render_widget(Paragraph::new(line), area);
            return;
        }
        if let Some(msg) = &self.flash {
            f.render_widget(Paragraph::new(Line::from(format!(" {msg}")).yellow()), area);
            return;
        }
        if self.typing {
            let line = Line::from(vec![
                " /".bold(),
                Span::raw(self.filter.clone()),
                "▏".slow_blink(),
            ]);
            f.render_widget(Paragraph::new(line), area);
            return;
        }
        // Esc clears a search before it goes back.
        let searching = self.current().is_some_and(|d| d.search_status().is_some());
        let back = if searching {
            "clear search".into()
        } else {
            self.back_label()
        };
        let tab = ("tab", if self.split { "hide source" } else { "source" });
        let mut reader = vec![
            ("↑↓", "scroll"),
            ("/", "search"),
            ("f", "follow"),
            ("o", "outline"),
            tab,
        ];
        if self.split {
            reader.push((
                "^w",
                if self.source_focus {
                    "to rendered"
                } else {
                    "to source"
                },
            ));
        }
        let keys: Vec<(&str, &str)> = match self.focus {
            Focus::List => {
                let mut keys = vec![
                    ("↑↓", "move"),
                    ("⏎", "read"),
                    ("/", "filter"),
                    ("s", "search text"),
                    tab,
                ];
                if !self.filter.is_empty() {
                    keys.push(("esc", "clear filter"));
                }
                keys.push((
                    "m",
                    if self.by_time {
                        "sort by name"
                    } else {
                        "sort by date"
                    },
                ));
                keys.extend([("?", "help"), ("q", "quit")]);
                keys
            }
            Focus::Reader => {
                reader.extend([("esc", back.as_str()), ("?", "help"), ("q", "quit")]);
                reader
            }
        };
        let mut spans = vec![Span::raw(" ")];
        for (k, what) in keys {
            spans.push(k.bold());
            spans.push(Span::raw(format!(" {what}  ")).dim());
        }
        // The search's match count, while there is one, then the position.
        let position = self
            .current()
            .map(|d| match d.search_status() {
                Some(matches) => format!("{matches} matches  {}", d.position()),
                None => d.position(),
            })
            .unwrap_or_default();
        let pos_width = wrap::width(&position) as u16 + 1;
        let [keys_area, pos_area] =
            Layout::horizontal([Constraint::Min(0), Constraint::Length(pos_width)]).areas(area);
        f.render_widget(Paragraph::new(Line::from(spans)), keys_area);
        f.render_widget(
            Paragraph::new(Line::from(format!("{position} ")).dim()).right_aligned(),
            pos_area,
        );
    }
}

/// One row of the file list: the path, with its directory dimmed and filter
/// matches highlighted, and the age right-aligned. Long paths lose their
/// start, so the file name stays visible.
fn list_line(rel: &str, hits: &[u32], age: &str, width: usize) -> Line<'static> {
    let chars: Vec<char> = rel.chars().collect();
    let dir_len = rel.rfind('/').map_or(0, |i| rel[..=i].chars().count());
    let age_w = wrap::width(age);
    let room = width.saturating_sub(age_w + 2).max(1);

    // Drop characters from the front until the rest fits, leaving room for "…".
    let mut skip = 0;
    let mut path_w: usize = chars.iter().map(|&c| char_width(c)).sum();
    if path_w > room {
        while skip < chars.len() && path_w + 1 > room {
            path_w -= char_width(chars[skip]);
            skip += 1;
        }
        path_w += 1;
    }

    let hit = Style::new().yellow().bold();
    let mut spans: Vec<Span> = vec![Span::raw(" ")];
    if skip > 0 {
        spans.push("…".dim());
    }
    for (i, &c) in chars.iter().enumerate().skip(skip) {
        let style = if hits.binary_search(&(i as u32)).is_ok() {
            hit
        } else if i < dir_len {
            Style::new().dim()
        } else {
            Style::new()
        };
        match spans.last_mut() {
            Some(last) if last.style == style => last.content.to_mut().push(c),
            _ => spans.push(Span::styled(c.to_string(), style)),
        }
    }
    let gap = width.saturating_sub(1 + path_w + age_w);
    if age_w > 0 && gap > 0 {
        spans.push(Span::raw(" ".repeat(gap)));
        spans.push(Span::raw(age.to_string()).dim());
    }
    Line::from(spans)
}

fn char_width(c: char) -> usize {
    wrap::width(c.encode_utf8(&mut [0; 4]))
}

fn draw_help(f: &mut Frame) {
    const KEYS: &[(&str, &str)] = &[
        ("↑↓ j k", "Move / scroll"),
        ("⏎ l →", "Read the selected file"),
        (
            "esc ⌫",
            "Back to the last document, then the list; quits from the list",
        ),
        ("q", "Quit"),
        ("← → h l", "Scroll long code lines sideways (0: back)"),
        ("] [", "Next / previous heading"),
        ("o", "Outline: jump to a heading"),
        ("L", "Links: what this links to, and what links here"),
        ("s", "Search the text of every file"),
        ("e", "Edit the file in $EDITOR, at this point"),
        ("y Y", "Copy the code block on screen / the file's path"),
        ("/ n N", "Search; next / previous match"),
        ("f", "Follow a link (type the letters shown on it)"),
        ("space b", "Page down / up (the preview, in the list)"),
        ("d u", "Half page down / up"),
        ("g G", "Top / bottom"),
        ("/", "Filter files (fuzzy)"),
        ("m", "Sort by name or by date"),
        ("\\", "Show or hide the list while reading"),
        ("tab", "Show the source beside the rendered text"),
        ("^w ⇧tab", "Switch between source and rendered"),
        ("< >", "Move the divider left / right"),
        ("Q", "Quit from anywhere"),
    ];
    let key_width = KEYS.iter().map(|(k, _)| wrap::width(k)).max().unwrap_or(0);
    let mut lines: Vec<Line> = KEYS
        .iter()
        .map(|(k, what)| {
            let pad = key_width - wrap::width(k);
            Line::from(vec![
                format!("{k}{}   ", " ".repeat(pad)).bold(),
                Span::raw(*what),
            ])
        })
        .collect();
    lines.push(Line::default());
    lines.push(Line::from("Press any key to close").dim());
    let area = f.area();
    let width = (lines.iter().map(Line::width).max().unwrap_or(0) as u16 + 4).min(area.width);
    let height = (lines.len() as u16 + 2).min(area.height);
    let rect = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .title(" Keys ")
                .padding(Padding::horizontal(1)),
        ),
        rect,
    );
}

/// `path` with the home directory shown as `~`.
pub fn display_path(path: &Path) -> String {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    if let Some(home) = home
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return if rest.as_os_str().is_empty() {
            "~".into()
        } else {
            format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display())
        };
    }
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn scanned<'t>(root: &Path, open: Option<PathBuf>, theme: &'t Theme) -> App<'t> {
        let mut app = App::new(
            Source::Browse {
                root: root.to_path_buf(),
                open,
                all: false,
            },
            theme,
            None,
        );
        while app.scan.is_some() {
            app.receive();
            std::thread::sleep(Duration::from_millis(5));
        }
        app
    }

    #[test]
    fn selects_the_file_named_on_the_command_line() {
        let dir = std::env::temp_dir().join(format!("lsmd-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for f in ["a.md", "b.md", "sub/c.md"] {
            std::fs::write(dir.join(f), "# x\n").unwrap();
        }
        let dir = std::fs::canonicalize(&dir).unwrap();
        let theme = Theme::plain();

        let app = scanned(&dir, Some(dir.join("sub/c.md")), &theme);
        assert_eq!(app.selected().unwrap().rel, "sub/c.md");

        let app = scanned(&dir, None, &theme);
        assert_eq!(app.selected().unwrap().rel, "a.md");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn list_line_right_aligns_age() {
        assert_eq!(
            text(&list_line("docs/a.md", &[], "2h ago", 20)),
            " docs/a.md    2h ago"
        );
    }

    #[test]
    fn list_line_trims_long_paths_from_the_front() {
        let line = list_line("very/long/directory/name.md", &[], "1d ago", 20);
        assert_eq!(text(&line), " …ory/name.md 1d ago");
    }
}
