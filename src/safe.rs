//! Keeping what documents say from turning into what the terminal does.
//!
//! Markdown files are often someone else's, and their text, file names and
//! link targets reach the terminal. Control characters there (ESC, BEL and
//! the rest) would start escape sequences: setting the window title,
//! writing to the clipboard, moving the cursor. Bidirectional overrides
//! would make text display in a different order from how it reads.

use std::borrow::Cow;

/// Whether a character mustn't reach the terminal as it is.
pub fn is_unsafe(c: char) -> bool {
    c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// `s` with tabs as spaces and every other unsafe character as U+FFFD (�),
/// so something odd in a document is visible rather than acted on.
pub fn printable(s: &str) -> Cow<'_, str> {
    if !s.chars().any(is_unsafe) {
        return Cow::Borrowed(s);
    }
    Cow::Owned(
        s.chars()
            .map(|c| match c {
                '\t' => ' ',
                c if is_unsafe(c) => '\u{fffd}',
                c => c,
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_escapes_and_overrides() {
        assert_eq!(printable("plain"), "plain");
        assert_eq!(printable("a\u{1b}]0;x\u{7}b"), "a\u{fffd}]0;x\u{fffd}b");
        assert_eq!(printable("\u{9b}31m"), "\u{fffd}31m");
        assert_eq!(printable("ab\u{202e}cd"), "ab\u{fffd}cd");
        assert_eq!(printable("a\tb"), "a b");
        assert_eq!(printable("日本語 ✓"), "日本語 ✓");
    }
}
