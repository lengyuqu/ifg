//! Terminal-safe rendering of server-provided text.
//!
//! Titles, bodies, comments, diffs and error messages are attacker-controlled
//! by anyone who can write to the repository: a body carrying `ESC ]0;…`
//! (window title), `ESC[2J` (clear screen) or `ESC]52;…` (clipboard write)
//! hijacks the terminal of whoever runs `ifg issue view`. Bidi overrides are
//! dropped too, because they let one string masquerade as another.
//!
//! Newlines and tabs stay: they are legitimate layout.

/// Remove control characters from `text`.
pub fn sanitize(text: &str) -> String {
    text.chars().filter(|ch| !is_control(*ch)).collect()
}

fn is_control(ch: char) -> bool {
    match ch {
        '\n' | '\t' => false,
        '\u{7f}' => true,
        '\u{061c}'
        | '\u{200b}'..='\u{200f}'
        | '\u{202a}'..='\u{202e}'
        | '\u{2066}'..='\u{2069}'
        | '\u{feff}' => true,
        c => (c as u32) < 0x20 || matches!(c as u32, 0x80..=0x9f),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_terminal_and_bidi_controls() {
        // OSC 0 (title), BEL terminator, and CSI clear-screen.
        assert_eq!(
            sanitize("a\u{1b}]0;pwned\u{7}b\u{1b}[2Jc"),
            "a]0;pwnedb[2Jc"
        );
        assert_eq!(sanitize("line1\nline2\ttab"), "line1\nline2\ttab");
        assert_eq!(sanitize("rtl\u{202e}evil\u{202c}"), "rtlevil");
        assert_eq!(sanitize("nul\u{0}del\u{7f}c1\u{9b}"), "nuldelc1");
        // Ordinary text, including CJK and emoji, is untouched.
        assert_eq!(sanitize("中文 ✅ ok"), "中文 ✅ ok");
    }
}
