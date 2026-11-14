//! The result of a text comparison, shaped for side-by-side display.

use std::fmt::Write as _;
use std::ops::Range;

use serde::Serialize;

/// What happened to a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RowKind {
    /// Same on both sides (after normalization).
    Equal,
    /// Only on the left (red).
    Removed,
    /// Only on the right (green).
    Added,
    /// On both sides but altered (yellow); see [`LineRef::changed`].
    Modified,
    /// Excluded from comparison by `ignore`/`only`/blank-line rules (dimmed).
    Ignored,
}

impl RowKind {
    /// Single-character gutter marker, so output reads correctly without color.
    pub fn marker(self) -> char {
        match self {
            Self::Equal => ' ',
            Self::Removed => '-',
            Self::Added => '+',
            Self::Modified => '~',
            Self::Ignored => '.',
        }
    }

    /// Is this row a difference the user cares about?
    pub fn is_change(self) -> bool {
        matches!(self, Self::Removed | Self::Added | Self::Modified)
    }
}

/// One side of a row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LineRef {
    /// 1-based line number in the source.
    pub number: usize,
    /// Displayed text (no line terminator).
    pub text: String,
    /// Byte ranges of `text` that differ from the other side. Only set on
    /// [`RowKind::Modified`] rows.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub changed: Vec<Range<usize>>,
}

impl LineRef {
    pub(crate) fn new(index: usize, text: &str) -> Self {
        Self {
            number: index + 1,
            text: text.to_owned(),
            changed: Vec::new(),
        }
    }
}

/// One display row: a left line, a right line, or both.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Row {
    /// Classification.
    pub kind: RowKind,
    /// Left line, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<LineRef>,
    /// Right line, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<LineRef>,
}

/// Counts per row kind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Stats {
    /// Equal rows.
    pub equal: usize,
    /// Removed rows.
    pub removed: usize,
    /// Added rows.
    pub added: usize,
    /// Modified rows.
    pub modified: usize,
    /// Ignored lines (counted per side).
    pub ignored: usize,
}

/// A run of rows to show together in unified output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hunk {
    /// Row indices covered (context included).
    pub rows: Range<usize>,
}

/// Full result of comparing two texts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TextDiff {
    /// Every line of both inputs, in display order.
    pub rows: Vec<Row>,
}

impl TextDiff {
    /// `true` if there are no removed, added or modified rows.
    pub fn is_identical(&self) -> bool {
        !self.rows.iter().any(|r| r.kind.is_change())
    }

    /// Counts per kind.
    pub fn stats(&self) -> Stats {
        let mut s = Stats::default();
        for r in &self.rows {
            match r.kind {
                RowKind::Equal => s.equal += 1,
                RowKind::Removed => s.removed += 1,
                RowKind::Added => s.added += 1,
                RowKind::Modified => s.modified += 1,
                RowKind::Ignored => {
                    s.ignored += usize::from(r.left.is_some()) + usize::from(r.right.is_some());
                }
            }
        }
        s
    }

    /// Indices of the first row of each contiguous block of changes. Used for
    /// "next/previous change" navigation.
    pub fn change_starts(&self) -> Vec<usize> {
        let mut out = Vec::new();
        let mut prev_change = false;
        for (i, r) in self.rows.iter().enumerate() {
            let change = r.kind.is_change();
            if change && !prev_change {
                out.push(i);
            }
            prev_change = change;
        }
        out
    }

    /// Group change rows into hunks with `context` rows of surrounding context.
    /// Hunks whose context would touch or overlap are merged.
    pub fn hunks(&self, context: usize) -> Vec<Hunk> {
        let mut hunks: Vec<Hunk> = Vec::new();
        for (i, r) in self.rows.iter().enumerate() {
            if !r.kind.is_change() {
                continue;
            }
            let start = i.saturating_sub(context);
            let end = (i + 1 + context).min(self.rows.len());
            match hunks.last_mut() {
                Some(h) if start <= h.rows.end => h.rows.end = end,
                _ => hunks.push(Hunk { rows: start..end }),
            }
        }
        hunks
    }

    /// Left-side lines in order (reconstructs the left input's displayed lines).
    pub fn left_lines(&self) -> impl Iterator<Item = &str> {
        self.rows
            .iter()
            .filter_map(|r| r.left.as_ref().map(|l| l.text.as_str()))
    }

