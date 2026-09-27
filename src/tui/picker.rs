//! A popup list to choose from, filtered by typing: the outline and the
//! links panel.

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph};
use std::path::PathBuf;

/// What choosing a row does.
#[derive(Clone, Debug)]
pub enum Target {
    /// Jump to this rendered line of the current document.
    Line(usize),
    /// Follow a link as written in the current document.
    Link(String),
    /// Open this Markdown file.
    File(PathBuf),
    /// Open this file at a search match: source line and query.
    Match(PathBuf, usize, String),
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
}

pub struct Picker {
    title: String,
    rows: Vec<Row>,
    filter: String,
    /// Selection among the visible rows.
    list: ListState,
}

impl Picker {
    /// `selected` is an index into `rows`.
    pub fn new(title: String, rows: Vec<Row>, selected: Option<usize>) -> Picker {
        let mut picker = Picker {
            title,
            rows,
            filter: String::new(),
            list: ListState::default(),
        };
        match selected {
            Some(i) => picker.list.select(Some(i)),
            None => picker.select_from(0, 1),
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
    /// looking in direction `step`. Keeps the selection if there's none.
    fn select_from(&mut self, from: isize, step: isize) {
        let visible = self.visible();
        let mut i = from;
        while i >= 0 && (i as usize) < visible.len() {
            if self.rows[visible[i as usize]].target.is_some() {
                self.list.select(Some(i as usize));
                return;
            }
            i += step;
        }
        if self.list.selected().is_none_or(|s| s >= visible.len()) {
            self.list.select(None);
        }
    }

    pub fn key(&mut self, key: KeyEvent, ctrl: bool) -> Outcome {
        let at = self.list.selected().map_or(-1, |s| s as isize);
        match key.code {
            KeyCode::Esc => return Outcome::Close,
            KeyCode::Enter => {
                let visible = self.visible();
                let target = self.list.selected().and_then(|s| visible.get(s));
                return match target.and_then(|&i| self.rows[i].target.clone()) {
                    Some(t) => Outcome::Choose(t),
                    None => Outcome::Close,
                };
            }
            KeyCode::Down => self.select_from(at + 1, 1),
            KeyCode::Up => self.select_from(at - 1, -1),
            KeyCode::Char('n') if ctrl => self.select_from(at + 1, 1),
            KeyCode::Char('p') if ctrl => self.select_from(at - 1, -1),
            KeyCode::Backspace => {
                self.filter.pop();
                self.select_from(0, 1);
            }
            KeyCode::Char(c) if !ctrl => {
                self.filter.push(c);
                self.select_from(0, 1);
            }
            _ => {}
        }
        Outcome::Stay
    }

    pub fn draw(&mut self, f: &mut Frame) {
        let visible = self.visible();
        let area = f.area();
        let widest = visible
            .iter()
            .map(|&i| self.rows[i].line.width())
            .max()
            .unwrap_or(0);
        let width = (widest as u16 + 4)
            .clamp(34, area.width.saturating_sub(4).max(1))
            .min(area.width);
        let height = (visible.len() as u16 + 3)
            .clamp(4, area.height.saturating_sub(2).max(1))
            .min(area.height);
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
        let prompt = if self.filter.is_empty() {
            Line::from(" type to filter, ⏎ to go".dim())
        } else {
            Line::from(vec![
                " ".into(),
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
    fn filters_rows() {
        let mut p = picker();
        p.key(key(KeyCode::Char('g')), false);
        p.key(key(KeyCode::Char('m')), false);
        assert_eq!(p.visible().len(), 1);
        assert_eq!(chosen(&mut p), Some(3));
    }
}
