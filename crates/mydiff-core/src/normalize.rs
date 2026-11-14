//! Turning raw lines into comparison keys.
//!
//! The rule that keeps the whole tool predictable: **options change the comparison
//! key, never the displayed text.**

use crate::Result;
use crate::filter::LineFilter;
use crate::options::{DiffOptions, WhitespaceMode};

/// Split text into lines on `\n`. Each returned slice excludes the `\n` but keeps a
/// trailing `\r` if present. A final terminator does not produce an extra empty line.
pub fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let body = text.strip_suffix('\n').unwrap_or(text);
    body.split('\n').collect()
}

/// The text to show for a raw line: the line without its `\r` terminator.
pub fn display_text(raw: &str) -> &str {
    raw.strip_suffix('\r').unwrap_or(raw)
}

/// Apply a whitespace mode to a string.
pub fn normalize_whitespace(s: &str, mode: WhitespaceMode) -> String {
    match mode {
        WhitespaceMode::Exact => s.to_owned(),
        WhitespaceMode::Trailing => s.trim_end().to_owned(),
        WhitespaceMode::Change => {
            let mut out = String::with_capacity(s.len());
            let mut in_ws = false;
            for c in s.trim_end().chars() {
                if c.is_whitespace() {
                    if !in_ws {
                        out.push(' ');
                    }
                    in_ws = true;
                } else {
                    out.push(c);
                    in_ws = false;
                }
            }
            out
        }
        WhitespaceMode::All => s.chars().filter(|c| !c.is_whitespace()).collect(),
    }
}

/// A raw line after normalization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedLine {
    /// What equality is decided on.
    pub key: String,
    /// `false` if the line is filtered out (ignore/only/blank-line rules).
    pub compared: bool,
}

/// Compiled normalization pipeline for one set of options.
#[derive(Debug, Clone)]
pub struct LineNormalizer {
    filter: LineFilter,
    whitespace: WhitespaceMode,
    ignore_case: bool,
    ignore_blank_lines: bool,
    ignore_line_endings: bool,
}

impl LineNormalizer {
    /// Compile `opts` (fails on invalid regexes).
    pub fn new(opts: &DiffOptions) -> Result<Self> {
        Ok(Self {
            filter: LineFilter::new(opts)?,
            whitespace: opts.whitespace,
            ignore_case: opts.ignore_case,
            ignore_blank_lines: opts.ignore_blank_lines,
            ignore_line_endings: opts.ignore_line_endings,
        })
    }

    /// The compiled line filter (for mask-aware inline highlighting).
    pub fn filter(&self) -> &LineFilter {
        &self.filter
    }

    /// Pipeline: strip `\r` (if ignoring line endings) → masks → whitespace → case.
    /// Filters are evaluated on the displayed text.
    pub fn normalize(&self, raw: &str) -> NormalizedLine {
        let shown = display_text(raw);
        let base = if self.ignore_line_endings { shown } else { raw };
        let masked = self.filter.apply_masks(base);
        let mut key = normalize_whitespace(&masked, self.whitespace);
        if self.ignore_case {
            key = key.to_lowercase();
        }
        let blank = key.trim().is_empty();
        let compared = self.filter.is_compared(shown) && !(self.ignore_blank_lines && blank);
        NormalizedLine { key, compared }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::empty("", &[])]
    #[case::single_no_newline("a", &["a"])]
    #[case::single_newline("a\n", &["a"])]
    #[case::blank_line_kept("a\n\nb\n", &["a", "", "b"])]
    #[case::only_newline("\n", &[""])]
    #[case::crlf_kept("a\r\nb\r\n", &["a\r", "b\r"])]
    fn split(#[case] text: &str, #[case] expected: &[&str]) {
        assert_eq!(split_lines(text), expected);
    }

    #[rstest]
    #[case::exact(WhitespaceMode::Exact, " a  b \t", " a  b \t")]
    #[case::trailing(WhitespaceMode::Trailing, " a  b \t", " a  b")]
    #[case::change(WhitespaceMode::Change, " a  \t b \t", " a b")]
    #[case::all(WhitespaceMode::All, " a  b \t", "ab")]
    fn whitespace(#[case] mode: WhitespaceMode, #[case] input: &str, #[case] expected: &str) {
        assert_eq!(normalize_whitespace(input, mode), expected);
    }

    #[allow(clippy::needless_pass_by_value, reason = "terser call sites")]
    fn norm(opts: DiffOptions, raw: &str) -> NormalizedLine {
        LineNormalizer::new(&opts).unwrap().normalize(raw)
    }

    #[test]
    fn crlf_ignored_by_default() {
        assert_eq!(norm(DiffOptions::default(), "x\r").key, "x");
    }

    #[test]
    fn crlf_kept_when_line_endings_matter() {
        let opts = DiffOptions {
            ignore_line_endings: false,
            ..DiffOptions::default()
        };
        assert_eq!(norm(opts, "x\r").key, "x\r");
    }

    #[test]
    fn case_folding() {
        let opts = DiffOptions {
            ignore_case: true,
            ..DiffOptions::default()
        };
        assert_eq!(norm(opts, "HeLLo").key, "hello");
    }

    #[test]
    fn blank_lines_uncompared_only_when_requested() {
        assert!(norm(DiffOptions::default(), "   ").compared);
        let opts = DiffOptions {
            ignore_blank_lines: true,
            ..DiffOptions::default()
        };
        assert!(!norm(opts, "   ").compared);
    }

    #[test]
    fn mask_runs_before_whitespace_and_case() {
        let opts = DiffOptions {
            whitespace: WhitespaceMode::All,
            ignore_case: true,
            mask: vec![crate::options::MaskRule {
                pattern: r"\d+".into(),
                replace: "N".into(),
            }],
            ..DiffOptions::default()
        };
        assert_eq!(norm(opts, "At 10 : 30").key, "atn:n");
    }

    #[test]
    fn filters_see_display_text_not_masked_text() {
        let opts = DiffOptions {
            ignore: vec!["^secret$".into()],
            mask: vec![crate::options::MaskRule {
                pattern: "secret".into(),
                replace: "x".into(),
            }],
            ..DiffOptions::default()
        };
        assert!(!norm(opts, "secret\r").compared);
    }
}