    /// Right-side lines in order.
    pub fn right_lines(&self) -> impl Iterator<Item = &str> {
        self.rows
            .iter()
            .filter_map(|r| r.right.as_ref().map(|l| l.text.as_str()))
    }

    /// Stable, human-readable rendering used by snapshot tests.
    ///
    /// ```text
    ///    1    1   unchanged
    ///    2      - removed
    ///         2 + added
    ///    3      ~ the [-quick-] fox
    ///         3 ~ the [+slow+] fox
    ///    4      . ignored
    /// ```
    pub fn render_review(&self) -> String {
        fn num(n: Option<&LineRef>) -> String {
            n.map_or_else(|| "    ".to_owned(), |l| format!("{:>4}", l.number))
        }
        let mut out = String::new();
        for r in &self.rows {
            let m = r.kind.marker();
            match (r.kind, &r.left, &r.right) {
                (RowKind::Modified, Some(l), Some(rt)) => {
                    let _ = writeln!(out, "{}      {m} {}", num(Some(l)), mark(l, "[-", "-]"));
                    let _ = writeln!(out, "     {} {m} {}", num(Some(rt)), mark(rt, "{+", "+}"));
                }
                (_, l, rt) if r.kind == RowKind::Ignored && l.is_some() && rt.is_some() => {
                    // Show both sides of a paired ignored row separately: texts may differ.
                    let (l, rt) = (l.as_ref(), rt.as_ref());
                    let _ = writeln!(out, "{}      {m} {}", num(l), l.map_or("", |x| &x.text));
                    let _ = writeln!(out, "     {} {m} {}", num(rt), rt.map_or("", |x| &x.text));
                }
                (_, l, rt) => {
                    let text = l.as_ref().or(rt.as_ref()).map_or("", |x| x.text.as_str());
                    let _ = writeln!(out, "{} {} {m} {text}", num(l.as_ref()), num(rt.as_ref()));
                }
            }
        }
        out
    }
}

/// Wrap changed ranges of a line in open/close markers.
fn mark(line: &LineRef, open: &str, close: &str) -> String {
    let mut out = String::with_capacity(line.text.len() + 8);
    let mut pos = 0;
    for r in &line.changed {
        out.push_str(&line.text[pos..r.start]);
        out.push_str(open);
        out.push_str(&line.text[r.clone()]);
        out.push_str(close);
        pos = r.end;
    }
    out.push_str(&line.text[pos..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(kind: RowKind) -> Row {
        Row {
            kind,
            left: None,
            right: None,
        }
    }

    fn diff(kinds: &[RowKind]) -> TextDiff {
        TextDiff {
            rows: kinds.iter().copied().map(row).collect(),
        }
    }

    use RowKind::{Added as A, Equal as E, Ignored as I, Modified as M, Removed as R};

    #[test]
    fn identical_when_no_changes() {
        assert!(diff(&[E, E, I]).is_identical());
        assert!(!diff(&[E, M]).is_identical());
    }

    #[test]
    fn change_starts_mark_block_beginnings() {
        assert_eq!(diff(&[E, R, A, E, M, E, A]).change_starts(), [1, 4, 6]);
    }

    #[test]
    fn hunks_merge_when_context_overlaps() {
        let d = diff(&[E, E, R, E, E, E, A, E, E, E, E, E, M]);
        let ranges: Vec<_> = d.hunks(1).into_iter().map(|h| h.rows).collect();
        assert_eq!(ranges, [1..4, 5..8, 11..13]);
        let ranges: Vec<_> = d.hunks(2).into_iter().map(|h| h.rows).collect();
        assert_eq!(ranges, [0..9, 10..13]);
    }

    #[test]
    fn hunks_with_zero_context() {
        let d = diff(&[R, A, E, M]);
        let ranges: Vec<_> = d.hunks(0).into_iter().map(|h| h.rows).collect();
        assert_eq!(ranges, [0..2, 3..4]);
    }

    #[test]
    fn mark_wraps_ranges() {
        let l = LineRef {
            number: 1,
            text: "the quick fox".into(),
            changed: vec![4..9],
        };
        assert_eq!(mark(&l, "[", "]"), "the [quick] fox");
    }
}
