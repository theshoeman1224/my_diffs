//! Pure helpers for showing lines in fixed-width cells, shared by every
//! front-end (CLI renderers and TUI).

use std::ops::Range;

use unicode_width::UnicodeWidthChar;

/// Split `text` into `(piece, is_changed)` segments by byte ranges.
pub fn segments<'a>(text: &'a str, changed: &[Range<usize>]) -> Vec<(&'a str, bool)> {
    let mut out = Vec::new();
    let mut pos = 0;
    for r in changed {
        if r.start > pos {
            out.push((&text[pos..r.start], false));
        }
        out.push((&text[r.clone()], true));
        pos = r.end;
    }
    if pos < text.len() {
        out.push((&text[pos..], false));
    }
    out
}

/// Make a piece of text safe for a fixed-width cell: tabs become spaces (4-column
/// stops), other control characters become `�`.
pub fn sanitize(s: &str, start_col: usize) -> String {
    let mut out = String::with_capacity(s.len());
    let mut col = start_col;
    for c in s.chars() {
        if c == '\t' {
            let n = 4 - col % 4;
            out.extend(std::iter::repeat_n(' ', n));
            col += n;
        } else if c.is_control() {
            out.push('\u{FFFD}');
            col += 1;
        } else {
            out.push(c);
            col += c.width().unwrap_or(0);
        }
    }
    out
}

/// Display width of a string.
pub fn width(s: &str) -> usize {
    s.chars().map(|c| c.width().unwrap_or(0)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_cover_text() {
        let s = "the quick fox";
        assert_eq!(
            segments(s, &[4..9]),
            [("the ", false), ("quick", true), (" fox", false)]
        );
        assert_eq!(segments(s, &[0..3]), [("the", true), (" quick fox", false)]);
        assert_eq!(segments("", &[]), Vec::<(&str, bool)>::new());
    }

    #[test]
    fn sanitize_expands_tabs_to_stops() {
        assert_eq!(sanitize("a\tb", 0), "a   b");
        assert_eq!(sanitize("\tb", 2), "  b");
        assert_eq!(sanitize("x\u{1b}y", 0), "x\u{FFFD}y");
    }

    #[test]
    fn width_counts_wide_chars() {
        assert_eq!(width("ab"), 2);
        assert_eq!(width("日本"), 4);
    }
}
