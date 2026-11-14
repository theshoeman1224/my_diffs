//! Intra-line ("inline") differences for modified rows, and line similarity.

use std::ops::Range;

use similar::{Algorithm, DiffOp, capture_diff_slices, diff_ratio};

use crate::filter::LineFilter;
use crate::options::WhitespaceMode;

/// Split a line into tokens: runs of word characters, runs of whitespace, and
/// single other characters. Returns byte ranges into `s`.
pub fn tokenize(s: &str) -> Vec<Range<usize>> {
    #[derive(PartialEq, Clone, Copy)]
    enum Class {
        Word,
        Space,
        Other,
    }
    fn class(c: char) -> Class {
        if c.is_alphanumeric() || c == '_' {
            Class::Word
        } else if c.is_whitespace() {
            Class::Space
        } else {
            Class::Other
        }
    }
    let mut out: Vec<Range<usize>> = Vec::new();
    let mut prev: Option<Class> = None;
    for (i, c) in s.char_indices() {
        let k = class(c);
        let end = i + c.len_utf8();
        match (prev, out.last_mut()) {
            (Some(p), Some(last)) if p == k && k != Class::Other => last.end = end,
            _ => out.push(i..end),
        }
        prev = Some(k);
    }
    out
}

/// Comparison key for one token, honoring whitespace and case options so that
/// differences the user asked to ignore are not highlighted.
fn token_key(tok: &str, ws: WhitespaceMode, ignore_case: bool) -> String {
    if ws != WhitespaceMode::Exact && tok.chars().all(char::is_whitespace) {
        return " ".to_owned();
    }
    if ignore_case {
        tok.to_lowercase()
    } else {
        tok.to_owned()
    }
}

/// Tokens of `s` with their comparison keys. Each mask match becomes a single
/// token keyed by its replacement, so masked regions (e.g. timestamps) that only
/// differ in masked content are not highlighted.
fn keyed_tokens(
    s: &str,
    ws: WhitespaceMode,
    ignore_case: bool,
    filter: &LineFilter,
) -> Vec<(Range<usize>, String)> {
    let mut out = Vec::new();
    let mut pos = 0;
    let push_plain = |out: &mut Vec<_>, from: usize, to: usize| {
        for r in tokenize(&s[from..to]) {
            let r = r.start + from..r.end + from;
            let key = token_key(&s[r.clone()], ws, ignore_case);
            out.push((r, key));
        }
    };
    for (span, key) in filter.mask_spans(s) {
        push_plain(&mut out, pos, span.start);
        pos = span.end;
        out.push((span, format!("\0{key}")));
    }
    push_plain(&mut out, pos, s.len());
    out
}

/// Byte ranges that changed in `old` and in `new`, at word granularity. Changed
/// tokens separated only by whitespace are merged into one range.
pub fn inline_changes(
    old: &str,
    new: &str,
    ws: WhitespaceMode,
    ignore_case: bool,
    filter: &LineFilter,
) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let (old_toks, old_keys): (Vec<_>, Vec<_>) = keyed_tokens(old, ws, ignore_case, filter)
        .into_iter()
        .unzip();
    let (new_toks, new_keys): (Vec<_>, Vec<_>) = keyed_tokens(new, ws, ignore_case, filter)
        .into_iter()
        .unzip();
    let ops = capture_diff_slices(Algorithm::Myers, &old_keys, &new_keys);

    let mut old_changed = Vec::new();
    let mut new_changed = Vec::new();
    for op in ops {
        let (_, o, n) = op.as_tag_tuple();
        if matches!(op, DiffOp::Equal { .. }) {
            continue;
        }
        push_merged(&mut old_changed, &old_toks[o], old);
        push_merged(&mut new_changed, &new_toks[n], new);
    }
    (old_changed, new_changed)
}

fn push_merged(out: &mut Vec<Range<usize>>, toks: &[Range<usize>], s: &str) {
    let (Some(first), Some(last)) = (toks.first(), toks.last()) else {
        return;
    };
    let range = first.start..last.end;
    match out.last_mut() {
        Some(prev) if s[prev.end..range.start].chars().all(char::is_whitespace) => {
            prev.end = range.end;
        }
        _ => out.push(range),
    }
}

/// Weight applied to character-level similarity. Below 1 so that whole-word
/// matches win over coincidental shared letters ("delta" is closer to
/// "delta changed" than to "zeta").
const CHAR_WEIGHT: f32 = 0.75;

/// Similarity of two (already normalized) lines in `0.0..=1.0`.
///
/// `max(word_similarity, CHAR_WEIGHT * char_similarity)`, where word similarity is
/// the Dice coefficient over non-whitespace tokens weighted by token length, and
/// char similarity is the Dice coefficient over characters. The char term keeps
/// single-word edits (`count` → `counter`) recognizable as modifications.
pub fn similarity(a: &str, b: &str) -> f32 {
    if a == b {
        return 1.0;
    }
    word_similarity(a, b).max(CHAR_WEIGHT * char_similarity(a, b))
}

