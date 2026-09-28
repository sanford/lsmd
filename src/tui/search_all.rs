//! Searching every file's text (`s`), and showing what's found.

use super::App;
use super::picker::{Picker, Row, Target};
use crate::grep::{self, Hit};
use crate::wrap;
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};

/// Show at most this many columns of a matching line.
const SNIPPET: usize = 100;

impl App {
    pub(super) fn start_grep(&mut self, query: String) {
        if query.is_empty() {
            return;
        }
        // The files in the list, in its order (and filter).
        let files: Vec<_> = self
            .shown
            .iter()
            .map(|s| &self.files[s.file])
            .map(|e| (e.path.clone(), e.rel.clone()))
            .collect();
        if files.is_empty() {
            self.flash = Some("No files to search".into());
            return;
        }
        self.grep = Some((grep::search(files, query.clone()), query));
        self.flash = Some("Searching…".into());
    }

    /// Shows the search's results, once they're in.
    pub(super) fn receive_grep(&mut self) {
        let Some((rx, _)) = &self.grep else { return };
        let Ok(hits) = rx.try_recv() else { return };
        let (_, query) = self.grep.take().unwrap();
        self.flash = None;
        if hits.is_empty() {
            self.flash = Some(format!("No matches for “{query}”"));
            return;
        }
        self.prompt = Some(super::nav::Prompt::Pick(results(hits, &query)));
    }
}

fn results(hits: Vec<Hit>, query: &str) -> Picker {
    let capped = hits.len() >= grep::MAX_HITS;
    let mut files = 0;
    let mut rows = Vec::new();
    let mut i = 0;
    while i < hits.len() {
        let rel = hits[i].rel.clone();
        let n = hits[i..].iter().take_while(|h| h.rel == rel).count();
        files += 1;
        rows.push(Row::heading(&format!("{rel} ({n})")));
        for hit in &hits[i..i + n] {
            let text = format!("{} {}", hit.rel, hit.text);
            let target = Target::Match(hit.path.clone(), hit.line, query.to_string());
            rows.push(Row::item(text, snippet(hit), target));
        }
        i += n;
    }
    let s = |n: usize| if n == 1 { "" } else { "es" };
    let mut title = format!(
        "{} match{} for “{query}” in {files} file{}",
        hits.len(),
        s(hits.len()),
        if files == 1 { "" } else { "s" }
    );
    if capped {
        title.push_str(" (stopped there)");
    }
    Picker::new(title, rows, None)
}

/// A matching line, trimmed to fit, with the match highlighted.
fn snippet(hit: &Hit) -> Line<'static> {
    let text = hit.text.trim_start();
    let cut = hit.text.len() - text.len();
    let (start, end) = (
        hit.range.start.saturating_sub(cut),
        hit.range.end.saturating_sub(cut),
    );
    // Start a little before the match if the line is long.
    let mut from = 0;
    if wrap::width(text) > SNIPPET && wrap::width(&text[..start]) > SNIPPET / 3 {
        from = text[..start]
            .char_indices()
            .rev()
            .scan(0, |w, (i, c)| {
                *w += wrap::width(c.encode_utf8(&mut [0; 4]));
                Some((i, *w))
            })
            .find(|&(_, w)| w >= SNIPPET / 3)
            .map_or(0, |(i, _)| i);
    }
    let mut spans = vec![Span::raw(format!("   {:>4}  ", hit.line)).dim()];
    if from > 0 {
        spans.push("…".dim());
    }
    spans.push(Span::raw(text[from..start].to_string()));
    spans.push(Span::raw(text[start..end].to_string()).yellow().bold());
    let rest = &text[end..];
    let room = SNIPPET.saturating_sub(wrap::width(&text[from..end]));
    let shown: String = rest
        .chars()
        .scan(0, |w, c| {
            *w += wrap::width(c.encode_utf8(&mut [0; 4]));
            (*w <= room).then_some(c)
        })
        .collect();
    let more = shown.len() < rest.len();
    spans.push(Span::raw(shown));
    if more {
        spans.push("…".dim());
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn hit(text: &str, query: &str) -> Hit {
        Hit {
            path: PathBuf::from("a.md"),
            rel: "a.md".into(),
            line: 7,
            text: text.into(),
            range: grep::find(text, query).unwrap(),
        }
    }

    fn plain(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn trims_long_lines_around_the_match() {
        let text = format!("{} needle {}", "x".repeat(200), "y".repeat(200));
        let line = snippet(&hit(&text, "needle"));
        let shown = plain(&line);
        assert!(shown.starts_with("      7  …"));
        assert!(shown.contains("needle"));
        assert!(wrap::width(&shown) < SNIPPET + 15);
    }

    #[test]
    fn keeps_short_lines_whole() {
        assert_eq!(
            plain(&snippet(&hit("  find the needle here", "needle"))),
            "      7  find the needle here"
        );
    }
}
