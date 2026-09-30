//! The interactive browser and reader.

mod copy;
mod found;
mod links;
mod menu;
mod mouse;
mod nav;
mod outline;
mod picker;
mod search_all;
mod themes;
mod tree;

use crate::doc::{Doc, Side, SourceSide, Split};
use crate::editor;
use crate::files::{self, Entry};
use crate::index::{self, Index};
use crate::omarchy::Follow;
use crate::theme::{Choice, Mode, Theme};
use crate::watch::Watch;
use crate::wrap;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Padding, Paragraph};
use ratatui::{DefaultTerminal, Frame};
use ratatui_image::picker::Picker;
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant, SystemTime};

/// A terminal this wide keeps the file list beside a document `Tab` opens.
const WIDE: u16 = 130;

/// How long pictures wait after moving before they're drawn in full.
const FIGURE_SETTLE: Duration = Duration::from_millis(150);
/// The most pictures kept ready to draw.
const MAX_DRAWN: usize = 32;

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
    /// Open documents with the outline pane beside them.
    pub outline: bool,
    /// Show diagrams as pictures, where the terminal can.
    pub images: bool,
    /// How many lines `j` and `k` scroll.
    pub scroll: usize,
    /// Draw headings big, where the terminal can.
    pub big_headings: bool,
    /// The theme asked for, by flag or config.
    pub choice: Choice,
    /// Colors come from Omarchy's theme, and follow it.
    pub omarchy: bool,
}

