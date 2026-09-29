//! Omarchy's themes, bundled so they can be picked anywhere. Each is the
//! theme's `colors.toml`, as Omarchy ships it (MIT: see `themes/LICENSE`).
//!
//! Themes of your own go in `~/.lsmd/themes/`, in the same format: as
//! `NAME.toml`, or as a folder `NAME/` with a `colors.toml` in it, like an
//! Omarchy theme's. One named like a bundled theme takes its place.

use omarchy_theme::Palette;
use std::path::Path;
use std::sync::OnceLock;

macro_rules! themes {
    ($($name:literal),* $(,)?) => {
        const THEMES: &[(&str, &str)] = &[
            $(($name, include_str!(concat!("../themes/", $name, ".toml")))),*
        ];
    };
}

themes!(
    "catppuccin",
    "catppuccin-latte",
    "ethereal",
    "everforest",
    "flexoki-light",
    "gruvbox",
    "hackerman",
    "kanagawa",
    "last-horizon",
    "lumon",
    "lupine",
    "matte-black",
    "miasma",
    "nord",
    "osaka-jade",
    "retro-82",
    "ristretto",
    "rose-pine",
    "solitude",
    "tokyo-night",
    "vantablack",
    "white",
);

/// The user's themes, and what was wrong with those that couldn't be read.
struct Mine {
    themes: Vec<(&'static str, Palette)>,
    errors: Vec<String>,
}

fn mine() -> &'static Mine {
    static MINE: OnceLock<Mine> = OnceLock::new();
    MINE.get_or_init(|| match crate::config::dir() {
        Some(dir) => read_dir(&dir.join("themes")),
        None => Mine {
            themes: Vec::new(),
            errors: Vec::new(),
        },
    })
}

/// The themes in `dir`, by name. Names live as long as lsmd, like the
/// bundled ones: there are only ever a few, read once.
fn read_dir(dir: &Path) -> Mine {
    let mut mine = Mine {
        themes: Vec::new(),
        errors: Vec::new(),
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return mine;
    };
    let mut found: Vec<(String, std::path::PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let name = if path.is_dir() {
                path.file_name()?.to_str()?.to_string()
            } else if path.extension().is_some_and(|x| x == "toml") {
                path.file_stem()?.to_str()?.to_string()
            } else {
                return None;
            };
            let file = if path.is_dir() {
                path.join("colors.toml")
            } else {
                path
            };
            (file.is_file() && !name.starts_with('.')).then_some((name, file))
        })
        .collect();
    found.sort();
    for (name, file) in found {
        if matches!(name.as_str(), "auto" | "dark" | "light")
            || name.chars().any(|c| c.is_control() || c.is_whitespace())
        {
            mine.errors
                .push(format!("{}: can't be a theme's name", file.display()));
            continue;
        }
        match Palette::from_file(&file) {
            Ok(p) => mine.themes.push((Box::leak(name.into_boxed_str()), p)),
            Err(e) => mine.errors.push(format!("{}: {e}", file.display())),
        }
    }
    mine
}

/// What was wrong with the user's themes that couldn't be read.
pub fn errors() -> &'static [String] {
    &mine().errors
}

/// The bundled themes' names, then the user's own.
pub fn names() -> impl Iterator<Item = &'static str> {
    let own = mine().themes.iter().map(|(name, _)| *name);
    THEMES
        .iter()
        .map(|(name, _)| *name)
        .filter(|name| !mine().themes.iter().any(|(n, _)| n == name))
        .chain(own)
}

/// The theme called `name`, by its own spelling.
pub fn find(name: &str) -> Option<&'static str> {
    names().find(|n| *n == name)
}

/// The palette of a theme, from [`find`].
pub fn palette(name: &str) -> Palette {
    if let Some((_, p)) = mine().themes.iter().find(|(n, _)| *n == name) {
        return p.clone();
    }
    let (_, text) = THEMES
        .iter()
        .find(|(n, _)| *n == name)
        .expect("a known theme");
    Palette::parse(text).expect("bundled themes parse")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_theme_parses() {
        for name in names() {
            palette(name);
        }
        assert!(!palette("catppuccin-latte").is_dark());
        assert!(palette("tokyo-night").is_dark());
    }

    #[test]
    fn reads_the_users_themes() {
        let dir = std::env::temp_dir().join(format!("lsmd-themes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("folder")).unwrap();
        let nord = include_str!("../themes/nord.toml");
        std::fs::write(dir.join("mine.toml"), nord).unwrap();
        std::fs::write(dir.join("folder/colors.toml"), nord).unwrap();
        std::fs::write(dir.join("broken.toml"), "background = \"#123\"").unwrap();
        std::fs::write(dir.join("dark.toml"), nord).unwrap();
        std::fs::write(dir.join("notes.txt"), "not a theme").unwrap();
        let mine = read_dir(&dir);
        let names: Vec<_> = mine.themes.iter().map(|(n, _)| *n).collect();
        assert_eq!(names, ["folder", "mine"]);
        assert_eq!(mine.errors.len(), 2, "{:?}", mine.errors);
        assert!(mine.errors[0].contains("broken.toml"));
        assert!(mine.errors[1].contains("dark.toml"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
