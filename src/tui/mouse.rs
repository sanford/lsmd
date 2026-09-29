//! The mouse: the wheel scrolls what's under the pointer, clicks choose
//! files and follow links, and dragging over the text copies what it
//! covers, as Markdown.

use super::nav::Prompt;
use super::{App, Focus};
use crate::doc::Side;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Position;

/// Lines per wheel notch.
const WHEEL: isize = 3;

impl App {
    pub(super) fn mouse(&mut self, m: MouseEvent) {
        let (x, y) = (m.column, m.row);
        let down = match m.kind {
            MouseEventKind::ScrollDown => Some(true),
            MouseEventKind::ScrollUp => Some(false),
            _ => None,
        };
        // A menu takes clicks: on an item, it does it; outside, it closes.
        if let Some(Prompt::Menu(menu)) = &mut self.prompt {
            if m.kind == MouseEventKind::Down(MouseButton::Left) {
                match menu.click(x, y) {
                    super::menu::Outcome::Stay => {}
                    super::menu::Outcome::Close => self.prompt = None,
                    super::menu::Outcome::Choose(action) => {
                        self.prompt = None;
                        self.act(action);
                    }
                }
            }
            return;
        }
        // A popup takes the wheel for its list; clicks outside close it.
        if let Some(Prompt::Pick(picker)) = &mut self.prompt {
            if let Some(down) = down {
                let code = if down { KeyCode::Down } else { KeyCode::Up };
                picker.key(KeyEvent::new(code, KeyModifiers::NONE), false);
            }
            return;
        }
        if self.drag_mouse(m) {
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
                if let Some(doc) = self.current()
                    && let Some(side) = doc.side_at(x, y)
                {
                    doc.scroll_by(delta, side);
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

    /// Dragging over the rendered text selects the pieces it covers, and
    /// letting go copies them. Returns whether it took the event.
    fn drag_mouse(&mut self, m: MouseEvent) -> bool {
        let y = m.row;
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // A click ends a selection made with the keyboard.
                if matches!(self.prompt, Some(Prompt::Select { .. })) {
                    self.prompt = None;
                    self.end_select();
                }
                let on_text = self
                    .current()
                    .is_some_and(|d| d.side_at(m.column, y) == Some(Side::Rendered));
                self.drag_from = on_text
                    .then(|| self.current().and_then(|d| d.piece_at(y)))
                    .flatten();
                false
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                let Some(anchor) = self.drag_from else {
                    return false;
                };
                let cursor = match &self.prompt {
                    Some(Prompt::Select { cursor, .. }) => *cursor,
                    _ => anchor,
                };
                let cursor = self.current().and_then(|d| d.piece_at(y)).unwrap_or(cursor);
                self.select(anchor, cursor, true);
                true
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.drag_from = None;
                let Some(Prompt::Select {
                    anchor,
                    cursor,
                    drag: true,
                }) = self.prompt
                else {
                    return false;
                };
                self.prompt = None;
                self.copy_selection(anchor, cursor);
                true
            }
            _ => false,
        }
    }

    /// A click on row `y` of the file list selects that row, or if it was
    /// already selected, opens the file or opens or closes the folder.
    fn click_list(&mut self, y: u16) {
        let row = usize::from(y.saturating_sub(self.list_area.y));
        let i = self.list.offset() + row;
        if i >= self.shown.len() || matches!(self.shown[i], super::Shown::Rule(_)) {
            return;
        }
        if self.list.selected() == Some(i) {
            if self.on_up() {
                self.leave_dir();
            } else if self.selected_dir().is_some() {
                self.toggle_dir();
            } else if let Some(e) = self.selected() {
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
