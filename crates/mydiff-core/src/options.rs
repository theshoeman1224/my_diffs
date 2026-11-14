//! User-facing comparison knobs.
//!
//! [`DiffOptions`] is plain data: it is what config files, fixture
//! `options.toml` files and CLI flags all deserialize into. Validation and
//! regex compilation happen later (see [`crate::normalize::LineNormalizer`]).

use serde::{Deserialize, Serialize};

/// How whitespace differences are treated when comparing lines.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WhitespaceMode {
    /// Whitespace is significant.
    #[default]
    Exact,
    /// Trailing whitespace is ignored.
    Trailing,
    /// Runs of whitespace compare equal to a single space; trailing whitespace is ignored
    /// (like `diff -b`).
    Change,
    /// All whitespace is ignored (like `diff -w`).
    All,
}

impl WhitespaceMode {
    /// The next mode in the cycle used by the TUI toggle key.
    #[must_use]
    pub fn cycle(self) -> Self {
        match self {
            Self::Exact => Self::Trailing,
            Self::Trailing => Self::Change,
            Self::Change => Self::All,
            Self::All => Self::Exact,
        }
    }

    /// Short label for status bars.
    pub fn label(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Trailing => "trailing",
            Self::Change => "change",
            Self::All => "all",
        }
    }
}

/// Line-diff algorithm.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Algorithm {
    /// Myers (default).
    #[default]
    Myers,
    /// Patience, often nicer for source code with moved blocks.
    Patience,
}

/// A regex substitution applied to each line before comparison, e.g. to blank out
/// timestamps. Displayed text is never altered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaskRule {
    /// Regex to search for.
    pub pattern: String,
    /// Replacement text (supports `$1`-style captures).
    #[serde(default)]
    pub replace: String,
}

/// Every comparison knob, for every mode. Unused fields are ignored by modes they
/// do not apply to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DiffOptions {
    // ---- text ----
    /// Whitespace handling.
    pub whitespace: WhitespaceMode,
    /// Compare case-insensitively.
    pub ignore_case: bool,
    /// Blank lines (after normalization) are not compared.
    pub ignore_blank_lines: bool,
    /// Treat CRLF and LF line endings as equal.
    pub ignore_line_endings: bool,
    /// Lines matching any of these regexes are not compared.
    pub ignore: Vec<String>,
    /// If non-empty, only lines matching at least one of these regexes are compared.
    pub only: Vec<String>,
    /// Substitutions applied before comparison.
    pub mask: Vec<MaskRule>,
    /// Unchanged lines of context around each hunk in unified output.
    pub context: usize,
    /// Minimum similarity (0.0–1.0) for a removed/added pair to be shown as one
    /// modified line instead of a removal plus an addition.
    pub modify_threshold: f32,
    /// Line-diff algorithm.
    pub algorithm: Algorithm,

    // ---- folder ----
    /// Glob patterns of relative paths to include (empty = everything).
    pub include: Vec<String>,
    /// Glob patterns of relative paths to exclude.
    pub exclude: Vec<String>,

    // ---- structured ----
    /// CSV: column names that identify a row. Empty = compare rows by position.
    pub key_columns: Vec<String>,
    /// JSON/YAML: paths to skip, e.g. `$.metadata.timestamp` or `$.items[*].id`.
    pub ignore_paths: Vec<String>,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            whitespace: WhitespaceMode::Exact,
            ignore_case: false,
            ignore_blank_lines: false,
            ignore_line_endings: true,
            ignore: Vec::new(),
            only: Vec::new(),
            mask: Vec::new(),
            context: 3,
            modify_threshold: 0.5,
            algorithm: Algorithm::Myers,
            include: Vec::new(),
            exclude: Vec::new(),
            key_columns: Vec::new(),
            ignore_paths: Vec::new(),
        }
    }
}

impl DiffOptions {
    /// Parse options from a TOML document whose top level is the options table
    /// (the format used by fixture `options.toml` files).
    pub fn from_toml(text: &str) -> crate::Result<Self> {
        toml::from_str(text).map_err(|e| crate::Error::Config(e.to_string()))
    }

    /// Check everything that can only be checked by compiling it (regexes, globs),
    /// so bad options fail before any input is read.
    pub fn validate(&self) -> crate::Result<()> {
        crate::filter::LineFilter::new(self)?;
        crate::folder::PathFilter::new(self)?;
        crate::structured::compile_patterns(self)?;
        if !(0.0..=f32::MAX).contains(&self.modify_threshold) {
            return Err(crate::Error::Config(format!(
                "modify_threshold must be >= 0, got {}",
                self.modify_threshold
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitespace_cycle_visits_every_mode_and_returns() {
        let mut m = WhitespaceMode::Exact;
        let mut seen = vec![m];
        for _ in 0..3 {
            m = m.cycle();
            seen.push(m);
        }
        assert_eq!(
            seen,
            [
                WhitespaceMode::Exact,
                WhitespaceMode::Trailing,
                WhitespaceMode::Change,
                WhitespaceMode::All
            ]
        );
        assert_eq!(m.cycle(), WhitespaceMode::Exact);
    }

    #[test]
    fn empty_toml_gives_defaults() {
        assert_eq!(DiffOptions::from_toml("").unwrap(), DiffOptions::default());
    }

    #[test]
    fn toml_overrides_selected_fields() {
        let o = DiffOptions::from_toml(
            r#"
            whitespace = "all"
            ignore = ['^#']
            [[mask]]
            pattern = '\d+'
            replace = "N"
            "#,
        )
        .unwrap();
        assert_eq!(o.whitespace, WhitespaceMode::All);
        assert_eq!(o.ignore, ["^#"]);
        assert_eq!(o.mask[0].replace, "N");
        assert_eq!(o.context, 3, "unspecified fields keep defaults");
    }

    #[test]
    fn validate_catches_bad_regex_and_threshold() {
        assert!(DiffOptions::default().validate().is_ok());
        let bad_regex = DiffOptions {
            mask: vec![MaskRule {
                pattern: "(".into(),
                replace: String::new(),
            }],
            ..DiffOptions::default()
        };
        assert!(
            bad_regex
                .validate()
                .unwrap_err()
                .to_string()
                .contains("mask pattern")
        );
        let bad_threshold = DiffOptions {
            modify_threshold: -1.0,
            ..DiffOptions::default()
        };
        assert!(bad_threshold.validate().is_err());
        let bad_glob = DiffOptions {
            exclude: vec!["[".into()],
            ..DiffOptions::default()
        };
        assert!(bad_glob.validate().is_err());
        let bad_path = DiffOptions {
            ignore_paths: vec!["no-dollar".into()],
            ..DiffOptions::default()
        };
        assert!(bad_path.validate().is_err());
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let err = DiffOptions::from_toml("ignore_whitespace = true").unwrap_err();
        assert!(err.to_string().contains("unknown field"), "{err}");
    }
}
