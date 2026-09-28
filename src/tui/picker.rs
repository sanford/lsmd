//! A popup list to choose from, filtered by typing: the outline, the
//! links panel and the themes.

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph};
use std::path::PathBuf;

/// What choosing a row does.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// Jump to this rendered line of the current document.
    Line(usize),
    /// Follow a link as written in the current document.
    Link(String),
    /// Open this Markdown file.
    File(PathBuf),
    /// Open this file at a search match: source line and query.
    Match(PathBuf, usize, String),
    /// Switch to this theme.
    Theme(crate::theme::Choice),
}

pub struct Row {
    /// What the filter matches against.
    text: String,
    line: Line<'static>,
    /// `None` for section headings in the list, which can't be chosen.
    target: Option<Target>,
}

impl Row {
    pub fn item(text: String, line: Line<'static>, target: Target) -> Row {
        Row {
            text,
            line,
            target: Some(target),
        }
    }

    pub fn target(&self) -> Option<&Target> {
        self.target.as_ref()
    }

    pub fn choosable(&self) -> bool {
        self.target.is_some()
    }

    pub fn heading(text: &str) -> Row {
        Row {
            text: String::new(),
            line: Line::from(format!(" {text}")).bold().dim(),
            target: None,
        }
    }
}

pub enum Outcome {
    Stay,
    Close,
    Choose(Target),
    Quit,
}

pub struct Picker {
    title: String,
    rows: Vec<Row>,
    filter: String,
    /// Typing a filter, after `/`: letters go into it rather than moving.
    typing: bool,
    /// Selection among the visible rows.
    list: ListState,
    /// Rows on screen at the last draw, for paging.
    page: usize,
}

impl Picker {
    /// `selected` is an index into `rows`.
    pub fn new(title: String, rows: Vec<Row>, selected: Option<usize>) -> Picker {
        let mut picker = Picker {
            title,
            rows,
            filter: String::new(),
            typing: false,
            list: ListState::default(),
            page: 10,
        };
        match selected {
            Some(i) => picker.list.select(Some(i)),
            None => {
                picker.select_from(0, 1);
            }
        }
        picker
    }

    /// Indices of the rows that pass the filter. Headings only show
    /// unfiltered.
    fn visible(&self) -> Vec<usize> {
        if self.filter.is_empty() {
            return (0..self.rows.len()).collect();
        }
        let pattern = Pattern::parse(&self.filter, CaseMatching::Smart, Normalization::Smart);
        let mut matcher = Matcher::new(Config::DEFAULT);
        let mut buf = Vec::new();
        (0..self.rows.len())
            .filter(|&i| {
                let row = &self.rows[i];
                row.target.is_some()
                    && pattern
                        .score(Utf32Str::new(&row.text, &mut buf), &mut matcher)
                        .is_some()
            })
            .collect()
    }

    /// Selects the first choosable visible row from position `from`,
    /// looking in direction `step`. Returns whether it found one; if not,
    /// keeps the selection (unless it's no longer visible).
    fn select_from(&mut self, from: isize, step: isize) -> bool {
        let visible = self.visible();
        let mut i = from;
        while i >= 0 && (i as usize) < visible.len() {
            if self.rows[visible[i as usize]].target.is_some() {
                self.list.select(Some(i as usize));
                return true;
            }
            i += step;
        }
        if self.list.selected().is_none_or(|s| s >= visible.len()) {
            self.list.select(None);
        }
        false
    }

    pub fn key(&mut self, key: KeyEvent, ctrl: bool) -> Outcome {
        let at = self.list.selected().map_or(0, |s| s as isize);
        let page = self.page.max(1) as isize;
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let last = self.visible().len() as isize - 1;
        // Keys that do the same whether or not a filter is being typed.
        match key.code {
            KeyCode::Enter => return self.choose(),
            KeyCode::Down if shift => return self.move_to(at + page, 1),
            KeyCode::Up if shift => return self.move_to(at - page, -1),
            KeyCode::Down => return self.move_to(at + 1, 1),
            KeyCode::Up => return self.move_to(at - 1, -1),
            KeyCode::PageDown => return self.move_to(at + page, 1),
            KeyCode::PageUp => return self.move_to(at - page, -1),
            KeyCode::Char('c') if ctrl => return Outcome::Quit,
            _ => {}
        }
        if self.typing {
            match key.code {
                KeyCode::Esc => {
                    self.typing = false;
                    self.filter.clear();
                }
                // Deleting past the start stops filtering, like Esc.
                KeyCode::Backspace | KeyCode::Delete if self.filter.is_empty() => {
                    self.typing = false;
                }
                KeyCode::Backspace | KeyCode::Delete => {
                    self.filter.pop();
                }
                KeyCode::Char(c) if !ctrl => self.filter.push(c),
                _ => return Outcome::Stay,
            }
            self.select_from(0, 1);
            return Outcome::Stay;
        }
        match key.code {
            KeyCode::Esc => Outcome::Close,
            KeyCode::Char('q') | KeyCode::Char('Q') => Outcome::Quit,
            KeyCode::Char('/') => {
                self.typing = true;
                Outcome::Stay
            }
            KeyCode::Char('j') => self.move_to(at + 1, 1),
            KeyCode::Char('k') => self.move_to(at - 1, -1),
            KeyCode::Char('J') => self.move_to(at + page, 1),
            KeyCode::Char('K') => self.move_to(at - page, -1),
            KeyCode::Char('g') | KeyCode::Home => self.move_to(0, 1),
            KeyCode::Char('G') | KeyCode::End => self.move_to(last, -1),
            KeyCode::Char('l') | KeyCode::Right => self.choose(),
            _ => Outcome::Stay,
        }
    }