pub fn run(source: Source, theme: Theme, settings: Settings) -> io::Result<()> {
    // Previewing `auto` in the theme picker needs the terminal's background,
    // and it can't be asked once the screen is ours.
    let terminal_dark = match settings.choice {
        Choice::Mode(Mode::Auto) if !settings.omarchy => theme.dark,
        _ if theme.color && !settings.omarchy => crate::theme::detect_dark(),
        _ => true,
    };
    let mut app = App::new(source, theme, settings.max_width);
    app.choice = settings.choice;
    app.terminal_dark = terminal_dark;
    if settings.omarchy {
        app.follow = Follow::start();
    }
    app.omarchy = settings.omarchy;
    app.split = settings.split;
    app.source_right = settings.source_side == SourceSide::Right;
    let mut terminal = ratatui::init();
    if settings.images && app.theme.color {
        app.picker = crate::figure::picker();
        if app.picker.is_some() {
            crate::picture::enable();
        }
    }
    if settings.big_headings && crate::sizing::detect() {
        crate::sizing::enable();
    }
    // A terminal may print some of the questions it doesn't know: draw the
    // whole screen afresh over them.
    terminal.clear()?;
    app.mouse_on = settings.mouse;
    app.by_time = settings.by_date;
    app.outline_pane = settings.outline;
    app.scroll = settings.scroll;
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

/// A symbol no cell drawn has: see `frame`.
const NOT_ON_SCREEN: &str = "\u{FFFF}";

/// Stops lsmd, as Ctrl-Z does outside raw mode, till the shell continues it
/// with `fg`.
fn stop() {
    #[cfg(unix)]
    // SAFETY: raise only sends a signal to this process.
    unsafe {
        libc::raise(libc::SIGTSTP);
    }
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

/// A row of the file list: a file that passes the filter, or in the tree,
/// a folder.
enum Shown {
    File {
        file: usize,
        /// The positions of the matched characters in what the row shows.
        hits: Vec<u32>,
        /// Where in its path the row starts: below the folder being
        /// listed, or in the tree, at its name.
        from: usize,
        depth: usize,
        /// Lines with the filter in them, for a file found by its text
        /// rather than its name; else 0.
        found: usize,
    },
    Dir(tree::Dir),
    /// `..`, inside a folder: up to this one.
    Up(String),
    /// A line between the folders (and the files beside them) and the files
    /// in the folders, listed flat; or before the files found by their text,
    /// labelled. It can't be selected.
    Rule(&'static str),
}

struct App {
    theme: Rc<Theme>,
    /// The theme shown: what was asked for, or what's being previewed.
    choice: Choice,
    /// The theme and choice from before the theme picker opened, while it's
    /// open.
    theme_before: Option<(Rc<Theme>, Choice)>,
    terminal_dark: bool,
    omarchy: bool,
    follow: Option<Follow>,
    max_width: Option<usize>,

    /// Standard input's document, when there's no file list.
    text: Option<(String, Doc)>,
    root_label: String,
    files: Vec<Entry>,
    scan: Option<Receiver<Vec<Entry>>>,
    /// The directory being browsed.
    root: Option<PathBuf>,
    /// The top of its repository, where links starting with `/` start.
    site_root: Option<PathBuf>,
    /// List hidden and ignored files too.
    all: bool,
    /// List hidden files (`.` toggles it).
    hidden: bool,
    /// Files live reload added while the scan was still running.
    live_added: HashSet<PathBuf>,
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
    /// The files in the list, in order, that pass the filter, whether or
    /// not their folders are open.
    listed: Vec<usize>,
    /// How many files are in the folder being listed.
    in_scope: usize,
    /// How wide the folders' counts are (see [`App::count_width`]), kept
    /// up to date by [`App::refresh`] rather than worked out every row.
    count_w: usize,
    /// Folders open in the tree, by path, ending in `/`.
    open: HashSet<String>,
    /// The folder being listed, ending in `/`, or `""` for the root.
    scope: String,
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
    /// The file selected when `Tab` left the reader for the list: until
    /// another is selected, the list shows what was being read, and `Tab`
    /// goes back to it.
    left_on: Option<PathBuf>,
    /// How wide the screen was drawn, below the header.
    body_width: u16,
    /// Show the outline pane beside the document while reading.
    outline_pane: bool,
    /// Where the outline pane's rows were drawn (empty when it isn't shown).
    outline_area: Rect,
    outline_list: ListState,
    /// The outline pane has the keyboard: the heading selected in it, and
    /// where the document was when it took the keyboard.
    outline_focus: Option<(usize, usize)>,
    /// Text typed after `/` in the outline, narrowing its headings.
    outline_filter: Option<String>,
    /// `o` opened the pane just while it has the keyboard.
    outline_temporary: bool,
    help: bool,
    /// How many lines `j` and `k` scroll.
    scroll: usize,
    /// Show the source beside the rendered document.
    split: bool,
    /// Number the rendered document's lines with their source lines.
    numbers: bool,
    /// In the split view, the source side has the keyboard.
    source_focus: bool,
    /// The source side's share of the split, in percent.
    ratio: u16,
    source_right: bool,
    /// A prompt or popup that has the keyboard, in the reader.
    prompt: Option<nav::Prompt>,
    /// Where following links has come from, to go back to.
    history: Vec<nav::Place>,
    /// The last search of a document, for `C-s` on an empty prompt.
    last_search: String,
    /// A message for the footer, until the next key.
    flash: Option<String>,
    /// A file to open in the editor, at a line, once the key's handled.
    edit: Option<(PathBuf, usize)>,
    /// Ctrl-Z was pressed: stop, once the key's handled.
    suspend: bool,
    /// The piece of the document a drag started on.
    drag_from: Option<usize>,
    /// Where big headings were drawn last frame.
    big_drawn: Vec<Rect>,
    /// Draw the screen again from scratch.
    redraw: bool,
    /// The mouse is in use.
    mouse_on: bool,
    /// A search of every file under way, and its query.
    grep: Option<(Receiver<Vec<crate::grep::Hit>>, String)>,
    /// Filtering, the search of the files' text.
    text_search: found::TextSearch,
    /// How the terminal draws pictures, if it can.
    picker: Option<Picker>,
    /// Pictures made ready to draw, by diagram, columns and rows.
    drawn: HashMap<(u64, usize, usize), crate::figure::Drawn>,
    /// Pictures to send the terminal, or free, after this frame.
    to_send: Vec<String>,
    /// The document on screen and how far it's scrolled, and when that
    /// last changed: an older iTerm2's pictures are drawn softly while it's
    /// moving.
    figure_at: Option<(Option<PathBuf>, usize)>,
    figure_moved: Option<Instant>,
    figure_moving: bool,
    docs: HashMap<PathBuf, Doc>,
}

impl App {
    fn new(source: Source, theme: Theme, max_width: Option<usize>) -> App {
        let mut app = App {
            theme: Rc::new(theme),
            choice: Choice::Mode(Mode::Auto),
            theme_before: None,
            terminal_dark: true,
            omarchy: false,
            follow: None,
            max_width,
            text: None,
            root_label: String::new(),
            files: Vec::new(),
            scan: None,
            root: None,
            site_root: None,
            all: false,
            hidden: false,
            live_added: HashSet::new(),
            watch: None,
            index: None,
            indexing: None,
            by_time: false,
            filter: String::new(),
            typing: false,
            matcher: Matcher::new(Config::DEFAULT.match_paths()),
            shown: Vec::new(),
            listed: Vec::new(),
            in_scope: 0,
            count_w: 0,
            open: HashSet::new(),
            scope: String::new(),
            list: ListState::default(),
            list_height: 0,
            list_area: Rect::default(),
            moved: false,
            want: None,
            focus: Focus::List,
            reading: None,
            list_in_reader: false,
            left_on: None,
            body_width: 0,
            outline_pane: false,
            outline_area: Rect::default(),
            outline_list: ListState::default(),
            outline_focus: None,
            outline_filter: None,
            outline_temporary: false,
            help: false,
            scroll: 2,
            split: false,
            numbers: false,
            source_focus: false,
            ratio: 50,
            source_right: true,
            prompt: None,
            history: Vec::new(),
            last_search: String::new(),
            flash: None,
            edit: None,
            suspend: false,
            drag_from: None,
            big_drawn: Vec::new(),
            redraw: false,
            grep: None,
            mouse_on: false,
            text_search: Default::default(),
            picker: None,
            drawn: HashMap::new(),
            to_send: Vec::new(),
            figure_at: None,
            figure_moved: None,
            figure_moving: false,
            docs: HashMap::new(),
        };
        match source {
            Source::Text { title, md } => {
                let mut doc = Doc::new(md);
                app.site_root = doc.base.as_deref().map(files::site_root);
                doc.site = app.site_root.clone();
                app.text = Some((title, doc));
                app.focus = Focus::Reader;
                app.watch = Watch::new();
            }
            Source::Browse { root, open, all } => {
                app.root_label = display_path(&root);
                app.scan = Some(files::scan(&root, all, false));
                app.watch = Watch::new();
                if let Some(watch) = &mut app.watch {
                    watch.add(&root);
                }
                app.site_root = Some(files::site_root(&root));
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
            self.preview_theme();
            self.frame(terminal)?;
            // While scanning or indexing, wake up now and then to show
            // what's new.
            let busy = self.scan.is_some() || self.indexing.is_some() || self.grep.is_some();
            let busy = busy || crate::picture::making() || self.figure_moving;
            let wait = if busy || self.text_search.busy() {
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
                let result = self.hand_over(terminal, || editor::edit(&path, line))?;
                if let Err(e) = result {
                    self.flash = Some(format!("Couldn't edit: {e}"));
                }
            }
            if std::mem::take(&mut self.suspend) {
                self.hand_over(terminal, stop)?;
            }
        }
    }

    /// Draws the screen. When big headings have moved, draws it again from
    /// scratch, since the terminal clears the whole of one when any of it
    /// is written over; all at once, so the first draw never shows.
    fn frame(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        use ratatui::crossterm::execute;
        use ratatui::crossterm::terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate};
        let sync = crate::sizing::enabled();
        if sync {
            execute!(io::stdout(), BeginSynchronizedUpdate)?;
        }
        terminal.draw(|f| self.draw(f))?;
        if std::mem::take(&mut self.redraw) {
            // Every cell again, but without clearing the screen: that would
            // take Kitty's pictures with it, which are sent only once. What
            // ratatui takes to be on screen is made something that never
            // is, so nothing is left out as unchanged.
            terminal.swap_buffers();
            for cell in &mut terminal.current_buffer_mut().content {
                cell.set_symbol(NOT_ON_SCREEN);
            }
            terminal.swap_buffers();
            terminal.draw(|f| self.draw(f))?;
        }
        // Kitty's pictures, sent once whatever is drawn over them.
        let mut out = io::stdout();
        for send in self.to_send.drain(..) {
            io::Write::write_all(&mut out, send.as_bytes())?;
        }
        io::Write::flush(&mut out)?;
        if sync {
            execute!(io::stdout(), EndSynchronizedUpdate)?;
        }
        Ok(())
    }

    /// Gives the terminal back as it was while `f` runs, then takes it
    /// again. Its pictures are gone with the screen: they're sent again.
    fn hand_over<T>(
        &mut self,
        terminal: &mut DefaultTerminal,
        f: impl FnOnce() -> T,
    ) -> io::Result<T> {
        set_mouse(self.mouse_on, false);
        ratatui::restore();
        let result = f();
        *terminal = ratatui::init();
        set_mouse(self.mouse_on, true);
        terminal.clear()?;
        self.forget_drawn();
        Ok(result)
    }

    /// Takes in files the scan has found since last time, and the link
    /// index when it's ready.
    fn receive(&mut self) {
        self.reload();
        self.restyle();
        self.receive_grep();
        self.receive_found();
        if crate::picture::take_news() {
            let text = self.text.iter_mut().map(|(_, doc)| doc);
            for doc in self.docs.values_mut().chain(text) {
                doc.relayout();
            }
        }
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
                    // Skip files live reload already added mid-scan.
                    let live = &self.live_added;
                    self.files
                        .extend(batch.into_iter().filter(|e| !live.contains(&e.path)));
                    changed = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.scan = None;
                    self.live_added.clear();
                    changed = true;
                    // Search the text of the files the scan found since.
                    if !self.filter.is_empty() {
                        self.filter_changed();
                    }
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
        if self.scan.is_none() {
            // The scan's done: a file asked for that it didn't list (hidden,
            // say) mustn't keep the selection from following the user.
            self.want = None;
        }
    }

    /// The selected file (`None` on a folder or `..`).
    fn selected(&self) -> Option<&Entry> {
        match self.shown.get(self.list.selected()?)? {
            Shown::File { file, .. } => Some(&self.files[*file]),
            Shown::Dir(_) | Shown::Up(_) | Shown::Rule(_) => None,
        }
    }

    /// Re-sorts and re-filters the list, keeping the same row selected.
    ///
    /// Until the user moves, the selection stays on the best match (or the
    /// file named on the command line), however the list changes.
    fn refresh(&mut self) {
        let chosen = self.moved.then(|| self.picked()).flatten();
        let keep = self.want.clone().map(tree::Pick::File).or(chosen);
        let scope = self.scope().to_string();
        let order = self.sorted_in(&scope);
        self.in_scope = order.len();
        self.count_w = if self.files.iter().any(|e| e.rel.contains('/')) {
            self.files.len().to_string().len()
        } else {
            0
        };

        // In the list's order, with each match's score.
        let mut matched: Vec<(usize, Vec<u32>, u32)> = Vec::new();
        if self.filter.is_empty() {
            matched = order.iter().map(|&file| (file, Vec::new(), 0)).collect();
        } else {
            let pattern = Pattern::parse(&self.filter, CaseMatching::Smart, Normalization::Smart);
            let mut buf = Vec::new();
            for &file in &order {
                let mut hits = Vec::new();
                let hay = Utf32Str::new(&self.files[file].rel[scope.len()..], &mut buf);
                if let Some(score) = pattern.indices(hay, &mut self.matcher, &mut hits) {
                    hits.sort_unstable();
                    hits.dedup();
                    matched.push((file, hits, score));
                }
            }
        }
        self.listed = matched.iter().map(|m| m.0).collect();

        // `..`, inside a folder; then the folders, and the files directly
        // in this one; then the files in the folders, with their paths.
        // Filtering, the folders that match, then the files, best first.
        let mut shown = Vec::new();
        if !scope.is_empty() {
            shown.push(Shown::Up(self.parent_scope(&scope)));
        }
        let mut dir_best = None;
        if self.filter.is_empty() {
            shown.extend(tree::listing(
                &self.files,
                &self.listed,
                &scope,
                &self.open,
                self.by_time,
            ));
            matched.clear();
        } else {
            let pattern = Pattern::parse(&self.filter, CaseMatching::Smart, Normalization::Smart);
            let dirs = tree::matching_dirs(
                &self.files,
                &order,
                &scope,
                self.by_time,
                &pattern,
                &mut self.matcher,
            );
            dir_best = dirs.first().map(|(score, _)| (*score, shown.len()));
            let any_dirs = !dirs.is_empty();
            shown.extend(dirs.into_iter().map(|(_, d)| Shown::Dir(d)));
            if any_dirs && !matched.is_empty() {
                shown.push(Shown::Rule(""));
            }
            matched.sort_by_key(|m| std::cmp::Reverse(m.2));
        }
        let file_best = matched.first().map(|m| (m.2, shown.len()));
        shown.extend(matched.into_iter().map(|(file, hits, _)| Shown::File {
            file,
            hits,
            from: scope.len(),
            depth: 0,
            found: 0,
        }));
        // Past the rule, the first file found by its text.
        let text_best = (!self.filter.is_empty()).then(|| shown.len() + 1);
        let found = self.found_rows(&order, &self.listed);
        let text_best = text_best.filter(|_| !found.is_empty());
        if !self.filter.is_empty() {
            shown.extend(found);
        }
        self.shown = shown;

        let index = keep.and_then(|k| self.row_of(&k));
        if index.is_some() && self.want.take().is_some() {
            // Found the file we were waiting for: from now on it counts as
            // chosen, and later batches mustn't move off it.
            self.moved = true;
        }
        // Filtering, the best match: a folder only if it beats every file.
        let best = match (dir_best, file_best) {
            (Some((d, at)), Some((f, _))) if d > f => Some(at),
            (Some((_, at)), None) => Some(at),
            (_, Some((_, at))) if !self.filter.is_empty() => Some(at),
            (None, None) => text_best,
            _ => None,
        };
        // Otherwise the README here, where there is one, or the first row
        // past `..`.
        let readme = self.shown.iter().position(|s| match s {
            Shown::File {
                file, from, depth, ..
            } => {
                // Not one in a folder opened in place.
                let shown = &self.files[*file].rel[*from..];
                *depth == 0 && !shown.contains('/') && shown.to_lowercase().starts_with("readme.")
            }
            _ => false,
        });
        let top = readme
            .or_else(|| self.shown.iter().position(|s| !matches!(s, Shown::Up(_))))
            .or((!self.shown.is_empty()).then_some(0));
        self.list.select(index.or(best).or(top));
        // Scrolled down a list that's since got shorter, or changed under a
        // selection the user didn't choose: show its top, scrolling only as
        // far as the selection needs.
        let last_top = self.shown.len().saturating_sub(self.list_height);
        let offset = if self.moved { self.list.offset() } else { 0 };
        *self.list.offset_mut() = offset.min(last_top);
    }

    /// The files in `scope`, in the list's order.
    fn sorted_in(&self, scope: &str) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.files.len())
            .filter(|&i| self.files[i].rel.starts_with(scope))
            .collect();
        let cmp = if self.by_time {
            files::by_modified
        } else {
            files::by_path
        };
        order.sort_by(|&a, &b| cmp(&self.files[a], &self.files[b]));
        order
    }

    /// The document for `path`, loading it the first time. (Changes to
    /// it are picked up by [`App::reload`].)
    fn doc(&mut self, path: &Path) -> &mut Doc {
        if !self.docs.contains_key(path) {
            // Files outside the root aren't watched yet.
            if let Some(watch) = &mut self.watch {
                watch.add(path);
            }
            let mut doc = Doc::load(path);
            doc.site = self.site_root.clone();
            self.docs.insert(path.to_path_buf(), doc);
        }
        self.docs.get_mut(path).unwrap()
    }

    /// Switches to the Omarchy theme's new colors, if it has changed.
    fn restyle(&mut self) {
        let Some(palette) = self.follow.as_ref().and_then(Follow::changed) else {
            return;
        };
        // Following is only on with color.
        self.set_theme(Rc::new(Theme::new(Mode::Auto, true, Some(&palette))));
    }

    fn set_theme(&mut self, theme: Rc<Theme>) {
        self.theme = theme;
        let text = self.text.iter_mut().map(|(_, doc)| doc);
        for doc in self.docs.values_mut().chain(text) {
            doc.restyle();
        }
    }

    /// Catches up with files that have changed: open documents re-render
    /// in place, and the file list and link index follow.
    fn reload(&mut self) {
        let Some(watch) = &self.watch else { return };
        let changed = watch.changed();
        let mut list_changed = false;
        let mut index_changed = false;
        for path in changed.into_iter().filter(|p| files::is_markdown(p)) {
            let exists = path.is_file();
            if let Some(doc) = self.docs.get_mut(&path)
                && exists
            {
                doc.reload(&path);
            }
            let (Some(root), Some(rel)) = (self.root.clone(), self.rel_of(&path)) else {
                continue;
            };
            let listed = self.files.iter().position(|e| e.path == path);
            // A new file only joins if the scan would have listed it: not in
            // node_modules or anything else .gitignore'd, nor hidden.
            if listed.is_none() && !(exists && files::listable(&root, &path, self.all, self.hidden))
            {
                continue;
            }
            if let Some(index) = &mut self.index {
                index_changed |= index.refresh(&rel, &path);
            }
            let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
            match listed {
                Some(i) if exists => self.files[i].modified = modified,
                Some(i) => {
                    self.files.remove(i);
                }
                None => {
                    // The scan may yet send it too: see `receive`.
                    if self.scan.is_some() {
                        self.live_added.insert(path.clone());
                    }
                    self.files.push(Entry {
                        path,
                        rel,
                        modified,
                    });
                }
            }
            list_changed = true;
        }
        if index_changed && let Some(index) = &mut self.index {
            index.commit();
        }
        if list_changed {
            self.refresh();
        }
    }

    /// The document on screen: the one being read, or else the selected one.
    fn current(&mut self) -> Option<&mut Doc> {
        match self.current_path() {
            Some(path) => Some(self.doc(&path)),
            None => self.text.as_mut().map(|(_, doc)| doc),
        }
    }

    fn select_by(&mut self, delta: isize) {
        if self.shown.is_empty() {
            return;
        }
        self.moved = true;
        let mut i = self
            .list
            .selected()
            .unwrap_or(0)
            .saturating_add_signed(delta)
            .min(self.shown.len() - 1);
        // Past a rule, the way we're going, or back if it's at the end.
        if matches!(self.shown[i], Shown::Rule(_)) {
            let on = if delta < 0 {
                i.checked_sub(1)
            } else {
                Some(i + 1)
            };
            let back = if delta < 0 { i + 1 } else { i.wrapping_sub(1) };
            match on.or(Some(back)).filter(|&j| j < self.shown.len()) {
                Some(j) => i = j,
                None => return,
            }
        }
        self.list.select(Some(i));
    }

    fn start_edit(&mut self) {
        if self.selected_folder().is_some() {
            self.flash = Some("Choose a file to edit".into());
            return;
        }
        let Some(path) = self.current_path() else {
            self.flash = Some("Standard input isn't a file to edit".into());
            return;
        };
        let line = self.current().map_or(1, |d| d.source_line());
        self.edit = Some((path, line));
    }

    /// `.`: shows or hides hidden files, scanning again, and keeping the
    /// selection where it can.
    fn toggle_hidden(&mut self) {
        let Some(root) = self.root.clone() else {
            return;
        };
        if self.all {
            self.flash = Some("Hidden files are listed already (--all)".into());
            return;
        }
        self.hidden = !self.hidden;
        self.want = self.selected().map(|e| e.path.clone());
        if !self.hidden && self.scope.split('/').any(|c| c.starts_with('.')) {
            self.scope.clear();
        }
        self.files.clear();
        self.live_added.clear();
        self.scan = Some(files::scan(&root, false, self.hidden));
        self.filter_changed();
        self.moved = false;
        self.refresh();
        self.flash = Some(
            if self.hidden {
                "Showing hidden files"
            } else {
                "Hiding hidden files"
            }
            .into(),
        );
    }

    /// The side of the document that scrolls.
    fn side(&self) -> Side {
        if self.split && self.source_focus {
            Side::Source
        } else {
            Side::Rendered
        }
    }

    /// `Tab`: from the list, reads the selected file, with the list kept
    /// beside it if there's room; from the reader, back to the list, which
    /// shows what was being read until another file's selected.
    fn switch_view(&mut self) {
        if self.text.is_some() {
            return;
        }
        match self.focus {
            // Back to what was being read, links followed and all.
            Focus::List if self.kept() => self.focus = Focus::Reader,
            Focus::List => {
                let Some(path) = self.selected().map(|e| e.path.clone()) else {
                    self.flash = Some("Choose a file to read".into());
                    return;
                };
                self.reading = Some(path);
                self.focus = Focus::Reader;
                self.history.clear();
                self.list_in_reader = self.body_width >= WIDE;
            }
            Focus::Reader => {
                self.focus = Focus::List;
                self.left_on = self.selected().map(|e| e.path.clone());
            }
        }
    }

    /// In the list, still showing what `Tab` left the reader on: nothing
    /// else has been selected since.
    pub(super) fn kept(&self) -> bool {
        self.focus == Focus::List
            && self.left_on.is_some()
            && self.left_on.as_ref() == self.selected().map(|e| &e.path)
    }

    /// Keys that work the same in the list and the reader.
    fn view_key(&mut self, key: KeyEvent, ctrl: bool) -> bool {
        match key.code {
            KeyCode::Char('e') if !ctrl => self.start_edit(),
            KeyCode::Char('c') if !ctrl => self.open_copy(None),
            KeyCode::Char('O') if self.focus == Focus::Reader => self.focus_outline(false),
            KeyCode::Char('O') => self.outline_pane = !self.outline_pane,
            KeyCode::Char('T') => self.open_themes(),
            KeyCode::Char('s') if !ctrl => {
                self.prompt = Some(nav::Prompt::Grep {
                    query: String::new(),
                });
            }
            KeyCode::Char('S') => self.split = !self.split,
            KeyCode::Tab | KeyCode::BackTab => self.switch_view(),
            KeyCode::Char('#') => self.numbers = !self.numbers,
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
        let key = emacs(key);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            return true;
        }
        if ctrl && key.code == KeyCode::Char('z') && cfg!(unix) {
            self.suspend = true;
            return false;
        }
        self.flash = None;
        if self.help {
            self.help = false;
            return false;
        }
        if let Some(prompt) = self.prompt.take() {
            return self.prompt_key(prompt, key, ctrl);
        }
        if self.typing {
            self.filter_key(key, ctrl);
            return false;
        }
        if self.outline_focus.is_some() {
            if self.focus == Focus::Reader && self.outline_pane {
                return self.outline_key(key);
            }
            self.leave_outline();
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
            KeyCode::Enter => return self.typing = false,
            KeyCode::Esc => {
                self.typing = false;
                self.filter.clear();
                self.filter_changed();
                // The match stays selected.
                self.moved = true;
                return self.refresh();
            }
            KeyCode::Backspace => {
                if self.filter.pop().is_none() {
                    self.typing = false;
                }
            }
            KeyCode::Char(c) if !ctrl => self.filter.push(c),
            _ => return,
        }
        self.filter_changed();
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
            KeyCode::Char('J') => self.select_by(page),
            KeyCode::Char('K') => self.select_by(-page),
            KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => self.select_by(page),
            KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => self.select_by(-page),
            KeyCode::Char('j') | KeyCode::Down => self.select_by(1),
            KeyCode::Char('k') | KeyCode::Up => self.select_by(-1),
            KeyCode::PageDown => self.select_by(page),
            KeyCode::PageUp => self.select_by(-page),
            KeyCode::Char('g') | KeyCode::Home => self.select_by(isize::MIN / 2),
            KeyCode::Char('G') | KeyCode::End => self.select_by(isize::MAX / 2),
            // → goes down into a folder, ← back up: the same in the tree
            // and the flat list.
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right if self.on_up() => {
                self.leave_dir();
            }
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right
                if self.selected_dir().is_some() =>
            {
                self.enter_dir()
            }
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
                if let Some(e) = self.selected() {
                    self.reading = Some(e.path.clone());
                    self.focus = Focus::Reader;
                }
            }
            KeyCode::Char('h') | KeyCode::Left | KeyCode::Backspace => {
                self.leave_dir();
            }
            KeyCode::Char(' ') if self.selected_dir().is_some() => self.toggle_dir(),
            KeyCode::Char('/') => self.typing = true,
            KeyCode::Char('.') => self.toggle_hidden(),
            KeyCode::Esc if !self.filter.is_empty() => {
                // The match stays selected.
                self.moved = true;
                self.filter.clear();
                self.filter_changed();
                self.refresh();
            }
            KeyCode::Esc if self.leave_dir() => {}
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
        // Of the Ctrl keys, only Emacs's searches are for getting around.
        let search = matches!(key.code, KeyCode::Char('s' | 'r'));
        if (!ctrl || search) && self.nav_key(key) {
            return false;
        }
        // Going back with somewhere to go back to was handled above.
        match key.code {
            KeyCode::Char('q') => return true,
            KeyCode::Esc if !browsing => return true,
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') if browsing => {
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
        let lines = self.scroll as isize;
        let Some(doc) = self.current() else {
            return false;
        };
        let page = doc.page();
        let half = (page / 2).max(1);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        match key.code {
            // Shift turns a line into a page.
            KeyCode::Char('J') => doc.scroll_by(page, side),
            KeyCode::Char('K') => doc.scroll_by(-page, side),
            KeyCode::Down if shift => doc.scroll_by(page, side),
            KeyCode::Up if shift => doc.scroll_by(-page, side),
            KeyCode::Char('d') if ctrl => doc.scroll_by(half, side),
            KeyCode::Char('u') if ctrl => doc.scroll_by(-half, side),
            KeyCode::Char('f') if ctrl => doc.scroll_by(page, side),
            KeyCode::Char('b') if ctrl => doc.scroll_by(-page, side),
            KeyCode::Char('j') | KeyCode::Down | KeyCode::Enter => doc.scroll_by(lines, side),
            KeyCode::Char('k') | KeyCode::Up => doc.scroll_by(-lines, side),
            KeyCode::Char('d') => doc.scroll_by(half, side),
            KeyCode::Char('u') => doc.scroll_by(-half, side),
            KeyCode::Char(' ') | KeyCode::PageDown => doc.scroll_by(page, side),
            KeyCode::Char('b') | KeyCode::PageUp => doc.scroll_by(-page, side),
            KeyCode::Char('g') | KeyCode::Home => doc.scroll_to_top(side),
            KeyCode::Char('G') | KeyCode::End => doc.scroll_to_bottom(side),
            _ => {}
        }
        false
    }

    fn draw(&mut self, f: &mut Frame) {
        self.preview_found();
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
        self.body_width = body.width;
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
            // Every path, as the rows below the folders show them, so the
            // list keeps its width as you go in and out of folders.
            let paths = self
                .files
                .iter()
                .map(|e| 1 + tree::indent(0) + wrap::width(&e.rel));
            let rows = self.shown.iter().map(|s| tree::row_width(&self.files, s));
            let widest = rows.chain(paths).max();
            // The time column, gaps and borders, and the folders' counts.
            let want = widest.unwrap_or(0) + 10 + tree::count_column(self.count_width());
            // Room for the folder being listed in the title: " docs/api/ (12) ".
            let title = wrap::width(self.scope()) + self.in_scope.to_string().len() + 7;
            let want = want.max(title);
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
        self.theme.paint(f.buffer_mut());
        // Last, so it's drawn with the colors on screen, over what's
        // still there to see.
        let big = crate::sizing::apply(f.buffer_mut());
        if big != self.big_drawn {
            self.big_drawn = big;
            self.redraw = true;
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
        let mut title = vec![" lsmd ".bold(), Span::raw(" "), summary.clone().dim()];
        // Reading: which section the top of the screen is in.
        let section: Vec<String> = match self.focus {
            Focus::Reader => self
                .current()
                .map(|d| d.section().iter().map(|s| s.to_string()).collect())
                .unwrap_or_default(),
            Focus::List => Vec::new(),
        };
        // " lsmd " and the "  § " before the trail, and a gap before the note.
        let gap = if note_w > 0 { 2 } else { 0 };
        let room = usize::from(title_area.width).saturating_sub(wrap::width(&summary) + 11 + gap);
        if let Some(trail) = section_trail(&section, room) {
            title.push(Span::raw("  § ").dim());
            title.push(Span::raw(trail));
        }
        f.render_widget(Paragraph::new(Line::from(title)), title_area);
    }

    fn pane(&self, title: impl Into<Line<'static>>, focused: bool) -> Block<'static> {
        // In the theme's accent, if it has one; dim without the keyboard.
        let border = self
            .theme
            .frame
            .map_or(Style::new(), |c| Style::new().fg(c));
        let border = if focused { border } else { border.dim() };
        Block::bordered().title(title).border_style(border)
    }

    fn draw_list(&mut self, f: &mut Frame, area: Rect) {
        let name = match self.scope() {
            "" => "Files",
            scope => scope,
        };
        let count = if self.filter.is_empty() {
            format!(" {name} ({}) ", self.in_scope)
        } else {
            let in_text = self
                .shown
                .iter()
                .filter(|s| matches!(s, Shown::File { found: 1.., .. }))
                .count();
            let mut n = self.listed.len().to_string();
            if in_text > 0 {
                n.push_str(&format!(" + {in_text}"));
            }
            if self.text_search.busy() {
                n.push_str(" …");
            }
            format!(" {name} ({n} of {}) ", self.in_scope)
        };
        // The filter shows on the list it narrows, not down in the footer.
        let mut title = vec![Span::raw(count)];
        if self.typing || !self.filter.is_empty() {
            title.extend(["/".bold(), Span::raw(self.filter.clone()).yellow()]);
            title.push(if self.typing {
                "▏".slow_blink()
            } else {
                Span::raw("")
            });
            title.push(Span::raw(" "));
        }
        let block = self.pane(Line::from(title), self.focus == Focus::List);
        let inner = block.inner(area);
        self.list_height = inner.height.into();
        self.list_area = inner;
        let now = SystemTime::now();
        let width = usize::from(inner.width);
        let items: Vec<ListItem> = self
            .shown
            .iter()
            .map(|s| ListItem::new(self.row_line(s, width, now)))
            .collect();
        let list = List::new(items)
            .block(block)
            .highlight_style(Style::new().add_modifier(Modifier::REVERSED));
        f.render_stateful_widget(list, area, &mut self.list);

        if self.shown.is_empty() && self.scan.is_none() {
            let msg = if self.files.is_empty() {
                "No Markdown files here"
            } else if self.text_search.busy() {
                "Searching the text…"
            } else {
                "Nothing matches"
            };
            f.render_widget(Paragraph::new(format!(" {msg}")).dim(), inner);
        }
    }

    /// How wide the folders' counts are: as wide as the most there could
    /// be, so the columns don't move from folder to folder.
    fn count_width(&self) -> usize {
        self.count_w
    }

    /// A row of the file list, `width` columns wide.
    fn row_line(&self, row: &Shown, width: usize, now: SystemTime) -> Line<'static> {
        let count_w = self.count_width();
        match row {
            Shown::File {
                file,
                hits,
                from,
                depth,
                found,
            } => {
                let e = &self.files[*file];
                // Files leave the folders' count column empty.
                let width = width.saturating_sub(tree::count_column(count_w));
                // Found by its text: how many lines have it, not its age.
                let age = match found {
                    0 => files::ago(e.modified, now),
                    1 => "1 line".into(),
                    n => format!("{n} lines"),
                };
                list_line(tree::indent(*depth), &e.rel[*from..], hits, &age, width)
            }
            Shown::Dir(d) => tree::dir_line(d, &files::ago(d.newest, now), width, count_w),
            Shown::Up(to) => tree::up_line(to),
            Shown::Rule(label) => rule_line(label, width),
        }
    }

    /// The folder selected in the list (or the one `..` goes up to).
    fn selected_folder(&self) -> Option<String> {
        if self.focus != Focus::List {
            return None;
        }
        match self.shown.get(self.list.selected()?)? {
            Shown::Dir(d) => Some(d.rel.clone()),
            Shown::Up(to) => Some(to.clone()),
            Shown::File { .. } | Shown::Rule(_) => None,
        }
    }

    /// What's in `scope`, as going into it lists it, `height` rows of it.
    fn folder_preview(&self, scope: &str, width: usize, height: usize) -> Vec<Line<'static>> {
        let order = self.sorted_in(scope);
        let now = SystemTime::now();
        let newest = order.iter().filter_map(|&i| self.files[i].modified).max();
        let n = order.len();
        let mut summary = format!("{n} Markdown file{}", if n == 1 { "" } else { "s" });
        let age = files::ago(newest, now);
        if !age.is_empty() {
            summary.push_str(&format!(", the newest changed {age}"));
        }
        let mut lines = vec![Line::from(summary).dim(), Line::default()];
        let rows = tree::listing(&self.files, &order, scope, &HashSet::new(), self.by_time);
        let room = height.saturating_sub(lines.len());
        // Room for a last line saying how many more there are.
        let fits = if rows.len() > room {
            room.saturating_sub(1)
        } else {
            rows.len()
        };
        lines.extend(rows[..fits].iter().map(|r| self.row_line(r, width, now)));
        let more = rows[fits..]
            .iter()
            .filter(|r| !matches!(r, Shown::Rule(_)))
            .count();
        if more > 0 {
            lines.push(Line::from(format!(" … and {more} more")).dim());
        }
        lines
    }

    fn draw_preview(&mut self, f: &mut Frame, area: Rect) {
        let folder = self.selected_folder();
        let path = self.current_path();
        let title = match (&folder, path.as_deref()) {
            (Some(dir), _) if dir.is_empty() => format!(" {} ", self.root_label),
            (Some(dir), _) => format!(" {dir} "),
            (None, Some(p)) => format!(" {} ", self.rel_label(p)),
            (None, None) => String::new(),
        };
        let block = self
            .pane(title, self.focus == Focus::Reader)
            .padding(Padding::horizontal(1));
        let inner = block.inner(area);
        f.render_widget(block, area);
        if let Some(dir) = folder {
            let lines =
                self.folder_preview(&dir, usize::from(inner.width), usize::from(inner.height));
            f.render_widget(Paragraph::new(lines), inner);
            return;
        }
        self.draw_doc(f, inner);
    }

    fn draw_doc(&mut self, f: &mut Frame, area: Rect) {
        self.outline_area = Rect::default();
        let mut area = area;
        let mut pane = None;
        if self.outline_pane && self.focus == Focus::Reader {
            let w = Self::outline_width(area.width);
            pane = Some(Rect {
                x: area.right() - w,
                width: w,
                ..area
            });
            // Leave a column for the scrollbar, and one more for air.
            area.width = area.width.saturating_sub(w + 2);
        }
        let mut width = usize::from(area.width);
        if let Some(max) = self.max_width {
            width = width.min(max);
        }
        let theme = Rc::clone(&self.theme);
        let split = self.split.then(|| Split {
            ratio: self.ratio,
            focus: self.side(),
            source_right: self.source_right,
            max_width: self.max_width,
            numbers: self.numbers,
        });
        let numbers = self.numbers;
        if let Some(doc) = self.current() {
            if let Some(split) = split {
                doc.draw_split(f, area, &split, &theme);
            } else {
                doc.draw(f, area, width, &theme, numbers);
            }
        }
        self.draw_figures(f);
        // After the document, so it shows the section it's scrolled to.
        if let Some(pane) = pane {
            self.draw_outline_pane(f, pane);
        }
    }

    /// Lets go of every picture made ready, freeing Kitty's in the terminal.
    fn forget_drawn(&mut self) {
        let forgotten = self.drawn.drain().filter_map(|(_, drawn)| drawn.forget());
        self.to_send.extend(forgotten);
    }

    /// The document's pictures, over the room it left for them.
    fn draw_figures(&mut self, f: &mut Frame) {
        if self.picker.is_none() {
            return;
        }
        let path = self.current_path();
        let Some((top, (area, figures))) = self.current().map(|d| (d.top(), d.figures())) else {
            self.figure_moving = false;
            return;
        };
        if figures.is_empty() {
            self.figure_moving = false;
            return;
        }
        // An iTerm2 without Kitty's pictures has its own sent again whenever
        // they move, a lot for it to keep up with at a key's repeat rate:
        // moving again soon after the last move, they're shown with less
        // detail till things settle. Other terminals keep up (Kitty's are
        // sent once, and only placed after), so theirs move as they are.
        let now = Instant::now();
        let settled = self.figure_moved.is_none_or(|t| now - t >= FIGURE_SETTLE);
        let at = Some((path, top));
        if self.figure_at != at {
            self.figure_moving = !settled;
            self.figure_at = at;
            self.figure_moved = Some(now);
        } else if settled {
            self.figure_moving = false;
        }
        if self.drawn.len() > MAX_DRAWN {
            self.forget_drawn();
        }
        let Some(picker) = &self.picker else { return };
        for (figure, y) in figures {
            let key = (figure.picture.key, figure.cols, figure.rows);
            if !self.drawn.contains_key(&key)
                && let Some(mut drawn) = crate::figure::Drawn::new(
                    picker,
                    &figure.picture.image,
                    figure.cols,
                    figure.rows,
                )
            {
                self.to_send.extend(drawn.take_send());
                self.drawn.insert(key, drawn);
            }
            let Some(drawn) = self.drawn.get(&key) else {
                continue;
            };
            drawn.draw(f.buffer_mut(), area, figure.x as u16, y, self.figure_moving);
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
        if self.outline_focus.is_some() {
            let mut spans = vec![Span::raw(" ")];
            // A filter being typed shows in the outline pane itself.
            let keys: &[(&str, &str)] = if self.outline_filter.is_some() {
                &[("↑↓", "move"), ("⏎", "read here"), ("esc", "clear")]
            } else {
                &[
                    ("↑↓", "move"),
                    ("/", "filter"),
                    ("⏎", "read here"),
                    ("esc", "back"),
                    ("O", "close"),
                    ("q", "quit"),
                ]
            };
            for (k, what) in keys {
                spans.push(k.bold());
                spans.push(Span::raw(format!(" {what}  ")).dim());
            }
            f.render_widget(Paragraph::new(Line::from(spans)), area);
            return;
        }
        if self.typing {
            // The filter itself shows in the list's border.
            let keys = [("↑↓", "move"), ("⏎", "done"), ("esc", "clear")];
            let mut spans = vec![Span::raw(" ")];
            for (k, what) in keys {
                spans.push(k.bold());
                spans.push(Span::raw(format!(" {what}  ")).dim());
            }
            f.render_widget(Paragraph::new(Line::from(spans)), area);
            return;
        }
        // Esc clears a search before it goes back.
        let searching = self.current().is_some_and(|d| d.search_status().is_some());
        let back = if searching {
            "clear search".into()
        } else {
            self.back_label()
        };
        let source = ("S", if self.split { "hide source" } else { "source" });
        let mut reader = vec![
            ("↑↓", "scroll"),
            ("/", "search"),
            ("f", "follow"),
            ("o", "outline"),
            source,
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
                let on_dir = self.selected_dir().is_some();
                let on_file = self.selected().is_some();
                let mut keys = vec![("↑↓", "move")];
                if on_dir {
                    keys.push(("→", "go in"));
                    if self.filter.is_empty() {
                        keys.push(("space", "look inside"));
                    }
                } else if on_file {
                    keys.push(("⏎", "read"));
                }
                if !self.scope.is_empty() {
                    keys.push(("←", "up"));
                }
                keys.extend([("/", "filter"), ("s", "search text")]);
                if self.kept() {
                    keys.push(("tab", "back to reading"));
                }
                if on_file {
                    keys.push(source);
                }
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
        // And a gap before it, for when the keys don't all fit.
        let pos_width = wrap::width(&position) as u16 + 3;
        let [keys_area, pos_area] =
            Layout::horizontal([Constraint::Min(0), Constraint::Length(pos_width)]).areas(area);
        f.render_widget(Paragraph::new(Line::from(spans)), keys_area);
        f.render_widget(
            Paragraph::new(Line::from(format!("{position} ")).dim()).right_aligned(),
            pos_area,
        );
    }
}

/// A long path shows at least this many characters of its file name beside
/// its top folder, or else gives up the folder.
const MIN_NAME: usize = 8;

/// Part of a path as shown: characters `start..end` of it, or a `…` where
/// some are left out.
#[derive(Debug, PartialEq)]
enum Piece {
    Keep(usize, usize),
    Gap,
}

/// What of a path fits in `room` columns: all of it; or its top folder
/// (which the folder rows above name) and file name, leaving out folders
/// in between; or the top folder and the start of the file name; or just
/// the start of the file name.
fn fit_path(chars: &[char], room: usize) -> Vec<Piece> {
    use Piece::{Gap, Keep};
    let w = |from: usize, to: usize| {
        chars[from..to]
            .iter()
            .map(|&c| char_width(c))
            .sum::<usize>()
    };
    let len = chars.len();
    if w(0, len) <= room {
        return vec![Keep(0, len)];
    }
    // The characters from `from` that fit in `budget` columns: all of them,
    // or as many as fit beside a `…` after them.
    let clip = |from: usize, budget: usize| -> Vec<Piece> {
        if w(from, len) <= budget {
            return vec![Keep(from, len)];
        }
        let (mut end, mut used) = (from, 0);
        while end < len && used + char_width(chars[end]) < budget {
            used += char_width(chars[end]);
            end += 1;
        }
        vec![Keep(from, end), Gap]
    };
    let name = chars.iter().rposition(|&c| c == '/').map_or(0, |i| i + 1);
    let top = chars.iter().position(|&c| c == '/').map_or(0, |i| i + 1);
    if top > 0 {
        let top_w = w(0, top);
        // Leave out the folders in between, from the outside in.
        for b in (top..name).filter(|&i| chars[i] == '/') {
            if top_w + 1 + w(b, len) <= room {
                return vec![Keep(0, top), Gap, Keep(b, len)];
            }
        }
        // Then the end of the file name.
        let mut pieces = vec![Keep(0, top)];
        let from = if name > top {
            pieces.push(Gap);
            name - 1
        } else {
            top
        };
        let used = w(0, top) + usize::from(name > top);
        let rest = clip(from, room.saturating_sub(used));
        if let Some(Keep(start, end)) = rest.first()
            && end.saturating_sub((*start).max(name)) >= MIN_NAME.min(len - name)
        {
            pieces.extend(rest);
            return pieces;
        }
    }
    let mut pieces = Vec::new();
    let from = if name > 0 {
        pieces.push(Gap);
        name - 1
    } else {
        0
    };
    pieces.extend(clip(from, room.saturating_sub(usize::from(name > 0))));
    pieces
}

/// One row of the file list: the path, with its directory dimmed and filter
/// matches highlighted, and the age right-aligned. Long paths lose the
/// folders in between, then the end of the file name: see [`fit_path`].
/// A rule across the list, with `label` in it if there is one.
fn rule_line(label: &str, width: usize) -> Line<'static> {
    let line = if label.is_empty() {
        format!(" {}", "─".repeat(width.saturating_sub(2)))
    } else {
        let rest = width.saturating_sub(6 + wrap::width(label));
        format!(" ── {label} {}", "─".repeat(rest))
    };
    Line::from(line).dim()
}

fn list_line(indent: usize, rel: &str, hits: &[u32], age: &str, width: usize) -> Line<'static> {
    let chars: Vec<char> = rel.chars().collect();
    let dir_len = rel.rfind('/').map_or(0, |i| rel[..=i].chars().count());
    let age_w = wrap::width(age);
    let room = width.saturating_sub(indent + age_w + 2).max(1);

    let hit = Style::new().yellow().bold();
    let mut spans: Vec<Span> = vec![Span::raw(" ".repeat(1 + indent))];
    let mut path_w = 0;
    for piece in fit_path(&chars, room) {
        let (start, end) = match piece {
            Piece::Gap => {
                spans.push("…".dim());
                path_w += 1;
                continue;
            }
            Piece::Keep(start, end) => (start, end),
        };
        for (i, &c) in chars.iter().enumerate().take(end).skip(start) {
            path_w += char_width(c);
            let style = if hits.binary_search(&(i as u32)).is_ok() {
                hit
            } else if i < dir_len {
                Style::new().dim()
            } else {
                Style::new()
            };
            match spans.last_mut() {
                Some(last) if last.style == style && last.content != "…" => {
                    last.content.to_mut().push(c)
                }
                _ => spans.push(Span::styled(c.to_string(), style)),
            }
        }
    }
    let gap = width.saturating_sub(1 + indent + path_w + age_w);
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
            "esc ⌫ ← h",
            "Back a document, then to the list (esc quits there)",
        ),
        ("q", "Quit"),
        ("^z", "Suspend: fg in the shell comes back"),
        ("] [", "Next / previous heading"),
        ("o", "Outline: the text follows as you move (/ filters)"),
        ("O", "Keep the outline open beside the text"),
        ("L", "Links: to and from this document"),
        ("s", "Search the text of every file"),
        ("T", "Pick a color theme"),
        ("e", "Edit the file in $EDITOR, at this point"),
        ("c", "Copy: code, section, all, a selection, link, path"),
        ("/ n N", "Search; next / previous match"),
        ("^s ^r", "Search forward / back; typing: next / previous"),
        ("f", "Follow a link (type the letters shown on it)"),
        ("⇧↓ ⇧↑ J K", "Page down / up"),
        ("space b", "Page down / up (the preview, in the list)"),
        ("d u", "Half page down / up"),
        ("g G", "Top / bottom"),
        ("^n ^p", "Emacs: down / up"),
        ("^v M-v", "Emacs: page down / up"),
        ("M-< M->", "Emacs: top / bottom (^g: esc)"),
        ("/", "Filter files by name (fuzzy), then text"),
        ("→ ←", "In the list: into a folder / up a folder"),
        ("space", "Look inside a folder, or close it (in the list)"),
        ("m", "Sort by name or by date"),
        (".", "Show or hide hidden files"),
        ("\\", "Show or hide the list while reading"),
        ("tab", "Between the list and the document"),
        ("S", "Show the source beside the rendered text"),
        ("#", "Show or hide line numbers"),
        ("^w", "Switch between source and rendered"),
        ("< >", "Move the divider left / right"),
        ("Q", "Quit from anywhere"),
    ];
    let key_width = KEYS.iter().map(|(k, _)| wrap::width(k)).max().unwrap_or(0);
    let rows: Vec<Line> = KEYS
        .iter()
        .map(|(k, what)| {
            let pad = key_width - wrap::width(k);
            Line::from(vec![
                format!("{k}{}   ", " ".repeat(pad)).bold(),
                Span::raw(*what),
            ])
        })
        .collect();
    let area = f.area();
    let mut lines = help_columns(rows, area);
    lines.push(Line::default());
    lines.push(Line::from("Press any key to close").dim());
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

