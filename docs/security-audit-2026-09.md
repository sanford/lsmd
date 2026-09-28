# Security audit, September 2026

An audit of lsmd 0.1.0, with every finding fixed in the release after it, and a [follow-up](#follow-up-color-themes) on the color themes added in 0.4.0.

## What was in scope

lsmd is often pointed at repositories someone else wrote, so the main threat is **hostile content**: Markdown text, file names and link targets chosen by an attacker. The audit checked what that content could make lsmd do:

- send escape sequences to the terminal, in the full-screen view, printed output and messages;
- run programs or other apps, through links;
- crash lsmd or exhaust memory;
- read or leak anything through its own files (`~/.lsmd`), the editor hand-off and the clipboard.

It also covered the dependencies (all 269 crates in `Cargo.lock`, against the OSV/RustSec advisory database) and the CI workflow, which runs on a self-hosted Windows machine.

Each finding was reproduced with a hostile test file before it was fixed, and checked again after.

## Findings

| # | Severity | Finding | Fix |
|---|---|---|---|
| 1 | High | **Escape sequences reached the terminal in printed output.** Control characters in a document's text, in HTML character references (`&#27;`), in link targets and in file names were written as they were by `lsmd FILE \| …`, `-p` and the file list. A document could set the window title or write to the clipboard (OSC 52). The full-screen view was not affected: ratatui drops control characters. | Every rendered line, and everything printed, replaces control characters and bidirectional overrides with `�`. |
| 2 | Medium | **Command injection on Windows through links.** Links were opened with `cmd /C start "" <target>`, and `cmd.exe` interpreted `&`, `\|`, `^` and `%VAR%` in the target, so a link to `https://x.io/&calc.exe` would also run `calc.exe` once opened. | Links open through `rundll32 url.dll,FileProtocolHandler`, which involves no command interpreter. |
| 3 | Medium | **Links could launch programs and app URL handlers.** After one confirmation, a link to `file:///…/Some.app`, `smb://`, `ssh://`, an app's own URL scheme, or a local `.command`, `.exe` or script was handed to the system opener, which runs such things. The confirmation showed only the target. | Only `http`, `https` and `mailto` links open, and only documents and media among local files (images, PDF, text, audio, video), never an executable file. Everything else is refused with a message. The confirmation names the host (`Open evil.example in your browser?`), so `https://yourbank.com@evil.example` can't hide where it goes. Targets with control characters are refused. |
| 4 | Medium | **Crash on HTML starting with a non-ASCII character.** `<p>日本語</p>` or `<div>é…` panicked; in the browser, just selecting such a file crashed lsmd. Ordinary READMEs could trigger it. | Fixed the slicing, which assumed one byte per character. |
| 5 | Medium | **Stack overflow on deeply nested blocks.** 20,000 nested `>` quotes aborted lsmd when previewed. | Nesting is capped at 64 levels; anything deeper shows as its text. |
| 6 | Low | **Background work read any file whole.** The link index and searching every file read each file fully into memory, however large. | Files over 16 MB are skipped in the background. They still open. |
| 7 | Low | **`~/.lsmd` was readable by other users** where home directories are. It holds the titles and links of every document browsed. | `~/.lsmd` and its index are made owner-only. |
| 8 | Low | **Mouse reporting left on after a crash**, so the shell received mouse codes. | A panic hook turns it off first. |

Along the way, a bare `<` in HTML ("2 < 3") was taken as the start of a tag and hid the text after it. That's fixed too.

## Checked and fine

- **The full-screen view** never shows control characters: ratatui filters them from everything it draws.
- **The editor** (`e`) is run directly, not through a shell, and always gets an absolute path, so a file named like an option (`--cmd=…`) can't inject one.
- **Copying** (`y`, `Y`) pipes text to the clipboard tool's input, or sends it base64-encoded in OSC 52. Neither lets the text act on anything.
- **The link index** resolves links without leaving the root, names its files by a hash of the root, and writes through a temporary file.
- **Scanning and watching** don't follow symbolic links, and only regular files are read.
- **No `unsafe` code, no network access,** and no shell anywhere.

## Dependencies

No crate has a known vulnerability. Two had "unmaintained" advisories, both through syntax highlighting:

- **yaml-rust** (RUSTSEC-2024-0320) was only for loading syntax definitions from YAML source. lsmd now asks syntect for just the features it uses, and yaml-rust is gone.
- **bincode** (RUSTSEC-2025-0141) decodes the syntax definitions built into the binary. That data is fixed at build time, never read from a document or a file, so this is accepted.

## CI

The Windows job runs on a self-hosted runner, so code from outside must never reach it. It only runs on pushes, never on pull requests. Pushes from anyone but the owners wait for approval in the `approval` environment, and GitHub's approval rule covers all outside contributors. The workflow can only read the repository. The actions it uses are now pinned to exact commits, so a compromised release of an action can't reach the runner.

## Not addressed

- **Release downloads have no published checksums or signatures,** and the Windows `.exe` is unsigned. Homebrew checks the source tarball's SHA-256 itself.
- **Terminal hyperlinks (OSC 8)** aren't used, so there's no link-spoofing surface there yet. If they're added, they need the same checks as finding 3.

## Follow-up: color themes

A review of what 0.4.0 added: following the Omarchy desktop's theme, the bundled themes, and the theme picker, which saves to the config file.

### What was in scope

- **Reading Omarchy's theme.** On Omarchy, lsmd reads `~/.local/state/omarchy/current/theme/colors.toml` and watches that directory for changes. The file usually comes from Omarchy, but themes can be installed from anyone's git repository, so its contents were treated as hostile.
- **Writing the config file.** Choosing a theme rewrites `~/.lsmd/config.toml`. Until now lsmd only read it.
- **Painting.** A picked theme draws its colors itself rather than changing the terminal's palette.
- **The new dependency**, `omarchy-themes`.

### Findings

| # | Severity | Finding | Fix |
|---|---|---|---|
| 9 | Low | **Saving a theme replaced a symlinked config with a plain file.** A `~/.lsmd/config.toml` linked into a dotfiles repository stopped being a link, and the repository's copy stopped changing. | The link is followed, and the file it points to is rewritten. |
| 10 | Low | **Saving a theme reset the config's permissions.** The rewritten file got the default permissions, so a config made owner-only became readable by other users. | The new file takes the old one's permissions before it replaces it. |

Both were found before release.

### Checked and fine

- **Theme files can't reach the terminal.** Only colors come out of `colors.toml`: each is parsed from `#rrggbb` into three numbers, and anything else in the file is dropped. No text from it is ever shown or printed.
- **Theme files can't crash lsmd.** The parser only slices strings at ASCII quote characters and checks colors are ASCII before slicing them. 300,000 mutated `colors.toml` files, about half of them valid enough to resolve every color, produced no panic. A file that doesn't parse leaves lsmd on its built-in colors.
- **The watcher** watches one directory, not its subdirectories, re-reads a single file on each change, and only redraws when the colors actually changed. Nothing in it runs anything.
- **The config file is written safely.** Only a theme name from lsmd's own list is written, so nothing from outside ends up in the file. It's written to a temporary file beside it and renamed into place, so it's never half-written, and the rest of the file is kept line for line.
- **Painting leaves the terminal as it was.** lsmd colors each cell itself and never changes the terminal's palette (OSC 4, 10, 11), so there's nothing to undo if it crashes or is killed.
- **The bundled themes** are Omarchy's `colors.toml` files, built into the binary, under the MIT license, whose notice ships beside them in `themes/LICENSE`.

### Dependencies

`omarchy-themes` 0.1.0 is new and has one maintainer, so its source was read in full before it was added:

- no build script and no `unsafe` code (`#![forbid(unsafe_code)]`)
- no network access and no processes started; it reads files and watches a directory.

Its only dependency, `notify`, was already used for live reload. It's pinned to exactly `=0.1.0`, and the checksum in `Cargo.lock` matches the source that was read, so an update can't arrive without being reviewed.

All 264 crates in `Cargo.lock` were checked against the OSV/RustSec advisory database again. The only advisory is the accepted one for `bincode`, described above.
