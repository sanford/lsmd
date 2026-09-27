//! The mouse: the wheel scrolls what's under the pointer, and clicks
//! choose files and follow links.

use super::nav::Prompt;
use super::{App, Focus};
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Position;

/// Lines per wheel notch.
const WHEEL: isize = 3;

impl App<'_> {
    pub(super) fn mouse(&mut self, m: MouseEvent) {
        let (x, y) = (m.column, m.row);
        let down = match m.kind {
            MouseEventKind::ScrollDown => Some(true),
            MouseEventKind::ScrollUp => Some(false),
            _ => None,
        };
        // A popup takes the wheel for its list; clicks outside close it.
        if let Some(Prompt::Pick(picker)) = &mut self.prompt {
            if let Some(down) = down {
                let code = if down { KeyCode::Down } else { KeyCode::Up };
                picker.key(KeyEvent::new(code, KeyModifiers::NONE), false);
            }
            return;
        }
        // The scrollbar and the outline pane take clicks and drags.
        if matches!(
            m.kind,
            MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Drag(MouseButton::Left)
        ) {
            if let Some(doc) = self.current()
                && doc.scrollbar_jump(x, y)
            {
                return;
            }
            if m.kind == MouseEventKind::Down(MouseButton::Left) && self.outline_click(x, y) {
                return;
            }
        }
        let over_list = self.list_area.contains(Position { x, y });
        match m.kind {
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp if over_list => {
                self.select_by(if down == Some(true) { 1 } else { -1 });
            }
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let delta = if down == Some(true) { WHEEL } else { -WHEEL };
                let sideways = m.modifiers.contains(KeyModifiers::SHIFT);
                if let Some(doc) = self.current()
                    && let Some(side) = doc.side_at(x, y)
                {
                    if sideways {
                        doc.scroll_sideways(delta * 2);
                    } else {
                        doc.scroll_by(delta, side);
                    }
                }
            }
            MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight => {
                let delta = if m.kind == MouseEventKind::ScrollRight {
                    6
                } else {
                    -6
                };
                if let Some(doc) = self.current() {
                    doc.scroll_sideways(delta);
                }
            }
            MouseEventKind::Down(MouseButton::Left) if over_list => self.click_list(y),
            MouseEventKind::Down(MouseButton::Left) => {
                let url = self.current().and_then(|d| d.link_at(x, y));
                if let Some(url) = url {
                    if self.focus == Focus::List {
                        self.reading = self.selected().map(|e| e.path.clone());
                        self.focus = Focus::Reader;
                    }
                    self.follow(&url);
                }
            }
            _ => {}
        }
    }

    /// A click on row `y` of the file list selects that file, or opens it
    /// if it was already selected.
    fn click_list(&mut self, y: u16) {
        let row = usize::from(y.saturating_sub(self.list_area.y));
        let i = self.list.offset() + row;
        if i >= self.shown.len() {
            return;
        }
        if self.list.selected() == Some(i) {
            if let Some(e) = self.selected() {
                self.reading = Some(e.path.clone());
                self.focus = Focus::Reader;
                self.history.clear();
            }
        } else {
            self.moved = true;
            self.list.select(Some(i));
            self.focus = Focus::List;
        }
    }
}