fn word_similarity(a: &str, b: &str) -> f32 {
    let words = |s: &'_ str| -> Vec<String> {
        tokenize(s)
            .into_iter()
            .map(|r| s[r].to_owned())
            .filter(|t| !t.chars().all(char::is_whitespace))
            .collect()
    };
    let (aw, bw) = (words(a), words(b));
    let len = |w: &[String]| w.iter().map(|t| t.chars().count()).sum::<usize>();
    let total = len(&aw) + len(&bw);
    if total == 0 {
        return 0.0;
    }
    let matched: usize = capture_diff_slices(Algorithm::Myers, &aw, &bw)
        .into_iter()
        .filter_map(|op| match op {
            DiffOp::Equal {
                old_index, len: n, ..
            } => Some(len(&aw[old_index..old_index + n])),
            _ => None,
        })
        .sum();
    2.0 * matched as f32 / total as f32
}

fn char_similarity(a: &str, b: &str) -> f32 {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let ops = capture_diff_slices(Algorithm::Myers, &a, &b);
    diff_ratio(&ops, a.len(), b.len())
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn texts<'a>(s: &'a str, ranges: &[Range<usize>]) -> Vec<&'a str> {
        ranges.iter().map(|r| &s[r.clone()]).collect()
    }

    #[rstest]
    #[case::empty("", &[])]
    #[case::words("foo bar", &["foo", " ", "bar"])]
    #[case::punct("a(b, c)", &["a", "(", "b", ",", " ", "c", ")"])]
    #[case::repeated_punct("==", &["=", "="])]
    #[case::underscore_digits("x_1 + 22", &["x_1", " ", "+", " ", "22"])]
    #[case::unicode("héllo wörld", &["héllo", " ", "wörld"])]
    fn tokens(#[case] s: &str, #[case] expected: &[&str]) {
        assert_eq!(texts(s, &tokenize(s)), expected);
    }

    #[rstest]
    #[case::one_word("the quick fox", "the slow fox", &["quick"], &["slow"])]
    #[case::appended("let x = 1;", "let x = 1; // hi", &[], &[" // hi"])]
    #[case::adjacent_merge("a b c", "a x y", &["b c"], &["x y"])]
    #[case::identical("same", "same", &[], &[])]
    fn inline_exact(
        #[case] old: &str,
        #[case] new: &str,
        #[case] old_exp: &[&str],
        #[case] new_exp: &[&str],
    ) {
        let (o, n) = inline_changes(
            old,
            new,
            WhitespaceMode::Exact,
            false,
            &LineFilter::default(),
        );
        assert_eq!(texts(old, &o), old_exp);
        assert_eq!(texts(new, &n), new_exp);
    }

    #[test]
    fn whitespace_changes_not_highlighted_when_ignored() {
        let (o, n) = inline_changes(
            "a  b c",
            "a b d",
            WhitespaceMode::Change,
            false,
            &LineFilter::default(),
        );
        assert_eq!(texts("a  b c", &o), ["c"]);
        assert_eq!(texts("a b d", &n), ["d"]);
    }

    #[test]
    fn case_changes_not_highlighted_when_ignored() {
        let (o, n) = inline_changes(
            "Foo bar",
            "foo baz",
            WhitespaceMode::Exact,
            true,
            &LineFilter::default(),
        );
        assert_eq!(texts("Foo bar", &o), ["bar"]);
        assert_eq!(texts("foo baz", &n), ["baz"]);
    }

    #[test]
    fn masked_regions_compare_by_replacement() {
        let filter = LineFilter::new(&crate::DiffOptions {
            mask: vec![crate::options::MaskRule {
                pattern: r"\d{2}:\d{2}".into(),
                replace: "<T>".into(),
            }],
            ..crate::DiffOptions::default()
        })
        .unwrap();
        let (old, new) = ("10:00 job failed", "11:30 job retried");
        let (o, n) = inline_changes(old, new, WhitespaceMode::Exact, false, &filter);
        assert_eq!(texts(old, &o), ["failed"]);
        assert_eq!(texts(new, &n), ["retried"]);
    }

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[rstest]
    #[case::same("abc", "abc", 1.0)]
    #[case::both_empty("", "", 1.0)]
    #[case::one_empty("", "abc", 0.0)]
    #[case::disjoint("abc", "xyz", 0.0)]
    // word: 0; chars: abcd/abxy dice 0.5 * 0.75
    #[case::single_word_char_fallback("abcd", "abxy", 0.375)]
    // word: "delta" matched (5) of 5 + 12 => 10/17
    #[case::word_append("delta", "delta changed", 10.0 / 17.0)]
    // word: 0; chars: "eta" 3 of 5 + 4 => 6/9 * 0.75
    #[case::shared_letters_only("delta", "zeta", 0.5)]
    fn similarity_values(#[case] a: &str, #[case] b: &str, #[case] expected: f32) {
        let got = similarity(a, b);
        assert!(
            approx(got, expected),
            "similarity({a:?}, {b:?}) = {got}, expected {expected}"
        );
    }

    #[test]
    fn whole_word_match_beats_shared_letters() {
        assert!(similarity("delta", "delta changed") > similarity("delta", "zeta"));
    }
}
