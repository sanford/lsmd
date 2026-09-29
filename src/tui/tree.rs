//! The folders at the top of the file list, so a project with thousands
//! of files is easy to get around. Each folder shows how many files are
//! under it and when the newest changed. `→` goes into a folder, listing
//! just what's in it, and `←` (or the `..` row) comes back up; `Space`
//! opens a folder in place, to look inside without going in.

use super::{App, Shown};
use crate::files::Entry;
use crate::wrap;
use nucleo_matcher::pattern::Pattern;
use nucleo_matcher::{Matcher, Utf32Str};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::time::SystemTime;

/// While filtering, the most folders to show above the matching files.
const MATCHING_DIRS: usize = 5;

/// A folder's row.
pub struct Dir {
    /// Its path from the root, ending in `/`: what opens and closes it.
    pub rel: String,
    /// Its name, or the names of a chain of folders that each hold only
    /// the next one: `web/components/`.
    pub label: String,
    pub depth: usize,
    pub open: bool,
    /// Files under it.
    pub files: usize,
    pub newest: Option<SystemTime>,
}

/// What's selected, to find again when the list changes.
pub enum Pick {
    /// The `..` row.
    Up,
    File(PathBuf),
    /// A folder, by its path from the root, ending in `/`.
    Dir(String),
}

#[derive(Default)]
struct Node {
    /// By lowercased name, then name: the order they're listed in.
    dirs: BTreeMap<(String, String), Node>,
    /// Files directly in it, in the list's order.
    files: Vec<usize>,
    count: usize,
    newest: Option<SystemTime>,
}

/// The tree's rows for `listed` (files below `scope`, already in order):
/// its folders, then the files directly in `scope`. Folders in `open` show
/// what's in them, or all of them with `all_open`.
pub fn rows(
    files: &[Entry],
    listed: &[usize],
    scope: &str,
    all_open: bool,
    open: &HashSet<String>,
    by_time: bool,
) -> Vec<Shown> {
    let mut root = Node::default();
    for &file in listed {
        let e = &files[file];
        let mut node = &mut root;
        let dirs = e.rel[scope.len()..].rsplit_once('/').map_or("", |(d, _)| d);
        for name in dirs.split('/').filter(|n| !n.is_empty()) {
            node.count += 1;
            node.newest = node.newest.max(e.modified);
            node = node
                .dirs
                .entry((name.to_lowercase(), name.to_string()))
                .or_default();
        }
        node.count += 1;
        node.newest = node.newest.max(e.modified);
        node.files.push(file);
    }
    let mut out = Vec::new();
    let tree = Tree {
        files,
        all_open,
        open,
        by_time,
    };
    tree.flatten(&root, scope, 0, &mut out);
    out
}

/// The list for `scope`, unfiltered: its folders and the files directly
/// in it, then every file in its folders, with its path from `scope`.
pub fn listing(
    files: &[Entry],
    listed: &[usize],
    scope: &str,
    open: &HashSet<String>,
    by_time: bool,
) -> Vec<Shown> {
    let mut out = rows(files, listed, scope, false, open, by_time);
    let mut nested = listed
        .iter()
        .filter(|&&i| files[i].rel[scope.len()..].contains('/'))
        .peekable();
    if nested.peek().is_some() {
        out.push(Shown::Rule(""));
    }
    out.extend(nested.map(|&file| Shown::File {
        file,
        hits: Vec::new(),
        from: scope.len(),
        depth: 0,
        found: 0,
    }));
    out
}

/// While filtering: the folders below `scope` (as the tree shows them)
/// whose paths match, best first, each with its score.
pub fn matching_dirs(
    files: &[Entry],
    listed: &[usize],
    scope: &str,
    by_time: bool,
    pattern: &Pattern,
    matcher: &mut Matcher,
) -> Vec<(u32, Dir)> {
    let mut buf = Vec::new();
    let mut found: Vec<(u32, Dir)> = rows(files, listed, scope, true, &HashSet::new(), by_time)
        .into_iter()
        .filter_map(|row| match row {
            Shown::Dir(d) => Some(d),
            _ => None,
        })
        .filter_map(|mut d| {
            let path = &d.rel[scope.len()..];
            let score = pattern.score(Utf32Str::new(path, &mut buf), matcher)?;
            d.label = path.to_string();
            d.depth = 0;
            d.open = false;
            Some((score, d))
        })
        .collect();
    // Best first; between equals, the shallower folder.
    found.sort_by_key(|(score, d)| (std::cmp::Reverse(*score), d.rel.matches('/').count()));
    found.truncate(MATCHING_DIRS);
    found
}

