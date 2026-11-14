//! Two-column output for terminals and pipes.
//!
//! ```text
//!    1 left text            ~    1 right text
//! ```
//! Gutter markers: `-` removed, `+` added, `~` modified, `.` ignored.

use std::fmt::Write as _;

use mydiff_core::{LineRef, TextDiff};

use super::{Theme, sanitize, segments, width};

/// Narrowest usable total width.
pub const MIN_WIDTH: usize = 40;

/// Render every row in two columns fitting `total_width`.
pub fn render(
    diff: &TextDiff,
    left_label: &str,
    right_label: &str,
    total_width: usize,
    theme: Theme,
) -> String {
    let total = total_width.max(MIN_WIDTH);
    // "NNNN " + col + " M " + "NNNN " + col
    let col = (total - 5 - 3 - 5) / 2;
    let mut out = String::new();
    let h = Theme::header();
    // Labels start where the text columns start.
    let header = format!(
        "     {}        {}",
        fit_plain(left_label, col),
        fit_plain(right_label, col)
    );
    let _ = writeln!(out, "{}", theme.paint(h, header.trim_end()));
    for r in &diff.rows {
        let style = Theme::row(r.kind);
        let left = cell(r.left.as_ref(), col, style, theme);
        let right = cell(r.right.as_ref(), col, style, theme);
        let line = format!(
            "{} {} {} {} {}",
            num(r.left.as_ref(), theme),
            left,
            theme.paint(style, &r.kind.marker().to_string()),
            num(r.right.as_ref(), theme),
            right
        );
        let _ = writeln!(out, "{}", line.trim_end());
    }
    out
}

fn num(l: Option<&LineRef>, theme: Theme) -> String {
    l.map_or_else(
        || "    ".to_owned(),
        |l| theme.paint(Theme::dim(), &format!("{:>4}", l.number)),
    )
}

/// Fit a label into `col` cells, keeping its end (the file name) when too long.
fn fit_plain(s: &str, col: usize) -> String {
    let clean = sanitize(s, 0);
    let chars: Vec<char> = clean.chars().collect();
    let cw = |c: &char| width(c.encode_utf8(&mut [0; 4]));
    let total: usize = chars.iter().map(cw).sum();
    let mut out = if total <= col {
        clean
    } else {
        let mut tail = Vec::new();
        let mut w = 1; // room for the leading ellipsis
        for c in chars.iter().rev() {
            if w + cw(c) > col {
                break;
            }
            w += cw(c);
            tail.push(*c);
        }
        std::iter::once('…').chain(tail.into_iter().rev()).collect()
    };
    let w = width(&out);
    out.extend(std::iter::repeat_n(' ', col.saturating_sub(w)));
    out
}

/// One column: sanitized, truncated with `…`, padded to exactly `col` cells.
fn cell(line: Option<&LineRef>, col: usize, base: anstyle::Style, theme: Theme) -> String {
    let Some(line) = line else {
        return " ".repeat(col);
    };
    let mut out = String::new();
    let mut used = 0;
    let full_width = width(&sanitize(&line.text, 0));
    let budget = if full_width > col { col - 1 } else { col };
    'outer: for (seg, hot) in segments(&line.text, &line.changed) {
        let clean = sanitize(seg, used);
        let mut piece = String::new();
        for c in clean.chars() {
            let cw = width(c.encode_utf8(&mut [0; 4]));
            if used + cw > budget {
                out.push_str(&theme.paint(if hot { Theme::changed_span() } else { base }, &piece));
                break 'outer;
            }
            piece.push(c);
            used += cw;
        }
        out.push_str(&theme.paint(if hot { Theme::changed_span() } else { base }, &piece));
    }
    if full_width > col {
        out.push_str(&theme.paint(Theme::dim(), "…"));
        used += 1;
    }
    out.extend(std::iter::repeat_n(' ', col.saturating_sub(used)));
    out
}

#[cfg(test)]
mod tests {
    use mydiff_core::{DiffOptions, diff_text};

    use super::*;

    fn sbs(left: &str, right: &str, w: usize) -> String {
        let d = diff_text(left, right, &DiffOptions::default()).unwrap();
        render(&d, "a.txt", "b.txt", w, Theme::PLAIN)
    }

    #[test]
    fn basic_layout() {
        insta::assert_snapshot!(sbs(
            "alpha\nbeta\ngamma\ndelta\n",
            "alpha\ngamma\ndelta changed\nzeta\n",
            50
        ), @"
          a.txt                     b.txt
        1 alpha                   1 alpha
        2 beta               -
        3 gamma                   2 gamma
        4 delta              ~    3 delta changed
                             +    4 zeta
        ");
    }

    #[test]
    fn long_labels_keep_their_end() {
        assert_eq!(fit_plain("some/long/dir/left.txt", 10), "…/left.txt");
        assert_eq!(fit_plain("short", 8), "short   ");
    }

    #[test]
    fn long_lines_truncate_with_ellipsis() {
        let out = sbs("a very long line that will not fit\n", "short\n", 40);
        assert!(out.contains("a very long …"), "{out}");
    }

    #[test]
    fn cells_are_exactly_column_width() {
        let line = |text: &str, changed: Vec<std::ops::Range<usize>>| LineRef {
            number: 1,
            text: text.to_owned(),
            changed,
        };
        let style = anstyle::Style::new();
        for (text, changed) in [
            ("", vec![]),
            ("\tx", vec![]),
            ("日本語", vec![]),
            ("日本語日本語日本語", vec![3..6]),
            ("a\tb\tc\td\te\tf", vec![2..3]),
            ("exactly ten", vec![]),
        ] {
            for col in [1, 2, 5, 10, 11] {
                let c = cell(Some(&line(text, changed.clone())), col, style, Theme::PLAIN);
                assert_eq!(width(&c), col, "text={text:?} col={col} cell={c:?}");
            }
        }
        assert_eq!(width(&cell(None, 7, style, Theme::PLAIN)), 7);
    }
}
