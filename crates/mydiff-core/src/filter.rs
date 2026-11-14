//! Regex-driven line filters: `ignore` (blacklist), `only` (whitelist) and `mask`
//! (substitute before comparing).

use std::borrow::Cow;

use regex::Regex;

use crate::options::{DiffOptions, MaskRule};
use crate::{Error, Result};

/// Compiled form of the `ignore`, `only` and `mask` options.
#[derive(Debug, Clone, Default)]
pub struct LineFilter {
    ignore: Vec<Regex>,
    only: Vec<Regex>,
    masks: Vec<(Regex, String)>,
}

fn compile(kind: &'static str, pattern: &str) -> Result<Regex> {
    Regex::new(pattern).map_err(|source| Error::Pattern {
        kind,
        pattern: pattern.to_owned(),
        source: Box::new(source),
    })
}

impl LineFilter {
    /// Compile the filter-related fields of `opts`.
    pub fn new(opts: &DiffOptions) -> Result<Self> {
        Ok(Self {
            ignore: opts
                .ignore
                .iter()
                .map(|p| compile("ignore", p))
                .collect::<Result<_>>()?,
            only: opts
                .only
                .iter()
                .map(|p| compile("only", p))
                .collect::<Result<_>>()?,
            masks: opts
                .mask
                .iter()
                .map(|MaskRule { pattern, replace }| {
                    Ok((compile("mask", pattern)?, replace.clone()))
                })
                .collect::<Result<_>>()?,
        })
    }

    /// Should this line take part in the comparison at all?
    ///
    /// A line is excluded if it matches any `ignore` pattern, or if `only` patterns
    /// exist and it matches none of them. Patterns are matched against the original
    /// (unmasked) line text.
    pub fn is_compared(&self, line: &str) -> bool {
        if self.ignore.iter().any(|r| r.is_match(line)) {
            return false;
        }
        self.only.is_empty() || self.only.iter().any(|r| r.is_match(line))
    }

    /// Non-overlapping regions of `line` matched by mask rules, sorted by position,
    /// each with its expanded replacement. Earlier rules win on overlap.
    pub fn mask_spans(&self, line: &str) -> Vec<(std::ops::Range<usize>, String)> {
        let mut spans: Vec<(std::ops::Range<usize>, String)> = Vec::new();
        for (re, replacement) in &self.masks {
            for caps in re.captures_iter(line) {
                let Some(m) = caps.get(0) else { continue };
                let r = m.range();
                if r.is_empty()
                    || spans
                        .iter()
                        .any(|(s, _)| s.start < r.end && r.start < s.end)
                {
                    continue;
                }
                let mut key = String::new();
                caps.expand(replacement, &mut key);
                spans.push((r, key));
            }
        }
        spans.sort_by_key(|(r, _)| r.start);
        spans
    }

    /// Apply every mask rule in order.
    pub fn apply_masks<'a>(&self, line: &'a str) -> Cow<'a, str> {
        let mut out = Cow::Borrowed(line);
        for (re, replacement) in &self.masks {
            if let Cow::Owned(s) = re.replace_all(&out, replacement.as_str()) {
                out = Cow::Owned(s);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn filter(ignore: &[&str], only: &[&str], mask: &[(&str, &str)]) -> LineFilter {
        LineFilter::new(&DiffOptions {
            ignore: ignore.iter().map(ToString::to_string).collect(),
            only: only.iter().map(ToString::to_string).collect(),
            mask: mask
                .iter()
                .map(|(p, r)| MaskRule {
                    pattern: (*p).to_owned(),
                    replace: (*r).to_owned(),
                })
                .collect(),
            ..DiffOptions::default()
        })
        .unwrap()
    }

    #[rstest]
    #[case::no_filters(&[], &[], "anything", true)]
    #[case::ignored(&["^#"], &[], "# comment", false)]
    #[case::not_ignored(&["^#"], &[], "code # trailing", true)]
    #[case::only_match(&[], &["ERROR"], "ERROR boom", true)]
    #[case::only_miss(&[], &["ERROR"], "INFO fine", false)]
    #[case::ignore_beats_only(&["boom"], &["ERROR"], "ERROR boom", false)]
    #[case::any_only_pattern(&[], &["ERROR", "WARN"], "WARN hmm", true)]
    fn is_compared(
        #[case] ignore: &[&str],
        #[case] only: &[&str],
        #[case] line: &str,
        #[case] expected: bool,
    ) {
        assert_eq!(filter(ignore, only, &[]).is_compared(line), expected);
    }

    #[rstest]
    #[case::none(&[], "a 1 b", "a 1 b")]
    #[case::digits(&[(r"\d+", "N")], "id 42 at 7", "id N at N")]
    #[case::captures(&[(r"(\w+)=\d+", "$1=?")], "x=1 y=22", "x=? y=?")]
    #[case::chained(&[(r"\d", "D"), ("D+", "<n>")], "v12", "v<n>")]
    fn masks(#[case] mask: &[(&str, &str)], #[case] line: &str, #[case] expected: &str) {
        assert_eq!(filter(&[], &[], mask).apply_masks(line), expected);
    }

    #[test]
    fn mask_spans_are_sorted_and_non_overlapping() {
        let f = filter(&[], &[], &[(r"b+", "B"), (r"\w+", "W$0")]);
        let spans = f.mask_spans("aa bb cc");
        let got: Vec<_> = spans.iter().map(|(r, k)| (r.clone(), k.as_str())).collect();
        assert_eq!(got, [(0..2, "Waa"), (3..5, "B"), (6..8, "Wcc")]);
    }

    #[test]
    fn bad_pattern_names_the_option() {
        let err = LineFilter::new(&DiffOptions {
            only: vec!["(".into()],
            ..DiffOptions::default()
        })
        .unwrap_err();
        assert!(
            err.to_string().starts_with("invalid only pattern `(`"),
            "{err}"
        );
    }
}
