//! Vim-style search: pattern syntax and the pure "where is the next match" logic.
//!
//! Patterns use the [Rust `regex` syntax](https://docs.rs/regex/latest/regex/#syntax)
//! (like vim's "very magic" `\v` mode: `( ) | + ? {n}` are special without a
//! backslash), plus these vim items:
//!
//! | Item | Meaning |
//! |---|---|
//! | `\c` anywhere | ignore case for the whole pattern |
//! | `\C` anywhere | match case for the whole pattern (the default); `\c` wins if both appear |
//! | `\<` / `\>` | start / end of a word |
//!
//! Vim's backslashed forms (`\(`, `\+`, `\{3}`) match those characters literally here.
//!
//! A search only looks at the text of the active pane (see `App::focus`).

use regex::{Regex, RegexBuilder};

/// Which way `/` (forward) or `?` (backward) searches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Backward,
}

impl Direction {
    /// The other way, used by `N`.
    #[must_use]
    pub fn reverse(self) -> Self {
        match self {
            Self::Forward => Self::Backward,
            Self::Backward => Self::Forward,
        }
    }

    /// The key that starts a search this way, shown in front of the pattern.
    pub fn leader(self) -> char {
        match self {
            Self::Forward => '/',
            Self::Backward => '?',
        }
    }
}

/// Compile a vim-style pattern (see the module docs).
///
/// The error is a one-line message for the status bar.
pub fn compile(pattern: &str) -> Result<Regex, String> {
    let mut source = String::with_capacity(pattern.len());
    let mut ignore_case = false;
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            source.push(c);
            continue;
        }
        match chars.next() {
            Some('c') => ignore_case = true,
            Some('C') => {}
            // Everything else, including `\<` and `\>`, is regex syntax already;
            // keep the pair so `\\c` stays a literal backslash then `c`.
            Some(other) => {
                source.push('\\');
                source.push(other);
            }
            None => source.push('\\'),
        }
    }
    RegexBuilder::new(&source)
        .case_insensitive(ignore_case)
        .build()
        .map_err(|e| {
            let text = e.to_string();
            let reason = text.lines().last().unwrap_or_default();
            let reason = reason.strip_prefix("error: ").unwrap_or(reason);
            format!("E383: Invalid search string: {pattern} ({reason})")
        })
}

/// One match: a byte range of the active side's text in diff row `row`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub row: usize,
    pub start: usize,
    pub end: usize,
}

/// Where the next search starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub row: usize,
    pub byte: usize,
    /// The cursor sits on a match, so a forward search must move past it.
    /// Otherwise (start of the top screen line) a match right here counts.
    pub on_match: bool,
}

/// Every match of `re` in `texts` (row index, text), in order.
pub fn collect<'a>(texts: impl IntoIterator<Item = (usize, &'a str)>, re: &Regex) -> Vec<Match> {
    texts
        .into_iter()
        .flat_map(|(row, text)| {
            re.find_iter(text).map(move |m| Match {
                row,
                start: m.start(),
                end: m.end(),
            })
        })
        .collect()
}

