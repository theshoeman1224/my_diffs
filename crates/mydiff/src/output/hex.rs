//! Hex-dump output for binary comparisons.

use std::fmt::Write as _;

use anstyle::Style;
use mydiff_core::RowKind;
use mydiff_core::binary::{HexDiff, HexRow, ROW_BYTES, ascii};

use super::Theme;

/// `00 01 .. 0f  |ascii|` for one side of a row, emphasizing differing bytes.
fn dump(bytes: &[u8], row: &HexRow<'_>, base: Style, theme: Theme) -> String {
    let mut hex = String::new();
    let mut text = String::new();
    for k in 0..ROW_BYTES {
        if k == ROW_BYTES / 2 {
            hex.push(' ');
        }
        let style = if row.diff[k] {
            Theme::changed_span()
        } else {
            base
        };
        match bytes.get(k) {
            Some(b) => {
                hex.push_str(&theme.paint(style, &format!("{b:02x}")));
                hex.push(' ');
                text.push_str(&theme.paint(style, &ascii(*b).to_string()));
            }
            None => hex.push_str("   "),
        }
    }
    let pad = ROW_BYTES - bytes.len().min(ROW_BYTES);
    format!("{hex} |{text}|{}", " ".repeat(pad))
}

/// Unified-style hex diff: changed rows only, grouped by adjacency.
pub fn unified(diff: &HexDiff, left: &str, right: &str, theme: Theme) -> String {
    if diff.is_identical() {
        return String::new();
    }
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{}",
        theme.paint(Theme::header(), &format!("--- {left}"))
    );
    let _ = writeln!(
        out,
        "{}",
        theme.paint(Theme::header(), &format!("+++ {right}"))
    );
    let mut prev: Option<usize> = None;
    for &r in &diff.changed_rows {
        let row = diff.row(r);
        if prev.is_none_or(|p| p + 1 != r) {
            let _ = writeln!(
                out,
                "{}",
                theme.paint(Theme::hunk(), &format!("@@ 0x{:08x} @@", row.offset))
            );
        }
        let (red, green) = (Theme::row(RowKind::Removed), Theme::row(RowKind::Added));
        let _ = writeln!(
            out,
            "{}{:08x}  {}",
            theme.paint(red, "-"),
            row.offset,
            dump(row.left, &row, red, theme).trim_end()
        );
        let _ = writeln!(
            out,
            "{}{:08x}  {}",
            theme.paint(green, "+"),
            row.offset,
            dump(row.right, &row, green, theme).trim_end()
        );
        prev = Some(r);
    }
    out
}

/// Every row, both sides next to each other (about 150 columns wide).
pub fn side_by_side(diff: &HexDiff, theme: Theme) -> String {
    let mut out = String::new();
    for r in 0..diff.row_count() {
        let row = diff.row(r);
        let kind = if row.is_change() {
            RowKind::Modified
        } else {
            RowKind::Equal
        };
        let style = Theme::row(kind);
        let _ = writeln!(
            out,
            "{:08x}  {} {} {}",
            row.offset,
            dump(row.left, &row, style, theme),
            theme.paint(style, &kind.marker().to_string()),
            dump(row.right, &row, style, theme).trim_end()
        );
    }
    out
}

/// One-line summary.
pub fn summary(diff: &HexDiff, left: &str, right: &str) -> String {
    let s = diff.stats();
    match s.first_difference {
        None => format!(
            "{left} vs {right}: identical binary ({} bytes)\n",
            s.left_len
        ),
        Some(first) => format!(
            "{left} vs {right}: binary, {} differing byte(s), first at 0x{first:x} (sizes {} vs {})\n",
            s.differing_bytes, s.left_len, s.right_len
        ),
    }
}

#[cfg(test)]
mod tests {
    use mydiff_core::binary::diff_bytes;

    use super::*;

    fn sample() -> HexDiff {
        let a: Vec<u8> = b"0123456789abcdef-the quick brown fox-0123456789abcdef!".to_vec();
        let mut b = a.clone();
        b[21] = b'Q'; // the 'q' of "quick"
        b.truncate(52);
        diff_bytes(a, b)
    }

    #[test]
    fn unified_hex() {
        insta::assert_snapshot!(unified(&sample(), "a.bin", "b.bin", Theme::PLAIN), @"
        --- a.bin
        +++ b.bin
        @@ 0x00000010 @@
        -00000010  2d 74 68 65 20 71 75 69  63 6b 20 62 72 6f 77 6e  |-the quick brown|
        +00000010  2d 74 68 65 20 51 75 69  63 6b 20 62 72 6f 77 6e  |-the Quick brown|
        @@ 0x00000030 @@
        -00000030  62 63 64 65 66 21                                 |bcdef!|
        +00000030  62 63 64 65                                       |bcde|
        ");
    }

    #[test]
    fn side_by_side_marks_changed_rows() {
        let out = side_by_side(&sample(), Theme::PLAIN);
        let markers: Vec<char> = out
            .lines()
            .map(|l| l.chars().nth(10 + 49 + 19 + 1).unwrap())
            .collect();
        assert_eq!(markers, [' ', '~', ' ', '~']);
    }

    #[test]
    fn summaries() {
        assert_eq!(
            summary(&sample(), "a", "b"),
            "a vs b: binary, 3 differing byte(s), first at 0x15 (sizes 54 vs 52)\n"
        );
        let same = diff_bytes(vec![1, 2], vec![1, 2]);
        assert_eq!(
            summary(&same, "a", "b"),
            "a vs b: identical binary (2 bytes)\n"
        );
        assert_eq!(unified(&same, "a", "b", Theme::PLAIN), "");
    }
}
