//! A popup menu of a few fixed actions, each with a key that does it at
//! once and a line saying what it'll do: copying, and opening links. Unlike
//! the pickers, it isn't filtered by typing: its letters are its keys.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

pub struct Item<A> {
    key: char,
    label: String,
    /// What it'll do, or why it can't.
    detail: String,
    /// `None` when it can't be done here.
    action: Option<A>,
}

impl<A> Item<A> {
    pub fn new(key: char, label: &str, detail: impl Into<String>, action: A) -> Item<A> {
        Item {
            key,
            label: label.into(),
            detail: detail.into(),
            action: Some(action),
        }
    }

    /// One that can't be done here, and `why`.
    pub fn off(key: char, label: &str, why: impl Into<String>) -> Item<A> {
        Item {
            key,
            label: label.into(),
            detail: why.into(),
            action: None,
        }
    }
}

pub struct Menu<A> {
    title: String,
    items: Vec<Item<A>>,
    selected: usize,
    /// Where it was drawn, for the mouse.
    area: Rect,
}

pub enum Outcome<A> {
    Stay,
    Close,
    Choose(A),
}

impl<A: Clone> Menu<A> {
    pub fn new(title: &str, items: Vec<Item<A>>) -> Menu<A> {
        let selected = items.iter().position(|i| i.action.is_some()).unwrap_or(0);
        Menu {
            title: title.into(),
            items,
            selected,
            area: Rect::default(),
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> Outcome<A> {
        match key.code {
            KeyCode::Esc => Outcome::Close,
            KeyCode::Enter => self.choose(self.selected),
            KeyCode::Char(c) if self.items.iter().any(|i| i.key == c) => {
                let i = self.items.iter().position(|i| i.key == c).unwrap_or(0);
                self.choose(i)
            }
            KeyCode::Down | KeyCode::Char('j') => self.step(1),
            KeyCode::Up | KeyCode::Char('k') => self.step(-1),
            _ => Outcome::Stay,
        }
    }

    /// Moves to the next item that can be done, `by` one way or the other.
    fn step(&mut self, by: isize) -> Outcome<A> {
        let n = self.items.len() as isize;
        let mut i = self.selected as isize;
        for _ in 0..n {
            i = (i + by).rem_euclid(n);
            if self.items[i as usize].action.is_some() {
                self.selected = i as usize;
                break;
            }
        }
        Outcome::Stay
    }

    /// Item `i`, if it can be done; otherwise the menu stays, showing why.
    fn choose(&mut self, i: usize) -> Outcome<A> {
        match self.items.get(i).and_then(|item| item.action.clone()) {
            Some(action) => Outcome::Choose(action),
            None => Outcome::Stay,
        }
    }

    /// A click: on an item, does it; anywhere else, closes the menu.
    pub fn click(&mut self, x: u16, y: u16) -> Outcome<A> {
        let inner = self.area.inner(ratatui::layout::Margin::new(1, 1));
        if !inner.contains(Position { x, y }) {
            return if self.area.contains(Position { x, y }) {
                Outcome::Stay
            } else {
                Outcome::Close
            };
        }
        self.choose(usize::from(y - inner.y))
    }

    pub fn draw(&mut self, f: &mut Frame) {
        let screen = f.area();
        let label_w = self
            .items
            .iter()
            .map(|i| i.label.width())
            .max()
            .unwrap_or(0);
        let detail_w = self
            .items
            .iter()
            .map(|i| i.detail.width())
            .max()
            .unwrap_or(0);
        // Key, label and detail, with room between and the border around.
        let want = 1 + 1 + 2 + label_w + 3 + detail_w + 1 + 2;
        let width = (want as u16)
            .min(screen.width.saturating_sub(4))
            .max(20.min(screen.width));
        let height = (self.items.len() as u16 + 2).min(screen.height);
        let rect = Rect {
            x: screen.x + (screen.width - width) / 2,
            y: screen.y + (screen.height - height) / 2,
            width,
            height,
        };
        self.area = rect;
        let block = Block::bordered().title(format!(" {} ", self.title));
        let inner = block.inner(rect);
        f.render_widget(Clear, rect);
        f.render_widget(block, rect);
        let room = usize::from(inner.width).saturating_sub(1 + 1 + 2 + label_w + 3 + 1);
        let lines: Vec<Line> = self
            .items
            .iter()
            .enumerate()
            .map(|(n, item)| {
                let detail = crate::safe::printable(&item.detail);
                let mut spans = vec![
                    Span::raw(" "),
                    Span::raw(item.key.to_string()).bold(),
                    Span::raw(format!("  {:<label_w$}   ", item.label)),
                    Span::raw(fit(&detail, room)).dim(),
                    Span::raw(" "),
                ];
                if item.action.is_none() {
                    for s in &mut spans {
                        s.style = s.style.add_modifier(Modifier::DIM);
                    }
                }
                let line = Line::from(spans);
                if n == self.selected {
                    line.style(Style::new().add_modifier(Modifier::REVERSED))
                } else {
                    line
                }
            })
            .collect();
        f.render_widget(Paragraph::new(lines), inner);
    }
}

/// `s` in at most `room` columns, cut short with "…" if it has to be.
fn fit(s: &str, room: usize) -> String {
    if s.width() <= room {
        return s.to_string();
    }
    let mut out = String::new();
    for c in s.chars() {
        if out.width() + c.to_string().width() + 1 > room {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn menu() -> Menu<u8> {
        Menu::new(
            "Copy",
            vec![
                Item::off('c', "Code", "none on screen"),
                Item::new('s', "Section", "\"Intro\"", 1),
                Item::new('p', "Path", "a.md", 2),
                Item::new('P', "Full path", "/x/a.md", 3),
            ],
        )
    }

    #[test]
    fn letters_do_their_item_at_once() {
        let mut m = menu();
        assert!(matches!(m.key(key(KeyCode::Char('P'))), Outcome::Choose(3)));
        assert!(matches!(m.key(key(KeyCode::Char('p'))), Outcome::Choose(2)));
        // One that can't be done stays, showing why.
        assert!(matches!(m.key(key(KeyCode::Char('c'))), Outcome::Stay));
        assert!(matches!(m.key(key(KeyCode::Char('z'))), Outcome::Stay));
        assert!(matches!(m.key(key(KeyCode::Esc)), Outcome::Close));
    }

    #[test]
    fn arrows_skip_what_cant_be_done() {
        let mut m = menu();
        assert_eq!(m.selected, 1, "starts on the first that can be done");
        m.key(key(KeyCode::Up));
        assert_eq!(m.selected, 3, "round past the one that can't");
        m.key(key(KeyCode::Down));
        assert!(matches!(m.key(key(KeyCode::Enter)), Outcome::Choose(1)));
    }

    #[test]
    fn fits_details_in_the_room() {
        assert_eq!(fit("short", 10), "short");
        assert_eq!(fit("/a/very/long/path.md", 8), "/a/very…");
    }
}
