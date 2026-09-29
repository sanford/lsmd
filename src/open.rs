//! Opening links outside lsmd: web pages in the browser, and some local
//! files with their default app.
//!
//! Links come from documents, which may be someone else's, so this is
//! careful about what it hands to the system: the system's opener will run
//! programs and trigger any app's URL handler if asked to.

use std::path::Path;
use std::process::{Command, Stdio};

/// URL schemes that are opened. Others (`file:`, `smb:`, `ssh:`, apps'
/// own schemes) can do more than show a page, so they're refused.
const SCHEMES: &[&str] = &["http", "https", "mailto"];

/// Kinds of local file that are opened with their default app: things to
/// look at, not things that run.
const VIEWABLE: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "tif", "tiff", "ico", "heic", "avif", "pdf",
    "txt", "csv", "tsv", "json", "yaml", "yml", "toml", "log", "mp4", "mov", "webm", "mkv", "mp3",
    "wav", "m4a", "ogg", "flac",
];

/// Something that may be opened, and how to describe it when asking.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub target: String,
    /// "example.com in your browser", "diagram.png"
    pub what: String,
}

/// Checks a web link. Returns what to ask about, or why it won't be opened.
pub fn web(url: &str) -> Result<Target, String> {
    if url
        .chars()
        .any(|c| crate::safe::is_unsafe(c) || c.is_whitespace())
    {
        return Err("Not opening a link with hidden characters in it".into());
    }
    let scheme = url.split_once(':').map(|(s, _)| s.to_ascii_lowercase());
    let scheme = scheme.filter(|s| SCHEMES.contains(&s.as_str()));
    let Some(scheme) = scheme else {
        return Err(format!(
            "Not opening {}: lsmd only opens web pages and email links",
            shorten(url)
        ));
    };
    let what = if scheme == "mailto" {
        let to = url["mailto:".len()..].split('?').next().unwrap_or("");
        format!("an email to {to}")
    } else {
        format!("{} in your browser", host(url).unwrap_or("this page"))
    };
    Ok(Target {
        target: url.to_string(),
        what,
    })
}

/// Checks a local file that isn't Markdown.
pub fn file(path: &Path) -> Result<Target, String> {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let refuse = || {
        Err(format!(
            "Not opening {name}: it may be a program, and lsmd only opens documents and media"
        ))
    };
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    if !path.is_file() || !ext.is_some_and(|e| VIEWABLE.contains(&e.as_str())) || executable(path) {
        return refuse();
    }
    if path.to_string_lossy().chars().any(crate::safe::is_unsafe) {
        return Err("Not opening a file with hidden characters in its name".into());
    }
    Ok(Target {
        target: path.display().to_string(),
        what: name,
    })
}

#[cfg(unix)]
fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn executable(_: &Path) -> bool {
    false
}

/// The host of a URL, without any `user@` in front of it (which is how
/// `https://yourbank.com@evil.example` hides where it goes).
fn host(url: &str) -> Option<&str> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    let host = match host.strip_prefix('[') {
        Some(v6) => v6.split(']').next()?,
        None => host.split(':').next()?,
    };
    (!host.is_empty()).then_some(host)
}

fn shorten(s: &str) -> String {
    let mut out: String = s.chars().take(60).collect();
    if out.len() < s.len() {
        out.push('…');
    }
    out
}

/// Opens a checked target with the system's default app. On Windows this
/// avoids `cmd /c start`, which would run anything after an `&` in a URL.
pub fn open(t: &Target) -> std::io::Result<()> {
    let mut cmd = if cfg!(target_os = "macos") {
        Command::new("open")
    } else if cfg!(windows) {
        let mut c = Command::new("rundll32.exe");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else {
        Command::new("xdg-open")
    };
    cmd.arg(&t.target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_web_pages_and_names_the_host() {
        assert_eq!(
            web("https://example.com/a?b").unwrap().what,
            "example.com in your browser"
        );
        assert_eq!(
            web("https://yourbank.com@evil.example/x").unwrap().what,
            "evil.example in your browser"
        );
        assert_eq!(
            web("http://[::1]:8080/").unwrap().what,
            "::1 in your browser"
        );
        assert_eq!(
            web("mailto:me@x.io?subject=hi").unwrap().what,
            "an email to me@x.io"
        );
    }

    #[test]
    fn refuses_other_schemes_and_hidden_characters() {
        for url in [
            "file:///Applications/Calculator.app",
            "smb://server/share",
            "ssh://host",
            "vscode://file/x",
            "ms-msdt:/id",
            "javascript:alert(1)",
            "https://x.io/\u{1b}]0;x",
            "https://x.io/a\u{202e}b",
            "https://x.io/a b",
        ] {
            assert!(web(url).is_err(), "{url}");
        }
    }

    #[test]
    fn opens_only_documents_and_media() {
        let dir = std::env::temp_dir().join(format!("lsmd-open-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Thing.app")).unwrap();
        for f in [
            "a.png",
            "b.PDF",
            "run.command",
            "setup.exe",
            "x.sh",
            "noext",
        ] {
            std::fs::write(dir.join(f), "").unwrap();
        }
        assert!(file(&dir.join("a.png")).is_ok());
        assert!(file(&dir.join("b.PDF")).is_ok());
        for f in [
            "run.command",
            "setup.exe",
            "x.sh",
            "noext",
            "Thing.app",
            "missing.png",
        ] {
            assert!(file(&dir.join(f)).is_err(), "{f}");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.join("a.png"), std::fs::Permissions::from_mode(0o755))
                .unwrap();
            assert!(file(&dir.join("a.png")).is_err(), "executable image");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