struct Tree<'a> {
    files: &'a [Entry],
    all_open: bool,
    open: &'a HashSet<String>,
    by_time: bool,
}

impl Tree<'_> {
    /// Adds `node`'s rows: its folders first, then its files. `rel` is its
    /// path from the root.
    fn flatten(&self, node: &Node, rel: &str, depth: usize, out: &mut Vec<Shown>) {
        let mut dirs: Vec<_> = node.dirs.iter().collect();
        if self.by_time {
            dirs.sort_by_key(|d| std::cmp::Reverse(d.1.newest));
        }
        for ((_, name), mut child) in dirs {
            let mut label = format!("{name}/");
            // Fold a chain of lone folders into one row.
            while child.files.is_empty() && child.dirs.len() == 1 {
                let ((_, name), only) = child.dirs.iter().next().unwrap();
                label.push_str(name);
                label.push('/');
                child = only;
            }
            let dir_rel = format!("{rel}{label}");
            let open = self.all_open || self.open.contains(&dir_rel);
            out.push(Shown::Dir(Dir {
                rel: dir_rel.clone(),
                label,
                depth,
                open,
                files: child.count,
                newest: child.newest,
            }));
            if open {
                self.flatten(child, &dir_rel, depth + 1, out);
            }
        }
        for &file in &node.files {
            let from = self.files[file].rel.rfind('/').map_or(0, |i| i + 1);
            out.push(Shown::File {
                file,
                hits: Vec::new(),
                from,
                depth,
                found: 0,
            });
        }
    }
}

/// A folder's row: its name, the age of its newest file in the column
/// the files' ages are in, and how many files are under it at the far
/// right, in a column `count_w` wide.
pub fn dir_line(dir: &Dir, age: &str, width: usize, count_w: usize) -> Line<'static> {
    let indent = " ".repeat(1 + 2 * dir.depth);
    let marker = if dir.open { "▾ " } else { "▸ " };
    let count = format!("{:>count_w$}", dir.files);
    let column = count_column(count_w);
    let age_w = wrap::width(age);
    let fixed = wrap::width(&indent) + 2;
    let room = width.saturating_sub(fixed + age_w + column + 1).max(1);
    let mut label = dir.label.clone();
    if wrap::width(&label) > room {
        // Keep the end: the folder itself, rather than the ones it's in.
        let chars: Vec<char> = label.chars().collect();
        let mut skip = 0;
        while skip < chars.len()
            && wrap::width(&chars[skip..].iter().collect::<String>()) + 1 > room
        {
            skip += 1;
        }
        label = format!("…{}", chars[skip..].iter().collect::<String>());
    }
    let used = fixed + wrap::width(&label);
    let mut spans = vec![
        Span::raw(indent),
        Span::styled(marker, Style::new().dim()),
        Span::raw(label).bold(),
    ];
    let gap = width.saturating_sub(used + age_w + column);
    spans.push(Span::raw(" ".repeat(gap)));
    spans.push(Span::raw(age.to_string()).dim());
    spans.push(Span::raw(format!("  {count}")));
    Line::from(spans)
}

/// Columns the folders' counts take at the right of the list, gap included:
/// none when there are no folders.
pub fn count_column(count_w: usize) -> usize {
    if count_w == 0 { 0 } else { count_w + 2 }
}

/// Columns a row wants, before the age.
pub fn row_width(files: &[Entry], row: &Shown) -> usize {
    match row {
        Shown::File {
            file, from, depth, ..
        } => 1 + indent(*depth) + wrap::width(&files[*file].rel[*from..]),
        Shown::Dir(d) => 1 + 2 * d.depth + 2 + wrap::width(&d.label),
        Shown::Up(to) => 1 + 2 + wrap::width(&up_label(to)),
        Shown::Rule(label) => 6 + wrap::width(label),
    }
}

