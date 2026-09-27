# lsmd

**Find your way around a lot of Markdown.** `lsmd` is a terminal reader for directories with hundreds of Markdown files: design docs, plans, notes, changelogs. It lists them all on one screen, searches every one of them, and lets you chase links from document to document and pop back to where you started.

```
$ cd ~/dev/lanternworks && lsmd      # select DESIGN.md, press L
 lsmd  332 files in ~/dev/lanternworks                                10 linked docs · linked from 1  L links
┌ Files (332) ─────────────────────────────┐┌ DESIGN.md ─────────────────────────────────────────────────────┐
│ README.md                          9d ago││ lineHeight: 1.35                                               │
│ CHANGELOG.md                       6h ago││ mono:                                                          │
│ CLAUDE.md                         14h ago││ fontFamily: "JetBrains Mono, Consolas, monospace"              │
│ DESIGN.┌ Links ───────────────────────────────────────────────────────────────────────────────────┐        │
│ DOTNET.│ Links to (10)                                                                            │        │
│ FEATURE│   docs/DESIGN-CORE.md                    Lanternworks Core Design                        │        │
│ GOTCHAS│   docs/DESIGN-ARCHITECTURE.md            Lanternworks Technical Architecture             │        │
│ HISTORY│   docs/DESIGN-UI.md                      Lanternworks UI Design                          │        │
│ IDEAS.m│   docs/DESIGN-CAPTURE.md                 Lanternworks Capture System                     │        │
│ MACOS.m│   docs/DESIGN-TOOLS.md                   Lanternworks Annotation Tools                   │        │
│ MACOS_P│   docs/DESIGN-FEATURES.md                Lanternworks Features                           │        │
│ PRODUCT│   docs/DESIGN-IMPLEMENTATION.md          Lanternworks Implementation Plan                │        │
│ THIRD_P│   docs/CAMERA_OVERLAY_COMPOSITE.md       Camera Overlay Composite — Preview vs. Export   │        │
│ TODO.md│   docs/RECORDER_PANEL_PICKER_SUBFLOW.md  Recorder Panel — Custom Area Sub-flow           │        │
│ docs/AD│   docs/RECORDER_PANEL_VERIFICATION.md    Recorder Panel — Manual Verification Checklist  │        │
│ docs/AG│ Linked from (1)                                                                          │        │
│ docs/AG│   README.md                              Lanternworks                                    │        │
│ docs/AI│ type to filter, ⏎ to go                                                                  │        │
│ docs/AI└──────────────────────────────────────────────────────────────────────────────────────────┘        │
│ docs/ANNOTATION_SCALING.md         6d ago││ components:                                                    │
│ docs/API.md                        9d ago││ button-primary:                                                │
│ docs/APPLESCRIPT_CAPTURE.md        9d ago││ backgroundColor: "{colors.accent-iris}"                        │
└──────────────────────────────────────────┘└────────────────────────────────────────────────────────────────┘
 ↑↓ choose  ⏎ go  esc close
```

That's 332 documents. `DESIGN.md` links to 10 of them, listed with their titles, and one document links back to it. `Enter` opens any of them. From there, `L`, `f` or a click goes further, and `Esc` steps back through everything you opened, to the place you left each one.

## Why

Documentation piles up. A project that's been worked on for a while, especially with AI agents writing plans and design notes, ends up with hundreds of Markdown files that refer to each other. Reading them one at a time in an editor or a pager loses the thread. `lsmd` is built for following it:

- **Everything on one screen.** One line per file with its age, and a live preview of the selected one. `/` narrows the list by name as you type.
- **Search every file.** `s` searches the text of every document in the list and shows the matches grouped by file. Opening one lands on the match, highlighted, with `n` and `N` for the next ones.

  ```
  │ ┌ 5 matches for “undo stack” in 4 files ─────────────────────────────────────────────────────────────────┐ │
  │ │ CHANGELOG.md (1)                                                                                       │ │
  │ │    831  … files rather than unwinding the undo stack, because trim, crop, grade and stage decor never e│ │
  │ │ HISTORY.md (2)                                                                                         │ │
  │ │   10673  ## Session: Toolbar UX and Undo Stack Fix (December 2025)                                     │ │
  │ │   10698  **4. Undo Stack Fix - Base Layer Not Undoable**                                               │ │
  ```

- **Chase links, then come back.** Follow a link with the keyboard (`f` puts a letter on every link on screen; type it), with the mouse, or from the links panel, and keep going as deep as you like. `Esc` goes back one document at a time, each scrolled to where you were, and the footer says where it'll take you: `esc back to DESIGN-ARCHITECTURE.md`. When there's nowhere left to go back to, it takes you to the file list.
- **See links both ways.** The header shows how the document on screen is connected (`10 linked docs · linked from 1`), and `L` lists both directions with titles. "Linked from" answers the question you usually have in a pile of docs: what else refers to this? Links to files that don't exist are struck through.
- **Find your place in a long document.** `/` searches it, `]` and `[` jump between headings, and `o` opens an outline you can filter.
- **Stays current.** Documents reload as they're edited, keeping your place, so `lsmd` works beside your editor, or beside an agent that's writing. `e` opens your editor at the line you're reading.

It reads well, too. Text uses the full width of the terminal, and tables, code, task lists, footnotes, GitHub's `> [!NOTE]` alerts and the HTML that READMEs open with all render. `Tab` shows the Markdown source beside the rendered text, scrolled together, for when you're writing rather than reading.

`lsmd` runs on macOS, Linux and Windows. When its output isn't a terminal, `lsmd FILE` prints the rendered document and `lsmd` lists the files, for scripts.

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