/// "Install › On Windows", dropping outer sections to fit `room` columns.
fn section_trail(section: &[String], room: usize) -> Option<String> {
    for skip in 0..section.len() {
        let mut trail = section[skip..].join(" › ");
        if skip > 0 {
            trail.insert_str(0, "… › ");
        }
        if wrap::width(&trail) <= room {
            return Some(trail);
        }
    }
    // Even the innermost section alone is too long: cut it short, unless
    // there's too little room for that to say anything.
    let last = section.last()?;
    if room < 8 {
        return None;
    }
    let mut trail = String::from("… › ");
    for c in last.chars() {
        if wrap::width(&trail) + wrap::width(c.encode_utf8(&mut [0; 4])) + 1 > room {
            break;
        }
        trail.push(c);
    }
    trail.push('…');
    Some(trail)
}

/// The help's rows, in one column, or in two side by side when one is too
/// tall for the screen and two fit across it.
fn help_columns(rows: Vec<Line<'static>>, area: Rect) -> Vec<Line<'static>> {
    // The border and padding, and the blank line and "Press any key" below.
    const FRAME_HEIGHT: usize = 4;
    const FRAME_WIDTH: usize = 4;
    const GAP: usize = 4;
    let column = rows.iter().map(Line::width).max().unwrap_or(0);
    let tall = rows.len() + FRAME_HEIGHT > usize::from(area.height);
    if !tall || 2 * column + GAP + FRAME_WIDTH > usize::from(area.width) {
        return rows;
    }
    let half = rows.len().div_ceil(2);
    let mut rows = rows.into_iter();
    let left: Vec<Line> = rows.by_ref().take(half).collect();
    let right: Vec<Line> = rows.collect();
    let mut right = right.into_iter();
    left.into_iter()
        .map(|mut line| {
            if let Some(r) = right.next() {
                let pad = column - line.width() + GAP;
                line.spans.push(Span::raw(" ".repeat(pad)));
                line.spans.extend(r.spans);
            }
            line
        })
        .collect()
}

/// Emacs's movement keys, as the keys they stand for, so they work
/// wherever those do: C-n C-p for ↓ ↑, C-v M-v for a page, M-< M-> for
/// the ends, and C-g for Esc.
fn emacs(key: KeyEvent) -> KeyEvent {
    let ctrl = key.modifiers == KeyModifiers::CONTROL;
    let alt = key.modifiers.difference(KeyModifiers::SHIFT) == KeyModifiers::ALT;
    let code = match key.code {
        KeyCode::Char('n') if ctrl => KeyCode::Down,
        KeyCode::Char('p') if ctrl => KeyCode::Up,
        KeyCode::Char('v') if ctrl => KeyCode::PageDown,
        KeyCode::Char('v') if alt => KeyCode::PageUp,
        KeyCode::Char('<') if alt => KeyCode::Home,
        KeyCode::Char('>') if alt => KeyCode::End,
        KeyCode::Char('g') if ctrl => KeyCode::Esc,
        _ => return key,
    };
    KeyEvent::new(code, KeyModifiers::NONE)
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

    fn scanned(root: &Path, open: Option<PathBuf>) -> App {
        let mut app = App::new(
            Source::Browse {
                root: root.to_path_buf(),
                open,
                all: false,
            },
            Theme::plain(),
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
        let app = scanned(&dir, Some(dir.join("sub/c.md")));
        assert_eq!(app.selected().unwrap().rel, "sub/c.md");

        // Without a README, the first row: the folder.
        let app = scanned(&dir, None);
        assert_eq!(app.selected_dir().unwrap().rel, "sub/");
        std::fs::write(dir.join("README.md"), "# x\n").unwrap();
        let app = scanned(&dir, None);
        assert_eq!(app.selected().unwrap().rel, "README.md");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn goes_into_folders_and_back_out() {
        let dir = std::env::temp_dir().join(format!("lsmd-tree-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("docs/api/v2/deep")).unwrap();
        for f in [
            "README.md",
            "docs/a.md",
            "docs/api/v2/deep/b.md",
            "docs/api/v2/deep/c.md",
        ] {
            std::fs::write(dir.join(f), "# x\n").unwrap();
        }
        let dir = std::fs::canonicalize(&dir).unwrap();
        let mut app = scanned(&dir, None);
        let row = |app: &App| match &app.shown[app.list.selected().unwrap()] {
            Shown::Dir(d) => d.rel.clone(),
            Shown::File { file, .. } => app.files[*file].rel.clone(),
            Shown::Up(to) => format!("..{to}"),
            Shown::Rule(_) => "─".into(),
        };
        let rows = |app: &App| -> Vec<String> {
            (0..app.shown.len())
                .map(|i| match &app.shown[i] {
                    Shown::Dir(d) => d.label.clone(),
                    Shown::File { file, from, .. } => app.files[*file].rel[*from..].into(),
                    Shown::Up(_) => "..".into(),
                    Shown::Rule(_) => "─".into(),
                })
                .collect()
        };
        // The folders and the files at the top, a rule, then every file in
        // the folders; starting on the first file.
        assert_eq!(
            rows(&app),
            [
                "docs/",
                "README.md",
                "─",
                "docs/a.md",
                "docs/api/v2/deep/b.md",
                "docs/api/v2/deep/c.md"
            ]
        );
        assert_eq!(row(&app), "README.md");
        // Moving steps over the rule, both ways.
        app.select_by(1);
        assert_eq!(row(&app), "docs/a.md");
        app.select_by(-1);
        assert_eq!(row(&app), "README.md");
        app.list.select(Some(0));

        // Down: `..` first, and paths from the folder.
        app.enter_dir();
        assert_eq!(app.listed.len(), 3, "only what's in docs/");
        assert_eq!(
            rows(&app),
            [
                "..",
                "api/v2/deep/",
                "a.md",
                "─",
                "api/v2/deep/b.md",
                "api/v2/deep/c.md"
            ],
            "a chain of lone folders is one row"
        );
        assert_eq!(row(&app), "docs/api/v2/deep/", "no README: the first row");
        // Looking inside without going in.
        app.toggle_dir();
        assert_eq!(app.shown.len(), 8);
        app.toggle_dir();
        app.enter_dir();
        assert!(
            matches!(&app.shown[0], Shown::Up(to) if to == "docs/"),
            "up skips the chain"
        );

        // Up: back to docs/, on the folder just left, then to the top.
        assert!(app.leave_dir());
        assert_eq!(app.scope(), "docs/");
        assert_eq!(row(&app), "docs/api/v2/deep/");
        app.select_by(-10);
        assert_eq!(row(&app), "..");
        assert!(app.leave_dir());
        assert_eq!(row(&app), "docs/");
        assert!(!app.leave_dir(), "nowhere further");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn folder_rows_keep_the_selection_and_the_top_readme() {
        let dir = std::env::temp_dir().join(format!("lsmd-keep-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::create_dir_all(dir.join(".hidden")).unwrap();
        for f in ["README.md", "docs/README.md", "docs/a.md", ".hidden/x.md"] {
            std::fs::write(dir.join(f), "# x\n").unwrap();
        }
        let dir = std::fs::canonicalize(&dir).unwrap();
        // Asked for a file the scan doesn't list: once it's done, the
        // selection follows the user again.
        let mut app = scanned(&dir, Some(dir.join(".hidden/x.md")));
        assert!(app.want.is_none());
        assert_eq!(app.count_w, 1, "folders: room for their counts");
        app.focus = Focus::List;
        app.list.select(Some(0));
        app.moved = true;
        app.toggle_dir();
        assert_eq!(app.selected_dir().unwrap().rel, "docs/", "Space keeps it");

        // With docs/ open in place, the README to start on is the top one.
        app.moved = false;
        app.refresh();
        assert_eq!(app.selected().unwrap().rel, "README.md");

        // e on a folder asks for a file, rather than blaming stdin.
        app.list.select(Some(0));
        app.start_edit();
        assert_eq!(app.flash.as_deref(), Some("Choose a file to edit"));
        assert!(app.edit.is_none());

        // `.` scans again with hidden files, keeping the selection.
        app.list.select(Some(
            app.row_of(&tree::Pick::File(dir.join("docs/a.md")))
                .unwrap(),
        ));
        app.toggle_hidden();
        while app.scan.is_some() {
            app.receive();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.files.iter().any(|e| e.rel == ".hidden/x.md"));
        assert_eq!(app.selected().unwrap().rel, "docs/a.md");
        app.toggle_hidden();
        while app.scan.is_some() {
            app.receive();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!app.files.iter().any(|e| e.rel == ".hidden/x.md"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn filtering_finds_files_by_their_text_too() {
        let dir = std::env::temp_dir().join(format!("lsmd-found-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        for (f, text) in [
            ("install.md", "# Install\n"),
            ("docs/a.md", "# A\n\nTo install, run it.\n"),
            ("docs/b.md", "# B\n\ninstall\n\nInstall again\n"),
            ("docs/c.md", "# C\n"),
        ] {
            std::fs::write(dir.join(f), text).unwrap();
        }
        let dir = std::fs::canonicalize(&dir).unwrap();
        let mut app = scanned(&dir, None);
        app.list_key(KeyEvent::from(KeyCode::Char('/')), false);
        for c in "install".chars() {
            app.key(KeyEvent::from(KeyCode::Char(c)));
        }
        while app.text_search.busy() {
            app.receive();
            std::thread::sleep(Duration::from_millis(5));
        }
        let rows: Vec<String> = app
            .shown
            .iter()
            .map(|s| match s {
                Shown::File { file, found, .. } => format!("{} {found}", app.files[*file].rel),
                Shown::Rule(label) => format!("─ {label}"),
                _ => "?".into(),
            })
            .collect();
        // By name first; then by text, most lines first.
        assert_eq!(
            rows,
            [
                "install.md 0",
                "─ in the text",
                "docs/b.md 2",
                "docs/a.md 1"
            ]
        );
        assert_eq!(app.selected().unwrap().rel, "install.md");

        // Moving onto one found by its text shows its first match.
        app.select_by(1);
        assert_eq!(app.selected().unwrap().rel, "docs/b.md");
        app.preview_found();
        assert_eq!(
            app.text_search.previewed,
            Some((dir.join("docs/b.md"), "install".into()))
        );

        // Drawn, the preview is on the match.
        let b = dir.join("docs/b.md");
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        term.draw(|f| app.draw(f)).unwrap();
        assert_eq!(app.doc(&b).search_status().as_deref(), Some("1/2"));

        // Clearing the filter leaves the previewed file as it was.
        app.key(KeyEvent::from(KeyCode::Esc));
        term.draw(|f| app.draw(f)).unwrap();
        assert!(app.doc(&b).search_status().is_none());
        assert_eq!(app.doc(&b).top(), 0);
        app.key(KeyEvent::from(KeyCode::Char('/')));
        for c in "install".chars() {
            app.key(KeyEvent::from(KeyCode::Char(c)));
        }
        while app.text_search.busy() {
            app.receive();
            std::thread::sleep(Duration::from_millis(5));
        }

        // Typing more searches again; nothing found by name, the first
        // found by its text is selected.
        app.key(KeyEvent::from(KeyCode::Char(',')));
        assert!(app.text_search.busy());
        while app.text_search.busy() {
            app.receive();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(app.selected().unwrap().rel, "docs/a.md");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn tab_goes_between_the_list_and_the_document() {
        let dir = std::env::temp_dir().join(format!("lsmd-tab-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.md"), "# A\n\n[to b](b.md)\n").unwrap();
        std::fs::write(dir.join("b.md"), "# B\n").unwrap();
        let dir = std::fs::canonicalize(&dir).unwrap();
        let tab = KeyEvent::from(KeyCode::Tab);
        for (width, beside) in [(160, true), (100, false)] {
            let mut app = scanned(&dir, None);
            app.body_width = width;
            app.list
                .select(app.row_of(&tree::Pick::File(dir.join("a.md"))));
            app.key(tab);
            assert!(app.focus == Focus::Reader);
            assert_eq!(app.reading, Some(dir.join("a.md")));
            assert_eq!(app.list_in_reader, beside, "{width}");

            // Follow a link, then Tab to the list: it still shows b.md.
            app.follow("b.md");
            assert_eq!(app.reading, Some(dir.join("b.md")));
            app.key(tab);
            assert!(app.focus == Focus::List && app.kept());
            assert_eq!(app.current_path(), Some(dir.join("b.md")));
            // And Tab goes back to it, with the way back to a.md.
            app.key(tab);
            assert!(app.focus == Focus::Reader);
            assert_eq!(app.reading, Some(dir.join("b.md")));
            assert!(!app.history.is_empty());

            // Choosing another file in the list ends that.
            app.key(tab);
            app.select_by(1);
            assert!(!app.kept());
            let selected = app.selected().map(|e| e.path.clone());
            assert_eq!(app.current_path(), selected);
        }
        // S shows the source; Tab doesn't.
        let mut app = scanned(&dir, None);
        app.key(KeyEvent::from(KeyCode::Char('S')));
        assert!(app.split);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn section_trail_drops_outer_sections_to_fit() {
        let s: Vec<String> = ["Guide", "Install", "On Windows"].map(String::from).into();
        assert_eq!(
            section_trail(&s, 80).unwrap(),
            "Guide › Install › On Windows"
        );
        assert_eq!(section_trail(&s, 25).unwrap(), "… › Install › On Windows");
        assert_eq!(section_trail(&s, 15).unwrap(), "… › On Windows");
        assert_eq!(section_trail(&s, 10).unwrap(), "… › On Wi…");
        assert_eq!(section_trail(&s, 5), None);
        assert_eq!(section_trail(&[], 80), None);
    }

    #[test]
    fn emacs_keys_stand_for_the_usual_ones() {
        let k = |c, m| emacs(KeyEvent::new(KeyCode::Char(c), m)).code;
        assert_eq!(k('n', KeyModifiers::CONTROL), KeyCode::Down);
        assert_eq!(k('p', KeyModifiers::CONTROL), KeyCode::Up);
        assert_eq!(k('v', KeyModifiers::CONTROL), KeyCode::PageDown);
        assert_eq!(k('v', KeyModifiers::ALT), KeyCode::PageUp);
        assert_eq!(
            k('<', KeyModifiers::ALT | KeyModifiers::SHIFT),
            KeyCode::Home
        );
        assert_eq!(k('>', KeyModifiers::ALT), KeyCode::End);
        assert_eq!(k('g', KeyModifiers::CONTROL), KeyCode::Esc);
        // Plain letters, and < > without Alt (the divider), are left alone.
        assert_eq!(k('n', KeyModifiers::NONE), KeyCode::Char('n'));
        assert_eq!(k('<', KeyModifiers::SHIFT), KeyCode::Char('<'));
    }

    #[test]
    fn help_goes_to_two_columns_only_when_too_short() {
        let rows =
            || -> Vec<Line<'static>> { (0..10).map(|i| Line::from(format!("row {i}"))).collect() };
        let area = |width, height| Rect::new(0, 0, width, height);
        // Room for all of it: one column.
        assert_eq!(help_columns(rows(), area(80, 14)).len(), 10);
        // Too short: two columns, the second half beside the first.
        let two = help_columns(rows(), area(80, 13));
        assert_eq!(two.len(), 5);
        assert_eq!(text(&two[0]), "row 0    row 5");
        // Too short, but too narrow for two: one column, clipped.
        assert_eq!(help_columns(rows(), area(17, 13)).len(), 10);
    }

    #[test]
    fn list_line_right_aligns_age() {
        assert_eq!(
            text(&list_line(0, "docs/a.md", &[], "2h ago", 20)),
            " docs/a.md    2h ago"
        );
    }

    #[test]
    fn long_paths_keep_the_top_folder_and_the_start_of_the_name() {
        let fit = |path: &str, room| {
            let chars: Vec<char> = path.chars().collect();
            fit_path(&chars, room)
                .into_iter()
                .map(|p| match p {
                    Piece::Keep(a, b) => chars[a..b].iter().collect(),
                    Piece::Gap => "…".to_string(),
                })
                .collect::<String>()
        };
        let deep = "src/App.Desktop/Platform/MacOS/rnnoise/VENDOR.md";
        assert_eq!(fit(deep, 60), deep);
        assert_eq!(fit(deep, 30), "src/…/MacOS/rnnoise/VENDOR.md");
        assert_eq!(fit(deep, 16), "src/…/VENDOR.md");
        // Too little room for the folder and enough of the name.
        assert_eq!(fit(deep, 14), "…/VENDOR.md");
        assert_eq!(fit(deep, 10), "…/VENDOR.…");
        let long = "docs/RELEASE_CHECKLIST_2026-09-27_FOLLOWUPS.md";
        assert_eq!(fit(long, 30), "docs/RELEASE_CHECKLIST_2026-0…");
        assert_eq!(fit("README_WITH_A_VERY_LONG_NAME.md", 12), "README_WITH…");
    }

    #[test]
    fn list_line_highlights_hits_around_a_gap() {
        // "docs/…/b.md", with the hit on the "b".
        let line = list_line(0, "docs/aaaaaaaaaaaaaaaa/b.md", &[22], "", 14);
        assert_eq!(text(&line), " docs/…/b.md");
        let hit = line.spans.iter().find(|s| s.style.fg.is_some()).unwrap();
        assert_eq!(hit.content, "b");
    }

    #[test]
    fn list_line_gives_up_the_folder_for_the_name() {
        let line = list_line(0, "very/long/directory/name.md", &[], "1d ago", 20);
        assert_eq!(text(&line), " …/name.md    1d ago");
    }
}