/// The `..` row: where it goes.
pub fn up_line(to: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw(" "),
        "..".bold(),
        Span::raw(up_label(to)).dim(),
    ])
}

fn up_label(to: &str) -> String {
    match to {
        "" => "  up to the top".into(),
        to => format!("  up to {to}"),
    }
}

/// Spaces before a file's name: under its folder's name, past the folders'
/// `▸`.
pub fn indent(depth: usize) -> usize {
    2 * depth + 2
}

impl App {
    /// The folder the list shows, ending in `/`, or `""` for the root.
    pub(super) fn scope(&self) -> &str {
        &self.scope
    }

    /// The folder above `scope`, the way the tree shows folders: past any
    /// that hold nothing but the one folder, since those fold into it.
    pub(super) fn parent_scope(&self, scope: &str) -> String {
        let mut up = scope.trim_end_matches('/');
        while let Some(i) = up.rfind('/') {
            up = &up[..i];
            let dir = format!("{up}/");
            if !self.lone_folder(&dir) {
                return dir;
            }
        }
        String::new()
    }

    /// Whether `dir` holds no files of its own and just one folder.
    fn lone_folder(&self, dir: &str) -> bool {
        let mut only = None;
        for e in &self.files {
            let Some(rest) = e.rel.strip_prefix(dir) else {
                continue;
            };
            match rest.split_once('/') {
                None => return false,
                Some((name, _)) if only.is_none_or(|o| o == name) => only = Some(name),
                Some(_) => return false,
            }
        }
        true
    }

    pub(super) fn picked(&self) -> Option<Pick> {
        match self.shown.get(self.list.selected()?)? {
            Shown::Up(_) => Some(Pick::Up),
            Shown::File { file, .. } => Some(Pick::File(self.files[*file].path.clone())),
            Shown::Dir(d) => Some(Pick::Dir(d.rel.clone())),
            Shown::Rule(_) => None,
        }
    }

    pub(super) fn selected_dir(&self) -> Option<&Dir> {
        match self.shown.get(self.list.selected()?)? {
            Shown::Dir(d) => Some(d),
            _ => None,
        }
    }

    /// Whether the `..` row is selected.
    pub(super) fn on_up(&self) -> bool {
        matches!(self.picked(), Some(Pick::Up))
    }

    /// The row showing `pick`, or failing that, the nearest folder
    /// around it, or the first file in it.
    pub(super) fn row_of(&self, pick: &Pick) -> Option<usize> {
        let rel = match pick {
            Pick::Up => return matches!(self.shown.first(), Some(Shown::Up(_))).then_some(0),
            Pick::File(path) => {
                let exact = self.shown.iter().position(
                    |s| matches!(s, Shown::File { file, .. } if self.files[*file].path == *path),
                );
                if exact.is_some() {
                    return exact;
                }
                self.files.iter().find(|e| &e.path == path)?.rel.clone()
            }
            Pick::Dir(rel) => rel.clone(),
        };
        // The deepest folder it's in, then one it's folded into.
        let around = self
            .shown
            .iter()
            .enumerate()
            .filter_map(|(i, s)| match s {
                Shown::Dir(d) if rel.starts_with(&d.rel) => Some((d.depth, i)),
                _ => None,
            })
            .max()
            .map(|(_, i)| i);
        around.or_else(|| {
            self.shown.iter().position(|s| match s {
                Shown::Dir(d) => d.rel.starts_with(&rel),
                Shown::File { file, .. } => self.files[*file].rel.starts_with(&rel),
                Shown::Up(_) | Shown::Rule(_) => false,
            })
        })
    }

    /// `Space` on a folder: opens or closes it in place, to see what's in
    /// it without going in. (Not while filtering.)
    pub(super) fn toggle_dir(&mut self) {
        let Some(d) = self.selected_dir() else { return };
        if !self.filter.is_empty() {
            return;
        }
        let rel = d.rel.clone();
        if !self.open.remove(&rel) {
            self.open.insert(rel);
        }
        self.moved = true;
        self.refresh();
    }

