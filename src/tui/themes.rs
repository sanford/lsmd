//! The theme picker: moving through it shows each theme, Enter keeps one
//! (saving it to the config file), and Esc goes back to what was there.

use super::App;
use super::nav::Prompt;
use super::picker::{Picker, Row, Target};
use crate::palettes;
use crate::theme::{Choice, Mode, Theme};
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use std::rc::Rc;

impl App {
    pub(super) fn open_themes(&mut self) {
        if self.omarchy {
            self.flash = Some("Colors follow the Omarchy theme".into());
            return;
        }
        if !self.theme.color {
            self.flash = Some("Colors are off".into());
            return;
        }
        let mut rows = vec![Row::heading("Terminal's colors")];
        for (mode, what) in [
            (Mode::Auto, "dark or light code, to suit the terminal"),
            (Mode::Dark, "dark code"),
            (Mode::Light, "light code"),
        ] {
            let choice = Choice::Mode(mode);
            let line = Line::from(vec![
                Span::raw(format!(" {:<18}", choice.name())),
                Span::raw(what).dim(),
            ]);
            rows.push(Row::item(choice.name().into(), line, Target::Theme(choice)));
        }
        rows.push(Row::heading("Themes"));
        for name in palettes::names() {
            rows.push(Row::item(
                name.into(),
                self.swatch(name),
                Target::Theme(Choice::Named(name)),
            ));
        }
        let selected = rows
            .iter()
            .position(|r| r.target() == Some(&Target::Theme(self.choice)));
        self.theme_before = Some((Rc::clone(&self.theme), self.choice));
        self.prompt = Some(Prompt::Pick(Picker::new("Theme".into(), rows, selected)));
    }

    /// A theme's name and a strip of its colors.
    fn swatch(&self, name: &'static str) -> Line<'static> {
        let p = palettes::palette(name);
        let mut spans = vec![Span::raw(format!(" {name:<18}"))];
        for c in [
            p.background(),
            p.red(),
            p.yellow(),
            p.green(),
            p.cyan(),
            p.blue(),
            p.magenta(),
            p.foreground(),
        ] {
            // Exact colors, which a painted theme leaves alone, and the
            // same both ways, so the selected row's reversing leaves them too.
            let c = self.theme.rgb(c.r, c.g, c.b);
            spans.push(Span::raw("██").fg(c).bg(c));
        }
        if !p.is_dark() {
            spans.push(Span::raw("  light").dim());
        }
        Line::from(spans)
    }

    /// While the theme picker is open, shows the theme selected in it; once
    /// it's closed without choosing, goes back to the one from before.
    pub(super) fn preview_theme(&mut self) {
        if self.theme_before.is_none() {
            return;
        }
        let selected = match &self.prompt {
            Some(Prompt::Pick(picker)) => match picker.selected() {
                Some(Target::Theme(choice)) => *choice,
                _ => return,
            },
            _ => {
                let (theme, choice) = self.theme_before.take().unwrap();
                self.choice = choice;
                self.set_theme(theme);
                return;
            }
        };
        if selected != self.choice {
            self.choice = selected;
            let theme = Theme::chosen(self.resolve(selected), true);
            self.set_theme(Rc::new(theme));
        }
    }

    /// `auto` as dark or light: the terminal can't be asked mid-screen.
    fn resolve(&self, choice: Choice) -> Choice {
        match choice {
            Choice::Mode(Mode::Auto) if self.terminal_dark => Choice::Mode(Mode::Dark),
            Choice::Mode(Mode::Auto) => Choice::Mode(Mode::Light),
            other => other,
        }
    }

    /// Keeps the theme chosen in the picker, and saves it for next time.
    pub(super) fn keep_theme(&mut self, choice: Choice) {
        self.theme_before = None;
        if choice != self.choice {
            self.choice = choice;
            let theme = Theme::chosen(self.resolve(choice), true);
            self.set_theme(Rc::new(theme));
        }
        self.flash = Some(match crate::config::save_theme(choice.name()) {
            Ok(path) => format!(
                "Theme {} saved in {}",
                choice.name(),
                super::display_path(&path)
            ),
            Err(e) => format!("Theme {}, but couldn't save it: {e}", choice.name()),
        });
    }
}
