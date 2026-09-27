//! Opening a file in the user's editor.

use std::io;
use std::path::Path;
use std::process::Command;

/// The command to edit `path` at `line`: `$VISUAL`, else `$EDITOR`, else vi
/// (notepad on Windows). The line goes the way the editor expects it.
pub fn command(path: &Path, line: usize) -> Option<Command> {
    let editor = ["VISUAL", "EDITOR"]
        .iter()
        .filter_map(std::env::var_os)
        .map(|v| v.to_string_lossy().into_owned())
        .find(|v| !v.trim().is_empty())
        .unwrap_or_else(|| {
            if cfg!(windows) {
                "notepad".into()
            } else {
                "vi".into()
            }
        });
    command_for(&editor, path, line)
}

/// The command to edit `path` at `line` with `editor`, which may include
/// arguments ("code -w").
fn command_for(editor: &str, path: &Path, line: usize) -> Option<Command> {
    let mut words = editor.split_whitespace();
    let program = words.next()?;
    let mut cmd = Command::new(program);
    cmd.args(words);
    let name = Path::new(program)
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let file = path.display();
    match name.as_str() {
        "code" | "code-insiders" | "cursor" | "codium" => {
            cmd.args(["--goto".into(), format!("{file}:{line}")])
        }
        "subl" | "zed" | "hx" | "helix" => cmd.arg(format!("{file}:{line}")),
        "notepad" => cmd.arg(path),
        _ => cmd.arg(format!("+{line}")).arg(path),
    };
    Some(cmd)
}

/// Runs the editor and waits for it.
pub fn edit(path: &Path, line: usize) -> io::Result<()> {
    let mut cmd = command(path, line).ok_or_else(|| io::Error::other("no editor set"))?;
    let status = cmd.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("editor exited with {status}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(editor: &str) -> Vec<String> {
        let cmd = command_for(editor, Path::new("a.md"), 12).unwrap();
        std::iter::once(cmd.get_program())
            .chain(cmd.get_args())
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn puts_the_line_where_each_editor_wants_it() {
        assert_eq!(args("vim"), ["vim", "+12", "a.md"]);
        assert_eq!(args("code -w"), ["code", "-w", "--goto", "a.md:12"]);
        assert_eq!(args("/usr/local/bin/hx"), ["/usr/local/bin/hx", "a.md:12"]);
    }
}