    /// `→` on a folder: lists only what's in it.
    pub(super) fn enter_dir(&mut self) {
        let Some(d) = self.selected_dir() else { return };
        self.scope = d.rel.clone();
        self.filter.clear();
        self.typing = false;
        self.filter_changed();
        self.moved = false;
        self.refresh();
    }

    /// `←`: up to the folder above, with the one just left selected. False
    /// when the whole root is listed already.
    pub(super) fn leave_dir(&mut self) -> bool {
        if self.scope.is_empty() {
            return false;
        }
        let left = std::mem::take(&mut self.scope);
        self.scope = self.parent_scope(&left);
        self.filter.clear();
        self.typing = false;
        self.filter_changed();
        // Select the folder just left, rather than what was selected in it.
        self.moved = false;
        self.refresh();
        self.moved = true;
        if let Some(i) = self.row_of(&Pick::Dir(left)) {
            self.list.select(Some(i));
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nucleo_matcher::Config;
    use nucleo_matcher::pattern::{CaseMatching, Normalization};

    fn entry(rel: &str) -> Entry {
        Entry {
            path: PathBuf::from("/r").join(rel),
            rel: rel.into(),
            modified: None,
        }
    }

    fn in_scope(files: &[Entry], scope: &str) -> Vec<usize> {
        (0..files.len())
            .filter(|&i| files[i].rel.starts_with(scope))
            .collect()
    }

    fn render(files: &[Entry], scope: &str, open: &[&str]) -> Vec<String> {
        let open: HashSet<String> = open.iter().map(|s| s.to_string()).collect();
        rows(files, &in_scope(files, scope), scope, false, &open, false)
            .iter()
            .map(|r| match r {
                Shown::Dir(d) => format!(
                    "{}{} {} ({})",
                    "  ".repeat(d.depth),
                    if d.open { "▾" } else { "▸" },
                    d.label,
                    d.files
                ),
                Shown::File {
                    file, from, depth, ..
                } => format!("{}{}", "  ".repeat(*depth), &files[*file].rel[*from..]),
                Shown::Up(_) => "..".into(),
                Shown::Rule(_) => "─".into(),
            })
            .collect()
    }

    fn sample() -> Vec<Entry> {
        [
            "README.md",
            "docs/a.md",
            "docs/api/b.md",
            "docs/api/c.md",
            "web/src/components/d.md",
        ]
        .map(entry)
        .into()
    }

    #[test]
    fn folders_come_first_with_counts_and_lone_chains_fold() {
        let files = sample();
        assert_eq!(
            render(&files, "", &[]),
            ["▸ docs/ (3)", "▸ web/src/components/ (1)", "README.md"]
        );
        assert_eq!(
            render(&files, "", &["docs/"]),
            [
                "▾ docs/ (3)",
                "  ▸ api/ (2)",
                "  a.md",
                "▸ web/src/components/ (1)",
                "README.md"
            ]
        );
        // Inside a folder, paths start there.
        assert_eq!(render(&files, "docs/", &[]), ["▸ api/ (2)", "a.md"]);
    }

    #[test]
    fn matching_dirs_are_by_path_below_the_scope() {
        let files = sample();
        let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
        let mut find = |query: &str, scope: &str| -> Vec<String> {
            let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
            let listed = in_scope(&files, scope);
            matching_dirs(&files, &listed, scope, false, &pattern, &mut matcher)
                .into_iter()
                .map(|(_, d)| d.label)
                .collect()
        };
        assert_eq!(find("api", ""), ["docs/api/"]);
        assert_eq!(find("comp", ""), ["web/src/components/"]);
        assert_eq!(find("api", "docs/"), ["api/"]);
        assert!(find("zzz", "").is_empty());
    }

    #[test]
    fn dir_line_puts_the_count_at_the_right() {
        let d = Dir {
            rel: "docs/".into(),
            label: "docs/".into(),
            depth: 0,
            open: false,
            files: 12,
            newest: None,
        };
        let text: String = dir_line(&d, "2h ago", 26, 3)
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(text, " ▸ docs/       2h ago   12");
    }
}