/// Index of the next match from `cursor` going `dir`, and whether the search
/// wrapped around the end (or start) of the file. `None` when there are no matches.
pub fn find(matches: &[Match], cursor: Cursor, dir: Direction) -> Option<(usize, bool)> {
    if matches.is_empty() {
        return None;
    }
    let here = (cursor.row, cursor.byte);
    let key = |m: &Match| (m.row, m.start);
    match dir {
        Direction::Forward => {
            let after = matches.partition_point(|m| {
                if cursor.on_match {
                    key(m) <= here
                } else {
                    key(m) < here
                }
            });
            Some(if after < matches.len() {
                (after, false)
            } else {
                (0, true)
            })
        }
        Direction::Backward => {
            let before = matches.partition_point(|m| key(m) < here);
            Some(match before.checked_sub(1) {
                Some(i) => (i, false),
                None => (matches.len() - 1, true),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::plain("foo", "a foo b", Some("foo"))]
    #[case::case_sensitive_by_default("foo", "FOO", None)]
    #[case::backslash_c_ignores_case(r"\cfoo", "FOO", Some("FOO"))]
    #[case::backslash_c_at_the_end(r"foo\c", "Foo", Some("Foo"))]
    #[case::backslash_upper_c_matches_case(r"\Cfoo", "FOO", None)]
    #[case::lower_c_wins_over_upper(r"\C\cfoo", "FOO", Some("FOO"))]
    #[case::escaped_backslash_is_literal(r"a\\c", r"a\c", Some(r"a\c"))]
    #[case::word_start(r"\<cat", "concat cat", Some("cat"))]
    #[case::word_end(r"cat\>", "cats cat", Some("cat"))]
    #[case::whole_word(r"\<cat\>", "concat cats cat", Some("cat"))]
    #[case::rust_regex_syntax(r"(ab)+\d", "xababa1abab2", Some("abab2"))]
    #[case::escaped_dot(r"a\.b", "axb a.b", Some("a.b"))]
    #[case::vim_group_is_literal_here(r"\(a\)\+", "aa (a)+", Some("(a)+"))]
    #[case::vim_count_is_literal_here(r"a\{2}", "aa a{2}", Some("a{2}"))]
    fn patterns(#[case] pattern: &str, #[case] hay: &str, #[case] found: Option<&str>) {
        let re = compile(pattern).unwrap();
        assert_eq!(re.find(hay).map(|m| m.as_str()), found);
    }

    #[test]
    fn word_boundaries_mark_the_right_spot() {
        let re = compile(r"\<cat\>").unwrap();
        assert_eq!(re.find("concat cats cat").unwrap().start(), 12);
    }

    #[rstest]
    #[case::unclosed_group("(ab", "unclosed group")]
    #[case::trailing_backslash(r"ab\", "incomplete escape sequence")]
    fn bad_patterns_explain_themselves(#[case] pattern: &str, #[case] reason: &str) {
        let err = compile(pattern).unwrap_err();
        assert!(
            err.starts_with(&format!("E383: Invalid search string: {pattern} (")),
            "{err}"
        );
        assert!(err.contains(reason), "{err}");
        assert!(!err.contains('\n'), "one line for the status bar: {err}");
    }

    fn m(row: usize, start: usize) -> Match {
        Match {
            row,
            start,
            end: start + 1,
        }
    }

    const MATCHES: [Match; 4] = [
        Match {
            row: 1,
            start: 0,
            end: 1,
        },
        Match {
            row: 1,
            start: 5,
            end: 6,
        },
        Match {
            row: 4,
            start: 2,
            end: 3,
        },
        Match {
            row: 9,
            start: 0,
            end: 1,
        },
    ];

    fn at(row: usize, byte: usize, on_match: bool) -> Cursor {
        Cursor {
            row,
            byte,
            on_match,
        }
    }

    #[rstest]
    #[case::from_top_line_counts_match_here(at(1, 0, false), Direction::Forward, Some((0, false)))]
    #[case::from_match_moves_past_it(at(1, 0, true), Direction::Forward, Some((1, false)))]
    #[case::same_row_later_column(at(1, 5, true), Direction::Forward, Some((2, false)))]
    #[case::forward_wraps_to_top(at(9, 0, true), Direction::Forward, Some((0, true)))]
    #[case::forward_from_past_the_end(at(20, 0, false), Direction::Forward, Some((0, true)))]
    #[case::backward_previous(at(4, 2, true), Direction::Backward, Some((1, false)))]
    #[case::backward_skips_top_line(at(4, 0, false), Direction::Backward, Some((1, false)))]
    #[case::backward_wraps_to_bottom(at(1, 0, true), Direction::Backward, Some((3, true)))]
    fn find_next_match(
        #[case] cursor: Cursor,
        #[case] dir: Direction,
        #[case] expected: Option<(usize, bool)>,
    ) {
        assert_eq!(find(&MATCHES, cursor, dir), expected);
    }

    #[test]
    fn single_match_wraps_onto_itself() {
        let one = [m(3, 0)];
        assert_eq!(
            find(&one, at(3, 0, true), Direction::Forward),
            Some((0, true))
        );
        assert_eq!(
            find(&one, at(3, 0, true), Direction::Backward),
            Some((0, true))
        );
    }

    #[test]
    fn no_matches_finds_nothing() {
        assert_eq!(find(&[], at(0, 0, false), Direction::Forward), None);
    }

    #[test]
    fn collect_lists_every_match_in_order() {
        let re = compile("a").unwrap();
        assert_eq!(
            collect([(2, "banana"), (5, "xyz"), (7, "a")], &re),
            [
                Match {
                    row: 2,
                    start: 1,
                    end: 2
                },
                Match {
                    row: 2,
                    start: 3,
                    end: 4
                },
                Match {
                    row: 2,
                    start: 5,
                    end: 6
                },
                Match {
                    row: 7,
                    start: 0,
                    end: 1
                },
            ]
        );
    }

    #[test]
    fn reverse_and_leader() {
        assert_eq!(Direction::Forward.reverse(), Direction::Backward);
        assert_eq!(Direction::Backward.reverse(), Direction::Forward);
        assert_eq!(Direction::Forward.leader(), '/');
        assert_eq!(Direction::Backward.leader(), '?');
    }
}
