//! Filtering the list finds files by their text too: a moment after typing
//! stops, the files in view are searched for the filter, and those that
//! have it in them but not in their names are listed below the rest.
//! Selecting one shows its first match.

use super::{App, Focus, Shown};
use crate::grep;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

/// How long typing has to stop for before the text is searched.
const PAUSE: Duration = Duration::from_millis(200);

/// Shorter filters don't search the text: nearly every file would match.
const MIN_CHARS: usize = 2;

/// A search's result to come, and how to stop it.
type Running = (Receiver<HashMap<PathBuf, usize>>, Arc<AtomicBool>);

#[derive(Default)]
pub struct TextSearch {
    /// When to start searching, once typing stops.
    due: Option<Instant>,
    /// A search under way, and how to stop it.
    running: Option<Running>,
    /// The files with the filter in them, and how many lines each.
    found: Option<HashMap<PathBuf, usize>>,
    /// The file whose preview was last moved to its first match, and the
    /// filter it was moved for.
    pub(super) previewed: Option<(PathBuf, String)>,
}

impl TextSearch {
    /// Whether it's waiting to search, or searching.
    pub fn busy(&self) -> bool {
        self.due.is_some() || self.running.is_some()
    }

    fn stop(&mut self) {
        if let Some((_, stop)) = self.running.take() {
            stop.store(true, Ordering::Relaxed);
        }
    }
}

impl App {
    /// The filter or the files in view have changed: whatever was found is
    /// out of date, so search again once typing stops.
    pub(super) fn filter_changed(&mut self) {
        let text = &mut self.text_search;
        text.stop();
        text.found = None;
        text.due = (self.filter.chars().count() >= MIN_CHARS).then(|| Instant::now() + PAUSE);
    }

    /// Starts the search once typing has stopped, and takes in what it
    /// found.
    pub(super) fn receive_found(&mut self) {
        let text = &mut self.text_search;
        if text.due.is_some_and(|due| Instant::now() >= due) {
            text.due = None;
            let scope = self.scope.clone();
            let files = self
                .files
                .iter()
                .filter(|e| e.rel.starts_with(&scope))
                .map(|e| e.path.clone())
                .collect();
            let stop = Arc::new(AtomicBool::new(false));
            let rx = grep::count(files, self.filter.clone(), Arc::clone(&stop));
            self.text_search.running = Some((rx, stop));
        }
        let text = &mut self.text_search;
        let Some((rx, _)) = &text.running else { return };
        if let Ok(found) = rx.try_recv() {
            text.running = None;
            text.found = Some(found);
            self.refresh();
        }
    }

    /// The rows for files in `order` found by their text and not in
    /// `named` (those found by name), most matches first, under a rule.
    pub(super) fn found_rows(&self, order: &[usize], named: &[usize]) -> Vec<Shown> {
        let Some(found) = &self.text_search.found else {
            return Vec::new();
        };
        let named: HashSet<usize> = named.iter().copied().collect();
        let mut rows: Vec<(usize, usize)> = order
            .iter()
            .filter(|i| !named.contains(i))
            .filter_map(|&i| found.get(&self.files[i].path).map(|&n| (i, n)))
            .collect();
        // Stable: equals stay in the list's order.
        rows.sort_by_key(|&(_, n)| std::cmp::Reverse(n));
        if rows.is_empty() {
            return Vec::new();
        }
        let mut out = vec![Shown::Rule("in the text")];
        out.extend(rows.into_iter().map(|(file, n)| Shown::File {
            file,
            hits: Vec::new(),
            from: self.scope.len(),
            depth: 0,
            found: n,
        }));
        out
    }

    /// With a file found by its text selected, moves its preview to the
    /// first match (once, so it can still be paged through).
    pub(super) fn preview_found(&mut self) {
        if self.focus != Focus::List {
            return;
        }
        let Some(Shown::File { file, found, .. }) =
            self.list.selected().and_then(|i| self.shown.get(i))
        else {
            return;
        };
        if *found == 0 {
            return;
        }
        let path = self.files[*file].path.clone();
        let want = Some((path.clone(), self.filter.clone()));
        if self.text_search.previewed == want {
            return;
        }
        self.text_search.previewed = want;
        let query = self.filter.clone();
        self.doc(&path).go_to_match(1, &query);
    }
}