    /// Selects the nearest choosable row to position `to`, looking in
    /// direction `step` (and back the other way at the ends).
    fn move_to(&mut self, to: isize, step: isize) -> Outcome {
        let last = self.visible().len() as isize - 1;
        let to = to.clamp(0, last.max(0));
        // Past the end there may be only a heading row: then look back.
        if !self.select_from(to, step) {
            self.select_from(to, -step);
        }
        Outcome::Stay
    }

    /// What the selected row points to.
    pub fn selected(&self) -> Option<&Target> {
        let visible = self.visible();
        let i = visible.get(self.list.selected()?)?;
        self.rows[*i].target.as_ref()
    }

    fn choose(&self) -> Outcome {
        let visible = self.visible();
        let target = self.list.selected().and_then(|s| visible.get(s));
        match target.and_then(|&i| self.rows[i].target.clone()) {
            Some(t) => Outcome::Choose(t),
            None => Outcome::Close,
        }
    }

    pub fn draw(&mut self, f: &mut Frame) {
        let visible = self.visible();
        let area = f.area();
        let widest = visible
            .iter()
            .map(|&i| self.rows[i].line.width())
            .max()
            .unwrap_or(0);
        // At least 34×4 if the screen allows, and never bigger than it.
        // (Not `clamp`: on a tiny screen its minimum would pass its maximum.)
        let max_width = area.width.saturating_sub(4).max(1).min(area.width);
        let max_height = area.height.saturating_sub(2).max(1).min(area.height);
        let width = (widest as u16 + 4).max(34).min(max_width);
        let height = (visible.len() as u16 + 3).max(4).min(max_height);
        let rect = Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + (area.height - height) / 2,
            width,
            height,
        };
        let block = Block::bordered().title(format!(" {} ", self.title));
        let inner = block.inner(rect);
        f.render_widget(Clear, rect);
        f.render_widget(block, rect);

        let list_area = Rect {
            height: inner.height.saturating_sub(1),
            ..inner
        };
        self.page = usize::from(list_area.height);
        let items: Vec<ListItem> = visible
            .iter()
            .map(|&i| ListItem::new(self.rows[i].line.clone()))
            .collect();
        let empty = items.is_empty();
        let list = List::new(items).highlight_style(Style::new().add_modifier(Modifier::REVERSED));
        f.render_stateful_widget(list, list_area, &mut self.list);
        if empty {
            f.render_widget(Paragraph::new(" Nothing matches").dim(), list_area);
        }

        let filter_area = Rect {
            y: inner.y + list_area.height,
            height: 1,
            ..inner
        };
        let prompt = if !self.typing {
            Line::from(" / to filter, ⏎ to go".dim())
        } else {
            Line::from(vec![
                " /".into(),
                Span::raw(self.filter.clone()),
                "▏".slow_blink(),
            ])
        };
        f.render_widget(Paragraph::new(prompt), filter_area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn picker() -> Picker {
        let item = |t: &str, n| Row::item(t.into(), Line::from(t.to_string()), Target::Line(n));
        let rows = vec![
            Row::heading("Docs"),
            item("alpha", 1),
            item("beta", 2),
            Row::heading("Web"),
            item("gamma", 3),
        ];
        Picker::new("T".into(), rows, None)
    }

    fn chosen(p: &mut Picker) -> Option<usize> {
        match p.key(key(KeyCode::Enter), false) {
            Outcome::Choose(Target::Line(n)) => Some(n),
            _ => None,
        }
    }

    #[test]
    fn skips_headings() {
        let mut p = picker();
        assert_eq!(p.list.selected(), Some(1));
        p.key(key(KeyCode::Down), false);
        p.key(key(KeyCode::Down), false);
        assert_eq!(chosen(&mut p), Some(3));
    }

    #[test]
    fn draws_on_any_screen_size() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        for w in 0..45 {
            for h in 0..10 {
                let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
                let mut p = picker();
                term.draw(|f| p.draw(f)).unwrap();
            }
        }
    }

    #[test]
    fn moves_with_j_and_k() {
        let mut p = picker();
        p.key(key(KeyCode::Char('j')), false);
        assert_eq!(p.list.selected(), Some(2));
        p.key(key(KeyCode::Char('j')), false);
        assert_eq!(p.list.selected(), Some(4), "skips the heading row");
        p.key(key(KeyCode::Char('k')), false);
        assert_eq!(p.list.selected(), Some(2));
        p.key(key(KeyCode::Char('G')), false);
        assert_eq!(chosen(&mut p), Some(3));
    }

    #[test]
    fn deleting_past_the_start_stops_filtering() {
        let mut p = picker();
        p.key(key(KeyCode::Char('/')), false);
        p.key(key(KeyCode::Char('x')), false);
        p.key(key(KeyCode::Backspace), false);
        assert!(p.typing, "one character deleted, still filtering");
        p.key(key(KeyCode::Backspace), false);
        assert!(!p.typing);
        p.key(key(KeyCode::Char('j')), false);
        assert_eq!(p.list.selected(), Some(2), "j moves again");
    }

    #[test]
    fn filters_rows() {
        let mut p = picker();
        p.key(key(KeyCode::Char('/')), false);
        p.key(key(KeyCode::Char('g')), false);
        p.key(key(KeyCode::Char('m')), false);
        assert_eq!(p.visible().len(), 1);
        assert_eq!(chosen(&mut p), Some(3));
    }
}
