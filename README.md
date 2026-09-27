# lsmd

**Browse and read Markdown in the terminal.** `lsmd` lists the Markdown files under a directory with a live preview, renders them the full width of your terminal, and shows the source side by side when you press `Tab`.

```
$ lsmd RELEASING.md        # then Tab
 lsmd  ~/dev/lsnet/RELEASING.md                                                  linked from 1  L links
  Releasing lsnet                                  │  1 # Releasing lsnet
  ═══════════════                                  │  2
                                                   │  3 A release is a version tag on this repo, a
  A release is a version tag on this repo, a       │    formula update in the Homebrew tap
  formula update in the Homebrew tap               │    ([sanford/homebrew-tap](https://github.com/sanf
  (sanford/homebrew-tap                            │    ord/homebrew-tap)), and a GitHub Release with
  (https://github.com/sanford/homebrew-tap)), and  │    notes and a Windows binary. Homebrew builds
  a GitHub Release with notes and a Windows        │    from the source tarball GitHub serves for
  binary. Homebrew builds from the source tarball  │    each tag, so macOS and Linux need no
  GitHub serves for each tag, so macOS and Linux   │    binaries. Windows has no Homebrew, so its
  need no binaries. Windows has no Homebrew, so    │    `lsnet.exe` is built from the tag and
  its lsnet.exe is built from the tag and attached │    attached to the release.
  to the release.                                  │  4
                                                   │  5 The examples below release `0.2.0` after
  The examples below release 0.2.0 after 0.1.0.    │    `0.1.0`. Substitute the real versions.
  Substitute the real versions.                    │  6
                                                   │  7 ## 1. Prepare
  1. Prepare                                       │  8
 ↑↓ scroll  / search  f follow  o outline  tab hide source  ^w to source  esc list  ? help  q quit  Top
```

It's in the spirit of [glow](https://github.com/charmbracelet/glow), for people who keep a lot of Markdown around: a dense file list, the whole terminal for reading, and ways to get around a set of documents (search, outlines, links in and out).

## Features

- **Browse and preview.** One line per file, with a live preview of the selected one. Fuzzy-filter names with `/`; search the text of every file with `s`.
- **Full width.** Text wraps to your terminal, not to 80 columns, and re-wraps when you resize.
- **Source side by side.** `Tab` shows the Markdown beside the rendered text, scrolled together block by block, so a 3-line table that renders as 10 lines stays lined up.
- **Renders what GitHub renders.** Tables, task lists, footnotes, `> [!NOTE]` alerts, front matter, syntax-highlighted code, and the HTML READMEs open with (logos and badges become `[image: …]`).
- **Gets around documents.** Search in a document (`/`), jump between headings (`]` `[`) or through an outline (`o`), and follow links with the keyboard (`f`) or the mouse. `Esc` goes back to where you were.
- **Links both ways.** The header shows how many documents a file links to and how many link to it; `L` lists them. Broken relative links are struck through.
- **Live reload.** Edit in another window, or press `e` to open your editor at the line you're reading; `lsmd` re-renders in place.
- **Scriptable.** When the output isn't a terminal, `lsmd FILE` prints the rendered document and `lsmd` lists the files.
- **macOS, Linux and Windows.**

## Install

lsmd runs on macOS, Linux (x86_64 and 64-bit ARM) and Windows 10 or later (x86_64).

