//! Settings from `~/.lsmd/config.toml`. Everything is optional, and
//! command-line flags win. For example:
//!
//! ```toml
//! theme = "dark"          # auto, dark, light or a theme's name
//! width = 100             # wrap text at 100 columns (0: the terminal's width)
//! source = true           # start with the source shown beside the text
//! source-side = "left"    # left or right
//! mouse = false           # leave the mouse to the terminal
//! all = true              # list hidden and .gitignored files too
//! sort = "date"           # name or date
//! outline = true          # show the outline pane beside documents
//! ```

use crate::doc::SourceSide;
use crate::theme::Choice;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sort {
    Name,
    Date,
}

#[derive(Default, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Config {
    pub theme: Option<Choice>,
    pub width: Option<usize>,
    pub source: Option<bool>,
    pub source_side: Option<SourceSide>,
    pub mouse: Option<bool>,
    pub all: Option<bool>,
    pub sort: Option<Sort>,
    pub outline: Option<bool>,
}

pub fn path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".lsmd").join("config.toml"))
}

/// Reads the config file. A missing file is fine; a broken one is reported
/// and otherwise ignored, so lsmd still starts.
pub fn load() -> Config {
    let Some(path) = path() else {
        return Config::default();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Config::default();
    };
    parse(&text).unwrap_or_else(|e| {
        eprintln!("lsmd: ignoring {}: {e}", path.display());
        Config::default()
    })
}

fn parse(text: &str) -> Result<Config, String> {
    toml::from_str(text).map_err(|e| e.message().to_string())
}

/// Saves the theme to the config file, keeping everything else in it as it
/// was. Returns the file's path.
pub fn save_theme(theme: &str) -> std::io::Result<PathBuf> {
    let path = path().ok_or_else(|| std::io::Error::other("no home directory"))?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Written beside it and renamed over it, so it's never half-written.
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, with_theme(&text, theme))?;
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

/// `text` with its `theme =` line set to `theme`, or one added.
fn with_theme(text: &str, theme: &str) -> String {
    let line = format!("theme = \"{theme}\"");
    let is_theme = |l: &str| {
        l.trim_start()
            .strip_prefix("theme")
            .is_some_and(|rest| rest.trim_start().starts_with('='))
    };
    let mut out: Vec<String> = Vec::new();
    let mut found = false;
    for l in text.lines() {
        if is_theme(l) {
            if !found {
                out.push(line.clone());
            }
            found = true;
        } else {
            out.push(l.to_string());
        }
    }
    if !found {
        // At the top: the config has no tables, but keys must come before
        // any that it someday has.
        out.insert(0, line);
    }
    out.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_the_theme_keeping_the_rest() {
        assert_eq!(with_theme("", "nord"), "theme = \"nord\"\n");
        assert_eq!(
            with_theme("width = 80 # narrow\n", "nord"),
            "theme = \"nord\"\nwidth = 80 # narrow\n"
        );
        assert_eq!(
            with_theme("# mine\ntheme = \"dark\"   # was\nwidth = 80", "gruvbox"),
            "# mine\ntheme = \"gruvbox\"\nwidth = 80\n"
        );
        assert_eq!(
            with_theme("themes = 1", "nord"),
            "theme = \"nord\"\nthemes = 1\n",
            "only the theme line"
        );
    }

    #[test]
    fn parses_every_setting() {
        let c = parse(
            "theme = \"light\"\nwidth = 100\nsource = true\nsource-side = \"left\"\nmouse = false\nall = true\nsort = \"date\"\n",
        )
        .unwrap();
        assert_eq!(c.theme, Some(Choice::Mode(crate::theme::Mode::Light)));
        assert_eq!(c.width, Some(100));
        assert_eq!(c.source, Some(true));
        assert_eq!(c.source_side, Some(SourceSide::Left));
        assert_eq!(c.mouse, Some(false));
        assert_eq!(c.all, Some(true));
        assert_eq!(c.sort, Some(Sort::Date));
    }

    #[test]
    fn empty_is_all_defaults() {
        let c = parse("").unwrap();
        assert!(c.theme.is_none() && c.mouse.is_none());
    }

    #[test]
    fn rejects_typos() {
        let e = parse("thme = \"dark\"").unwrap_err();
        assert!(e.contains("thme"), "{e}");
        assert!(parse("theme = \"blue\"").is_err());
    }
}
