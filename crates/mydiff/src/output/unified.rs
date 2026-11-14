//! Unified diff output (`diff -u` compatible when no filters are active).
//!
//! Ignored rows inside a hunk are still printed as `-`/`+` (dimmed) when their
//! sides differ, so the output always applies as a patch — the same approach as
//! GNU `diff -I`. Hunks are only created around real changes.

use std::fmt::Write as _;

use mydiff_core::{Row, RowKind, TextDiff};

use super::Theme;

/// Render a text diff in unified format. Returns an empty string when identical.
pub fn render(
    diff: &TextDiff,
    left_label: &str,
    right_label: &str,
    context: usize,
    theme: Theme,
) -> String {
    let hunks = diff.hunks(context);
    if hunks.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    let h = Theme::header();
    let _ = writeln!(out, "{}", theme.paint(h, &format!("--- {left_label}")));
    let _ = writeln!(out, "{}", theme.paint(h, &format!("+++ {right_label}")));

    // Lines of each side that precede the current row.
    let (mut left_before, mut right_before) = (0usize, 0usize);
    let mut cursor = 0usize;
    for hunk in hunks {
        for r in &diff.rows[cursor..hunk.rows.start] {
            left_before += usize::from(r.left.is_some());
            right_before += usize::from(r.right.is_some());
        }
        let rows = &diff.rows[hunk.rows.clone()];
        let lc = rows.iter().filter(|r| r.left.is_some()).count();
        let rc = rows.iter().filter(|r| r.right.is_some()).count();
        let start = |before: usize, count: usize| if count == 0 { before } else { before + 1 };
        let header = format!(
            "@@ -{},{lc} +{},{rc} @@",
            start(left_before, lc),
            start(right_before, rc)
        );
        let _ = writeln!(out, "{}", theme.paint(Theme::hunk(), &header));
        render_rows(&mut out, rows, theme);
        left_before += lc;
        right_before += rc;
        cursor = hunk.rows.end;
    }
    out
}

/// Print rows, emitting each run of non-context rows as all `-` lines then all `+`
/// lines, as unified diffs require.
fn render_rows(out: &mut String, rows: &[Row], theme: Theme) {
    let mut minus: Vec<String> = Vec::new();
    let mut plus: Vec<String> = Vec::new();
    let flush = |out: &mut String, minus: &mut Vec<String>, plus: &mut Vec<String>| {
        for l in minus.drain(..).chain(plus.drain(..)) {
            out.push_str(&l);
            out.push('\n');
        }
    };
    for r in rows {
        let style = Theme::row(r.kind);
        let is_context = match (&r.left, &r.right) {
            (Some(l), Some(rt)) => {
                r.kind == RowKind::Equal || (r.kind == RowKind::Ignored && l.text == rt.text)
            }
            _ => false,
        };
        if is_context {
            flush(out, &mut minus, &mut plus);
            let text = &r.left.as_ref().map_or("", |l| l.text.as_str());
            let _ = writeln!(out, "{}", theme.paint(style, &format!(" {text}")));
            continue;
        }
        if let Some(l) = &r.left {
            minus.push(format!(
                "{}{}",
                theme.paint(style, "-"),
                theme.paint_line(style, &l.text, &l.changed)
            ));
        }
        if let Some(rt) = &r.right {
            plus.push(format!(
                "{}{}",
                theme.paint(style, "+"),
                theme.paint_line(style, &rt.text, &rt.changed)
            ));
        }
    }
    flush(out, &mut minus, &mut plus);
}

#[cfg(test)]
mod tests {
    use mydiff_core::{DiffOptions, diff_text};

    use super::*;

    fn unified(left: &str, right: &str, opts: &DiffOptions, context: usize) -> String {
        let d = diff_text(left, right, opts).unwrap();
        render(&d, "a.txt", "b.txt", context, Theme::PLAIN)
    }

    #[test]
    fn identical_prints_nothing() {
        assert_eq!(unified("a\n", "a\n", &DiffOptions::default(), 3), "");
    }

    #[test]
    fn basic_hunk() {
        insta::assert_snapshot!(unified(
            "alpha\nbeta\ngamma\ndelta\nepsilon\n",
            "alpha\ngamma\ndelta changed\nzeta\nepsilon\n",
            &DiffOptions::default(),
            1
        ), @r"
        --- a.txt
        +++ b.txt
        @@ -1,5 +1,5 @@
         alpha
        -beta
         gamma
        -delta
        +delta changed
        +zeta
         epsilon
        ");
    }

    #[test]
    fn separate_hunks_and_line_numbers() {
        let left: String = (1..=20).map(|i| format!("line {i}\n")).collect();
        let right = left
            .replace("line 3\n", "line three\n")
            .replace("line 18\n", "");
        insta::assert_snapshot!(unified(&left, &right, &DiffOptions::default(), 1), @r"
        --- a.txt
        +++ b.txt
        @@ -2,3 +2,3 @@
         line 2
        -line 3
        +line three
         line 4
        @@ -17,3 +17,2 @@
         line 17
        -line 18
         line 19
        ");
    }

    #[test]
    fn pure_insertion_into_empty_file() {
        insta::assert_snapshot!(unified("", "new\n", &DiffOptions::default(), 3), @r"
        --- a.txt
        +++ b.txt
        @@ -0,0 +1,1 @@
        +new
        ");
    }

    #[test]
    fn ignored_lines_stay_patch_correct() {
        let opts = DiffOptions {
            ignore: vec!["^#".into()],
            ..DiffOptions::default()
        };
        insta::assert_snapshot!(unified("# v1\nx = 1\n", "# v2\nx = 2\n", &opts, 3), @r"
        --- a.txt
        +++ b.txt
        @@ -1,2 +1,2 @@
        -# v1
        -x = 1
        +# v2
        +x = 2
        ");
    }

    #[test]
    fn only_ignored_differences_make_no_hunks() {
        let opts = DiffOptions {
            ignore: vec!["^#".into()],
            ..DiffOptions::default()
        };
        assert_eq!(unified("# v1\nx\n", "# v2\nx\n", &opts, 3), "");
    }
}
