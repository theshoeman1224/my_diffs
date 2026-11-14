//! Property tests: invariants that must hold for *any* input.

use mydiff_core::normalize::{display_text, split_lines};
use mydiff_core::options::{Algorithm, WhitespaceMode};
use mydiff_core::{DiffOptions, RowKind, diff_text};
use proptest::prelude::*;

/// Text built from a tiny alphabet so that lines frequently match.
fn text() -> impl Strategy<Value = String> {
    (
        prop::collection::vec("[ab \t]{0,5}", 0..12),
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(|(lines, trailing_nl, crlf)| {
            let sep = if crlf { "\r\n" } else { "\n" };
            let mut s = lines.join(sep);
            if trailing_nl && !s.is_empty() {
                s.push_str(sep);
            }
            s
        })
}

fn options() -> impl Strategy<Value = DiffOptions> {
    (
        prop_oneof![
            Just(WhitespaceMode::Exact),
            Just(WhitespaceMode::Trailing),
            Just(WhitespaceMode::Change),
            Just(WhitespaceMode::All),
        ],
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        prop_oneof![Just(vec![]), Just(vec!["^b".to_owned()])],
        prop_oneof![Just(Algorithm::Myers), Just(Algorithm::Patience)],
        0.0f32..1.2,
    )
        .prop_map(
            |(ws, case, blank, eol, ignore, algorithm, thr)| DiffOptions {
                whitespace: ws,
                ignore_case: case,
                ignore_blank_lines: blank,
                ignore_line_endings: eol,
                ignore,
                algorithm,
                modify_threshold: thr,
                ..DiffOptions::default()
            },
        )
}

fn shown(s: &str) -> Vec<&str> {
    split_lines(s).into_iter().map(display_text).collect()
}

proptest! {
    #[test]
    fn a_text_equals_itself(a in text(), opts in options()) {
        prop_assert!(diff_text(&a, &a, &opts).unwrap().is_identical());
    }

    #[test]
    fn rows_reconstruct_both_inputs(a in text(), b in text(), opts in options()) {
        let d = diff_text(&a, &b, &opts).unwrap();
        prop_assert_eq!(d.left_lines().collect::<Vec<_>>(), shown(&a));
        prop_assert_eq!(d.right_lines().collect::<Vec<_>>(), shown(&b));
    }

    #[test]
    fn line_numbers_are_consecutive(a in text(), b in text(), opts in options()) {
        let d = diff_text(&a, &b, &opts).unwrap();
        let left: Vec<_> = d.rows.iter().filter_map(|r| r.left.as_ref().map(|l| l.number)).collect();
        let right: Vec<_> = d.rows.iter().filter_map(|r| r.right.as_ref().map(|l| l.number)).collect();
        prop_assert_eq!(left, (1..=shown(&a).len()).collect::<Vec<_>>());
        prop_assert_eq!(right, (1..=shown(&b).len()).collect::<Vec<_>>());
    }

    #[test]
    fn row_shapes_match_kinds(a in text(), b in text(), opts in options()) {
        for r in diff_text(&a, &b, &opts).unwrap().rows {
            match r.kind {
                RowKind::Equal | RowKind::Modified => prop_assert!(r.left.is_some() && r.right.is_some()),
                RowKind::Removed => prop_assert!(r.left.is_some() && r.right.is_none()),
                RowKind::Added => prop_assert!(r.left.is_none() && r.right.is_some()),
                RowKind::Ignored => prop_assert!(r.left.is_some() || r.right.is_some()),
            }
            for side in [&r.left, &r.right].into_iter().flatten() {
                if r.kind != RowKind::Modified {
                    prop_assert!(side.changed.is_empty());
                }
                let mut prev_end = 0;
                for range in &side.changed {
                    prop_assert!(range.start >= prev_end && range.start < range.end);
                    prop_assert!(side.text.is_char_boundary(range.start));
                    prop_assert!(side.text.is_char_boundary(range.end));
                    prev_end = range.end;
                }
                prop_assert!(prev_end <= side.text.len());
            }
        }
    }

    #[test]
    fn ignoring_all_whitespace_ignores_reformatting(
        words in prop::collection::vec(prop::collection::vec("[a-z]{1,4}", 1..4), 0..8),
        seps in prop::collection::vec(prop_oneof![Just(""), Just(" "), Just("  "), Just("\t")], 32),
    ) {
        let mut k = 0;
        let mut next = || { k += 1; seps[k % seps.len()] };
        let a: String = words.iter().map(|ws| ws.join(" ") + "\n").collect();
        let b: String = words
            .iter()
            .map(|ws| {
                let mut line = next().to_owned();
                for w in ws {
                    line.push_str(w);
                    line.push_str(next());
                }
                line + "\n"
            })
            .collect();
        let opts = DiffOptions { whitespace: WhitespaceMode::All, ..DiffOptions::default() };
        prop_assert!(diff_text(&a, &b, &opts).unwrap().is_identical());
    }

    #[test]
    fn swapping_sides_swaps_removed_and_added_totals(a in text(), b in text()) {
        let opts = DiffOptions { modify_threshold: 2.0, ..DiffOptions::default() };
        let ab = diff_text(&a, &b, &opts).unwrap().stats();
        let ba = diff_text(&b, &a, &opts).unwrap().stats();
        // Edit distance is symmetric even when the particular script is not.
        prop_assert_eq!(ab.removed + ab.added, ba.removed + ba.added);
    }
}
