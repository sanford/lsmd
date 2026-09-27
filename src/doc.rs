//! An open document: its text, its rendered lines and where it's scrolled to.

use crate::render::{RLine, render};
use crate::theme::Theme;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use std::path::Path;
use std::time::SystemTime;

pub struct Doc {
    md: String,
    /// When the file was last modified, as of loading it.
    pub modified: Option<SystemTime>,
    lines: Vec<RLine>,
    /// The width `lines` was wrapped to (0 before the first layout).
    width: usize,
    /// Index of the first visible line.
    top: usize,
    /// Visible lines at the last draw.
    height: usize,
}

impl Doc {
    pub fn new(md: String) -> Doc {
        Doc {
            md,
            modified: None,
            lines: Vec::new(),
            width: 0,
            top: 0,
            height: 0,
        }
    }

    /// Reads a file. Errors become the document's text, so they show where
    /// the document would have.
    pub fn load(path: &Path) -> Doc {
        let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        let md = match std::fs::read(path) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(e) => format!("> [!CAUTION]\n> Couldn't read {}: {e}\n", path.display()),
        };
        Doc {
            modified,
            ..Doc::new(md)
        }
    }

    /// Wraps the document for `width`, keeping the same part of it at the
    /// top of the screen as before.
    fn layout(&mut self, width: usize, theme: &Theme) {
        if width == self.width {
            return;
        }
        let anchor = self.lines.get(self.top..).and_then(|l| l.iter().find_map(|l| l.src));
        self.lines = render(&self.md, width, theme);
        self.width = width;
        self.top = match anchor {
            Some((first, _)) => self
                .lines
                .iter()
                .position(|l| l.src.is_some_and(|(f, _)| f >= first))
                .unwrap_or(0),
            None => 0,
        };
    }

    /// Draws the visible lines into `area`, wrapped to `width` (which may be
    /// less than the area's).
    pub fn draw(&mut self, f: &mut Frame, area: Rect, width: usize, theme: &Theme) {
        self.layout(width.max(1), theme);
        self.height = area.height.into();
        self.top = self.top.min(self.max_top());
        let visible: Vec<Line> = self.lines[self.top..]
            .iter()
            .take(self.height)
            .map(|l| Line::from(l.spans.clone()))
            .collect();
        f.render_widget(Paragraph::new(visible), area);
    }

    fn max_top(&self) -> usize {
        self.lines.len().saturating_sub(self.height)
    }

    pub fn scroll_by(&mut self, delta: isize) {
        self.top = self.top.saturating_add_signed(delta).min(self.max_top());
    }

    pub fn page(&self) -> isize {
        self.height.max(1) as isize
    }

    pub fn scroll_to_top(&mut self) {
        self.top = 0;
    }

    pub fn scroll_to_bottom(&mut self) {
        self.top = self.max_top();
    }

    /// "Top", "Bot", "All" or a percentage, like less and vim.
    pub fn position(&self) -> String {
        if self.lines.len() <= self.height {
            "All".into()
        } else if self.top == 0 {
            "Top".into()
        } else if self.top >= self.max_top() {
            "Bot".into()
        } else {
            let seen = self.top + self.height;
            format!("{}%", seen * 100 / self.lines.len())
        }
    }
}