With [Homebrew](https://brew.sh), on macOS and Linux:

```sh
brew install sanford/tap/lsmd
```

On Windows, download `lsmd-windows-x64.zip` from the [latest release](https://github.com/sanford/lsmd/releases/latest), unzip it, and put `lsmd.exe` in a folder on your `PATH`. It needs nothing else installed. Or, from PowerShell:

```powershell
$dir = "$env:LOCALAPPDATA\Programs\lsmd"
Invoke-WebRequest https://github.com/sanford/lsmd/releases/latest/download/lsmd-windows-x64.zip -OutFile "$env:TEMP\lsmd.zip"
Expand-Archive "$env:TEMP\lsmd.zip" $dir -Force
[Environment]::SetEnvironmentVariable('Path', "$([Environment]::GetEnvironmentVariable('Path', 'User'));$dir", 'User')
```

Then open a new terminal and run `lsmd`.

With Cargo, if you have a [Rust toolchain](https://rustup.rs):

```sh
cargo install --git https://github.com/sanford/lsmd
```

Or build from source:

```sh
git clone https://github.com/sanford/lsmd
cd lsmd
cargo build --release
./target/release/lsmd
```

While hacking on it, `./run.sh [ARGS]` (or `.\run.ps1 [ARGS]` on Windows) builds, installs to `~/.local/bin`, and runs in one step.

## Usage

```sh
lsmd                 # browse the Markdown files under this directory
lsmd docs/           # ...under docs/
lsmd README.md       # read one file (Esc for the list of the others)
lsmd -s README.md    # ...with its source alongside
cat notes.md | lsmd  # read standard input
lsmd README.md | less -R   # not a terminal: print it rendered
lsmd | wc -l         # not a terminal: list the files
```

`.gitignore`d and hidden files are left out; `-a` includes them. `-w 100` caps the text width. `-p` prints without colors, as does setting `NO_COLOR`.

### Keys

In the file list:

| Key | |
|---|---|
| `↑` `↓` `j` `k` | Move |
| `Enter` `→` `l` | Read the selected file |
| `/` | Filter the list by name (fuzzy) |
| `s` | Search the text of every file in the list |
| `m` | Sort by name or by date |
| `Space` `b` | Page through the preview |
| `q` `Esc` | Quit |

Reading:

| Key | |
|---|---|
| `↑` `↓` `j` `k`, `Space` `b`, `d` `u`, `g` `G` | Scroll by line, page, half page; top, bottom |
| `←` `→` `h` `l` | Scroll long code lines sideways (`0` back to the start) |
| `/` `n` `N` | Search this document; next and previous match |
| `]` `[` | Next and previous heading |
| `o` | Outline: jump to a heading |
| `f` | Follow a link: type the letters drawn on it |
| `L` | Links: what this links to, and what links here |
| `Esc` `Backspace` | Back to where you were before following a link; then the list |
| `q` | Quit |

Anywhere:

| Key | |
|---|---|
| `Tab` | Show or hide the source beside the rendered text |
| `Ctrl-W` `Shift-Tab` | Scroll the source side or the rendered side |
| `<` `>` | Move the divider |
| `e` | Edit the file in `$VISUAL` or `$EDITOR`, at the line you're reading |
| `y` `Y` | Copy the code block on screen, or the file's path |
| `\` | Keep the file list on screen while reading |
| `?` | All of the above |

The mouse works too: the wheel scrolls what's under it, clicking a file selects it and clicking again opens it, and clicking a link follows it. `--no-mouse` leaves the mouse to the terminal, so you can select text without holding a modifier key.

### Links between documents

The header shows how the document on screen is connected, like `3 linked docs · linked from 2`, and `L` lists those documents with their titles, along with links to sections, other files and the web. Following a link to another Markdown file opens it in `lsmd`; web links open in your browser after you confirm.

"Linked from" comes from an index of the links between all the Markdown files under the directory you're browsing. It's built in the background when `lsmd` starts, kept up to date as files change, and saved in `~/.lsmd/index/` so the next start only re-reads files that changed.

### Copying over SSH

`y` and `Y` use the system clipboard (`pbcopy`, `wl-copy`, `xclip`, `xsel` or `clip`). Over SSH they ask your terminal to do the copying instead, with an OSC 52 escape sequence, so the text lands on your own machine. Most modern terminals support this; in tmux, it needs `set -g set-clipboard on`. macOS Terminal doesn't support it.

## Configuration

Defaults for the options can go in `~/.lsmd/config.toml`. Everything is optional, and flags win over it:

```toml
theme = "dark"          # auto, dark or light (auto asks the terminal)
width = 100             # wrap text at 100 columns (0: the terminal's width)
source = true           # start with the source shown beside the text
source-side = "left"    # left or right
mouse = false           # leave the mouse to the terminal
all = true              # list hidden and .gitignored files too
sort = "date"           # name or date
```

## License

Copyright (C) 2026 Sanford Lincoln

`lsmd` is free software: you can redistribute it and/or modify it under the terms of the GNU General Public License as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version. See [LICENSE](LICENSE).
