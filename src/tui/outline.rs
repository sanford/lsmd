//! The outline pane (`O`): the document's headings beside it, with the
//! section on screen highlighted as you scroll.

use super::App;
use super::picker::Target;
use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

impl App {
    /// How wide the pane is beside a document area of `width` columns.
    pub(super) fn outline_width(width: u16) -> u16 {
        // Constant bounds, so clamp is safe here, unlike with a screen size.
        (width / 4).clamp(22, 40).min(width / 2)
    }

    pub(super) fn draw_outline_pane(&mut self, f: &mut Frame, area: Rect) {
        let title = if self.outline_focus.is_some() {
            " Outline ".bold()
        } else {
            " Outline ".dim()
        };
        let block = Block::new()
            .borders(Borders::LEFT)
            .border_style(Style::new().dim())
            .title(title);
        let mut inner = block.inner(area);
        f.render_widget(block, area);
        // A filter being typed goes at the top of the pane it narrows.
        if let Some(filter) = &self.outline_filter {
            let line = Line::from(vec![
                " /".bold(),
                Span::raw(filter.clone()).yellow(),
                "▏".slow_blink(),
            ]);
            f.render_widget(Paragraph::new(line), Rect { height: 1, ..inner });
            inner.y += 1;
            inner.height = inner.height.saturating_sub(1);
        }
        self.outline_area = inner;
        // With the keyboard, the pane shows its own selection; without, the
        // section the document is at.
        let focused = self.outline_focus.map(|(sel, _)| sel);
        let filter = self.outline_filter.clone();
        let Some(doc) = self.current() else { return };
        if doc.headings().is_empty() {
            f.render_widget(Paragraph::new(" No headings").dim(), inner);
            return;
        }
        let current = focused.or(doc.current_heading());
        let visible = visible(doc.headings(), filter.as_deref());
        let items: Vec<ListItem> = visible
            .iter()
            .map(|&i| &doc.headings()[i])
            .map(|h| {
                let indent = "  ".repeat(usize::from(h.level.saturating_sub(1)));
                let text = Span::raw(h.text.clone());
                let text = if h.level <= 2 { text.bold() } else { text };
                ListItem::new(Line::from(vec![Span::raw(format!(" {indent}")), text]))
            })
            .collect();
        self.outline_list
            .select(current.and_then(|c| visible.iter().position(|&i| i == c)));
        if visible.is_empty() {
            f.render_widget(Paragraph::new(" Nothing matches").dim(), inner);
            return;
        }
        let highlight = if focused.is_some() {
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::new().add_modifier(Modifier::BOLD).cyan()
        };
        let list = List::new(items).highlight_style(highlight);
        f.render_stateful_widget(list, inner, &mut self.outline_list);
    }

    /// A click on the outline pane jumps to that heading. Returns whether
    /// the click was on it.
    pub(super) fn outline_click(&mut self, x: u16, y: u16) -> bool {
        if !self.outline_area.contains(Position { x, y }) {
            return false;
        }
        let row = self.outline_list.offset() + usize::from(y - self.outline_area.y);
        let filter = self.outline_filter.clone();
        let line = self.current().and_then(|d| {
            let shown = visible(d.headings(), filter.as_deref());
            shown.get(row).map(|&i| d.headings()[i].line)
        });
        if let Some(line) = line {
            self.go(Target::Line(line));
        }
        true
    }

    /// Gives the outline pane the keyboard, starting at the section on
    /// screen. `O` opens it to stay; `o` (`just_for_now`) only while it has
    /// the keyboard, unless it was already open.
    pub(super) fn focus_outline(&mut self, just_for_now: bool) {
        let Some(doc) = self.current() else { return };
        if doc.headings().is_empty() {
            self.flash = Some("No headings".into());
            return;
        }
        let at = (doc.current_heading().unwrap_or(0), doc.top());
        self.outline_temporary = just_for_now && !self.outline_pane;
        self.outline_pane = true;
        self.outline_focus = Some(at);
        self.outline_filter = None;
    }

    /// Hands the keyboard back to the document, closing the pane if `o`
    /// opened it.
    pub(super) fn leave_outline(&mut self) {
        self.outline_focus = None;
        self.outline_filter = None;
        if self.outline_temporary {
            self.outline_temporary = false;
            self.outline_pane = false;
        }
    }

