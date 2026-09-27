//! Settings from `~/.lsmd/config.toml`. Everything is optional, and
//! command-line flags win. For example:
//!
//! ```toml
//! theme = "dark"          # auto, dark or light
//! width = 100             # wrap text at 100 columns (0: the terminal's width)
//! source = true           # start with the source shown beside the text
//! source-side = "left"    # left or right
//! mouse = false           # leave the mouse to the terminal
//! all = true              # list hidden and .gitignored files too
//! sort = "date"           # name or date
//! ```

use crate::doc::SourceSide;
use crate::theme::Mode;
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
    pub theme: Option<Mode>,
    pub width: Option<usize>,
    pub source: Option<bool>,
    pub source_side: Option<SourceSide>,
    pub mouse: Option<bool>,
    pub all: Option<bool>,
    pub sort: Option<Sort>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_setting() {
        let c = parse(
            "theme = \"light\"\nwidth = 100\nsource = true\nsource-side = \"left\"\nmouse = false\nall = true\nsort = \"date\"\n",
        )
        .unwrap();
        assert_eq!(c.theme, Some(Mode::Light));
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
