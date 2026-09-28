mod ansi;
mod clipboard;
mod config;
mod doc;
mod editor;
mod files;
mod grep;
mod highlight;
mod html;
mod index;
mod omarchy;
mod open;
mod palettes;
mod render;
mod safe;
mod theme;
mod tui;
mod watch;
mod wrap;

use clap::Parser;
use doc::SourceSide;
use std::io::{self, ErrorKind, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use theme::{Choice, Mode, Theme};

/// Browse and read Markdown in the terminal.
///
/// With no arguments, lists the Markdown files under the current directory,
/// with a preview of each. With a file, opens it in the reader. When output
/// isn't a terminal, prints the rendered file, or the list of files.
///
/// Defaults for the options can go in ~/.lsmd/config.toml, e.g. `theme =
/// "dark"`, `width = 100`, `source-side = "left"`, `mouse = false`.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// Markdown file to read, directory to browse, or "-" for standard input
    path: Option<PathBuf>,

    /// Include hidden files and files ignored by .gitignore
    #[arg(short, long)]
    all: bool,

    /// Wrap text at N columns instead of the terminal's width
    #[arg(short, long, value_name = "N")]
    width: Option<usize>,

    /// Start with the source shown beside the rendered text
    #[arg(short, long)]
    source: bool,

    /// Which side of the split view the source goes on
    #[arg(long, value_enum, value_name = "SIDE")]
    source_side: Option<SourceSide>,

    /// Don't use the mouse, so the terminal's own text selection works
    #[arg(long)]
    no_mouse: bool,

    /// Print without colors or styles
    #[arg(short, long)]
    plain: bool,

    /// Color theme: auto, dark, light, or one of Omarchy's themes, like
    /// tokyo-night, which colors everything
    #[arg(long)]
    theme: Option<Choice>,
}

fn main() -> ExitCode {
    let args = Args::parse();
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e.kind() == ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("lsmd: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Args) -> io::Result<()> {
    let no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
    let interactive = io::stdout().is_terminal() && !args.plain;
    // Flags win over the config file.
    let config = config::load();
    let choice = args
        .theme
        .or(config.theme)
        .unwrap_or(Choice::Mode(Mode::Auto));
    let color = !args.plain && !no_color;
    // On Omarchy the desktop's theme wins, unless code is asked to be
    // plain dark or light.
    let palette = (color && !matches!(choice, Choice::Mode(Mode::Dark | Mode::Light)))
        .then(omarchy::palette)
        .flatten();
    let theme = match &palette {
        Some(p) => Theme::new(Mode::Auto, color, Some(p)),
        None => Theme::chosen(choice, color),
    };
    let width = args.width.or(config.width).filter(|&w| w > 0);
    let all = args.all || config.all.unwrap_or(false);

    let source = source(args.path.as_deref(), all)?;
    if interactive {
        let settings = tui::Settings {
            max_width: width,
            split: args.source || config.source.unwrap_or(false),
            source_side: args
                .source_side
                .or(config.source_side)
                .unwrap_or(SourceSide::Right),
            mouse: !args.no_mouse && config.mouse.unwrap_or(true),
            by_date: config.sort == Some(config::Sort::Date),
            outline: config.outline.unwrap_or(false),
            choice,
            omarchy: palette.is_some(),
        };
        return tui::run(source, theme, settings);
    }
    // Relative links are checked against the document's directory.
    let (md, base) = match source {
        tui::Source::Text { md, .. } => (md, std::env::current_dir().ok()),
        tui::Source::Browse {
            open: Some(path), ..
        } => (read(&path)?, path.parent().map(Path::to_path_buf)),
        tui::Source::Browse {
            root,
            open: None,
            all,
        } => return list_files(&root, all),
    };
    let base = base.as_deref();
    let mut lines = render::render(
        &md,
        width.unwrap_or_else(terminal_width),
        &theme,
        base,
        base.map(files::site_root).as_deref(),
    )
    .lines;
    for span in lines.iter_mut().flat_map(|l| &mut l.spans) {
        span.style = theme.recolor(span.style);
    }
    ansi::print(&lines, &mut io::stdout().lock())
}

/// Works out what to show from the command line.
fn source(path: Option<&Path>, all: bool) -> io::Result<tui::Source> {
    let stdin = || -> io::Result<tui::Source> {
        let mut bytes = Vec::new();
        io::stdin().read_to_end(&mut bytes)?;
        let md = String::from_utf8_lossy(&bytes).into_owned();
        Ok(tui::Source::Text {
            title: "stdin".into(),
            md,
        })
    };
    let cwd = std::fs::canonicalize(std::env::current_dir()?)?;
    let path = match path {
        Some(p) if p.as_os_str() == "-" => return stdin(),
        Some(p) => std::fs::canonicalize(p)
            .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", p.display())))?,
        None if !io::stdin().is_terminal() => return stdin(),
        None => cwd.clone(),
    };
    if path.is_dir() {
        return Ok(tui::Source::Browse {
            root: path,
            open: None,
            all,
        });
    }
    // Browse from the current directory if the file is somewhere under it,
    // otherwise from the file's own directory.
    let root = if path.starts_with(&cwd) {
        cwd
    } else {
        path.parent().map_or(cwd, Path::to_path_buf)
    };
    Ok(tui::Source::Browse {
        root,
        open: Some(path),
        all,
    })
}

fn read(path: &Path) -> io::Result<String> {
    let bytes = std::fs::read(path)
        .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Prints the files a browser would list, one per line, like `ls`.
fn list_files(root: &Path, all: bool) -> io::Result<()> {
    let mut entries: Vec<files::Entry> = files::scan(root, all).into_iter().flatten().collect();
    entries.sort_by(files::by_path);
    let mut out = io::stdout().lock();
    for e in entries {
        writeln!(out, "{}", safe::printable(&e.rel))?;
    }
    out.flush()
}

/// Width for printed output: $COLUMNS, else the terminal's, else 80.
fn terminal_width() -> usize {
    std::env::var("COLUMNS")
        .ok()
        .and_then(|c| c.parse().ok())
        .filter(|&w| w > 0)
        .or_else(|| {
            ratatui::crossterm::terminal::size()
                .ok()
                .map(|(w, _)| w.into())
        })
        .filter(|&w| w > 0)
        .unwrap_or(80)
}