    /// Keys while the outline has the keyboard. Returns true to quit.
    pub(super) fn outline_key(&mut self, key: KeyEvent) -> bool {
        let Some((sel, from)) = self.outline_focus else {
            return false;
        };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let page = usize::from(self.outline_area.height.max(1));

        // Typing a filter: letters are text, and the arrows still move.
        if let Some(mut filter) = self.outline_filter.take() {
            match key.code {
                // Esc, or deleting past the start, stops filtering.
                KeyCode::Esc => return false,
                KeyCode::Backspace | KeyCode::Delete if filter.is_empty() => return false,
                KeyCode::Backspace | KeyCode::Delete => {
                    filter.pop();
                }
                KeyCode::Char(c) if !ctrl => filter.push(c),
                KeyCode::Enter
                | KeyCode::Up
                | KeyCode::Down
                | KeyCode::PageUp
                | KeyCode::PageDown => {
                    self.outline_filter = Some(filter);
                    return self.outline_move(key.code, shift, page, sel, from);
                }
                _ => {}
            }
            // Select the best match as the filter changes: the first one
            // could be a heading that only matches loosely.
            let best = self.current().and_then(|d| best(d.headings(), &filter));
            self.outline_filter = Some(filter);
            if let Some(best) = best {
                self.outline_select(best, from);
            }
            return false;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => true,
            KeyCode::Char('c') if ctrl => true,
            KeyCode::Char('/') => {
                self.outline_filter = Some(String::new());
                false
            }
            // Never mind: back to where the document was.
            KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left => {
                if let Some(doc) = self.current() {
                    doc.jump_to(from);
                }
                self.leave_outline();
                false
            }
            KeyCode::Char('O') | KeyCode::Char('o') => {
                self.leave_outline();
                self.outline_pane = false;
                false
            }
            code => self.outline_move(code, shift, page, sel, from),
        }
    }

    /// Moves through the outline (as filtered), or reads from the selected
    /// heading with Enter. Returns false: none of this quits.
    fn outline_move(
        &mut self,
        code: KeyCode,
        shift: bool,
        page: usize,
        sel: usize,
        from: usize,
    ) -> bool {
        // Read from here, even when the filter matches nothing.
        if matches!(code, KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right) {
            self.leave_outline();
            return false;
        }
        let filter = self.outline_filter.clone();
        let Some(doc) = self.current() else {
            return false;
        };
        let shown = visible(doc.headings(), filter.as_deref());
        if shown.is_empty() {
            return false;
        }
        let at = shown.iter().position(|&i| i == sel).unwrap_or(0);
        let last = shown.len() - 1;
        let to = match code {
            KeyCode::Down if shift => at.saturating_add(page),
            KeyCode::Up if shift => at.saturating_sub(page),
            KeyCode::Char('J') | KeyCode::PageDown => at.saturating_add(page),
            KeyCode::Char('K') | KeyCode::PageUp => at.saturating_sub(page),
            KeyCode::Char('j') | KeyCode::Down => at + 1,
            KeyCode::Char('k') | KeyCode::Up => at.saturating_sub(1),
            KeyCode::Char('g') | KeyCode::Home => 0,
            KeyCode::Char('G') | KeyCode::End => last,
            _ => return false,
        };
        self.outline_select(shown[to.min(last)], from);
        false
    }

    /// Selects heading `i` in the outline, bringing the document to it.
    fn outline_select(&mut self, i: usize, from: usize) {
        if let Some(doc) = self.current()
            && let Some(line) = doc.headings().get(i).map(|h| h.line)
        {
            doc.jump_to(line);
        }
        self.outline_focus = Some((i, from));
    }
}

/// The headings (as indices) that match `filter`; all of them without one.
fn visible(headings: &[crate::render::Heading], filter: Option<&str>) -> Vec<usize> {
    let Some(filter) = filter.filter(|f| !f.is_empty()) else {
        return (0..headings.len()).collect();
    };
    scores(headings, filter).map(|(i, _)| i).collect()
}

/// The heading that matches `filter` best; the earliest of equals.
fn best(headings: &[crate::render::Heading], filter: &str) -> Option<usize> {
    if filter.is_empty() {
        return None;
    }
    scores(headings, filter)
        .max_by_key(|&(i, score)| (score, std::cmp::Reverse(i)))
        .map(|(i, _)| i)
}

/// Each heading that matches `filter`, with how well.
fn scores<'h>(
    headings: &'h [crate::render::Heading],
    filter: &str,
) -> impl Iterator<Item = (usize, u32)> + 'h {
    // Each word as typed, in any order, rather than fuzzily: a heading's
    // few words make fuzzy matches mostly noise.
    let pattern = Pattern::new(
        filter,
        CaseMatching::Smart,
        Normalization::Smart,
        AtomKind::Substring,
    );
    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut buf = Vec::new();
    headings.iter().enumerate().filter_map(move |(i, h)| {
        let text = Utf32Str::new(&h.text, &mut buf);
        pattern.score(text, &mut matcher).map(|s| (i, s))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::Heading;

    fn headings(texts: &[&str]) -> Vec<Heading> {
        texts
            .iter()
            .enumerate()
            .map(|(line, t)| Heading {
                line,
                level: 2,
                text: t.to_string(),
                slug: String::new(),
            })
            .collect()
    }

    #[test]
    fn filtering_picks_the_best_match_not_the_first() {
        let h = headings(&[
            "Lanternworks Technical Architecture",
            "Threading",
            "Document State Machine",
            "State Diagram",
        ]);
        assert_eq!(visible(&h, Some("state")), vec![2, 3]);
        assert_eq!(best(&h, "sta"), Some(2));
        assert_eq!(best(&h, "diag"), Some(3));
        assert_eq!(best(&h, "diagram state"), Some(3));
        assert_eq!(best(&h, "zzz"), None);
        assert_eq!(visible(&h, Some("")).len(), 4);
    }
}
