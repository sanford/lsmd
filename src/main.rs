mod ansi;
mod clipboard;
mod doc;
mod editor;
mod files;
mod highlight;
mod html;
mod index;
mod render;
mod theme;
mod tui;
mod watch;
mod wrap;

use clap::Parser;
use doc::SourceSide;
use std::io::{self, ErrorKind, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use theme::{Mode, Theme};

/// Browse and read Markdown in the terminal.
///
/// With no arguments, lists the Markdown files under the current directory,
/// with a preview of each. With a file, opens it in the reader. When output
/// isn't a terminal, prints the rendered file, or the list of files.
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
    #[arg(long, value_enum, value_name = "SIDE", default_value_t = SourceSide::Right)]
    source_side: SourceSide,

    /// Print without colors or styles
    #[arg(short, long)]
    plain: bool,

    /// Color theme
    #[arg(long, value_enum, default_value_t = Mode::Auto)]
    theme: Mode,
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
    let theme = Theme::new(args.theme, !args.plain && !no_color);
    let width = args.width.filter(|&w| w > 0);

    let source = source(args.path.as_deref(), args.all)?;
    if interactive {
        let settings = tui::Settings {
            max_width: width,
            split: args.source,
            source_side: args.source_side,
        };
        return tui::run(source, &theme, settings);
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
    let lines = render::render(
        &md,
        width.unwrap_or_else(terminal_width),
        &theme,
        true,
        base,
    )
    .lines;
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
        writeln!(out, "{}", e.rel)?;
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
