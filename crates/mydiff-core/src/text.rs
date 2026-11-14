//! Line-based text comparison.
//!
//! 1. Split both inputs into lines and normalize each into a comparison key.
//! 2. Lines excluded by filters become [`RowKind::Ignored`] and are not diffed.
//! 3. Remaining keys are diffed with `similar`.
//! 4. Each changed block (deletes + inserts between equal runs) is aligned by
//!    [`align_block`]: sufficiently similar line pairs become
//!    [`RowKind::Modified`] with inline highlights; the rest are removed/added.

use similar::{DiffOp, capture_diff_slices};

use crate::Result;
use crate::inline::{inline_changes, similarity};
use crate::model::{LineRef, Row, RowKind, TextDiff};
use crate::normalize::{LineNormalizer, NormalizedLine, display_text, split_lines};
use crate::options::{Algorithm, DiffOptions};

/// Blocks larger than this (old × new lines) are aligned positionally instead of
/// by the quadratic best-match search.
const MAX_ALIGN_CELLS: usize = 10_000;

/// Compare two texts.
pub fn diff_text(left: &str, right: &str, opts: &DiffOptions) -> Result<TextDiff> {
    let norm = LineNormalizer::new(opts)?;
    let l = Side::new(left, &norm);
    let r = Side::new(right, &norm);
    let lk = l.compared_keys();
    let rk = r.compared_keys();
    let alg = match opts.algorithm {
        Algorithm::Myers => similar::Algorithm::Myers,
        Algorithm::Patience => similar::Algorithm::Patience,
    };
    let ops = capture_diff_slices(alg, &lk, &rk);

    let mut b = Builder {
        rows: Vec::new(),
        l: &l,
        r: &r,
        li: 0,
        ri: 0,
        opts,
        norm: &norm,
    };
    let mut block: Option<(usize, usize, usize, usize)> = None;
    for op in ops {
        if let DiffOp::Equal {
            old_index,
            new_index,
            len,
        } = op
        {
            if let Some(blk) = block.take() {
                b.block(blk, &lk, &rk);
            }
            for t in 0..len {
                let (li, ri) = (l.compared[old_index + t], r.compared[new_index + t]);
                b.ignored_both(li, ri);
                b.push(RowKind::Equal, Some(li), Some(ri));
            }
        } else {
            let (o, n) = (op.old_range(), op.new_range());
            block = Some(match block {
                None => (o.start, o.end, n.start, n.end),
                Some((os, oe, ns, ne)) => (
                    os.min(o.start),
                    oe.max(o.end),
                    ns.min(n.start),
                    ne.max(n.end),
                ),
            });
        }
    }
    if let Some(blk) = block.take() {
        b.block(blk, &lk, &rk);
    }
    b.ignored_both(l.raw.len(), r.raw.len());
    Ok(TextDiff { rows: b.rows })
}

struct Side<'a> {
    raw: Vec<&'a str>,
    norm: Vec<NormalizedLine>,
    /// Raw indices of lines that take part in the comparison.
    compared: Vec<usize>,
}

impl<'a> Side<'a> {
    fn new(text: &'a str, n: &LineNormalizer) -> Self {
        let raw = split_lines(text);
        let norm: Vec<_> = raw.iter().map(|l| n.normalize(l)).collect();
        let compared = norm
            .iter()
            .enumerate()
            .filter(|(_, n)| n.compared)
            .map(|(i, _)| i)
            .collect();
        Self {
            raw,
            norm,
            compared,
        }
    }

    fn compared_keys(&self) -> Vec<&str> {
        self.compared
            .iter()
            .map(|&i| self.norm[i].key.as_str())
            .collect()
    }

    fn line(&self, i: usize) -> LineRef {
        LineRef::new(i, display_text(self.raw[i]))
    }
}

struct Builder<'s, 'a> {
    rows: Vec<Row>,
    l: &'s Side<'a>,
    r: &'s Side<'a>,
    /// Next raw left index not yet emitted.
    li: usize,
    /// Next raw right index not yet emitted.
    ri: usize,
    opts: &'s DiffOptions,
    norm: &'s LineNormalizer,
}

impl Builder<'_, '_> {
    fn push(&mut self, kind: RowKind, li: Option<usize>, ri: Option<usize>) {
        let left = li.map(|i| self.l.line(i));
        let right = ri.map(|i| self.r.line(i));
        if let Some(i) = li {
            self.li = i + 1;
        }
        if let Some(i) = ri {
            self.ri = i + 1;
        }
        self.rows.push(Row { kind, left, right });
    }

    /// Emit ignored lines before `lu`/`ru`, pairing them side by side where possible.
    fn ignored_both(&mut self, lu: usize, ru: usize) {
        while self.li < lu && self.ri < ru {
            self.push(RowKind::Ignored, Some(self.li), Some(self.ri));
        }
        self.ignored_left(lu);
        self.ignored_right(ru);
    }

    fn ignored_left(&mut self, until: usize) {
        while self.li < until {
            self.push(RowKind::Ignored, Some(self.li), None);
        }
    }

    fn ignored_right(&mut self, until: usize) {
        while self.ri < until {
            self.push(RowKind::Ignored, None, Some(self.ri));
        }
    }

    fn block(&mut self, (os, oe, ns, ne): (usize, usize, usize, usize), lk: &[&str], rk: &[&str]) {
        let lu = if os < oe {
            self.l.compared[os]
        } else {
            self.li
        };
        let ru = if ns < ne {
            self.r.compared[ns]
        } else {
            self.ri
        };
        self.ignored_both(lu, ru);
        for item in align_block(&lk[os..oe], &rk[ns..ne], self.opts.modify_threshold) {
            match item {
                BlockItem::Removed(i) => {
                    let li = self.l.compared[os + i];
                    self.ignored_left(li);
                    self.push(RowKind::Removed, Some(li), None);
                }
                BlockItem::Added(j) => {
                    let ri = self.r.compared[ns + j];
                    self.ignored_right(ri);
                    self.push(RowKind::Added, None, Some(ri));
                }
                BlockItem::Modified(i, j) => {
                    let (li, ri) = (self.l.compared[os + i], self.r.compared[ns + j]);
                    self.ignored_left(li);
                    self.ignored_right(ri);
                    self.push(RowKind::Modified, Some(li), Some(ri));
                    let row = self.rows.last_mut().expect("just pushed");
                    let (Some(left), Some(right)) = (row.left.as_mut(), row.right.as_mut()) else {
                        unreachable!("modified rows have both sides")
                    };
                    let (lc, rc) = inline_changes(
                        &left.text,
                        &right.text,
                        self.opts.whitespace,
                        self.opts.ignore_case,
                        self.norm.filter(),
                    );
                    left.changed = lc;
                    right.changed = rc;
                }
            }
        }
    }
}

/// How one line of a changed block is presented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockItem {
    /// Old line `i` was removed.
    Removed(usize),
    /// New line `j` was added.
    Added(usize),
    /// Old line `i` became new line `j`.
    Modified(usize, usize),
}

/// Align the old and new lines of one changed block.
///
/// Pairs lines in order (no crossing) to maximize total similarity, considering
/// only pairs with `similarity >= threshold`. Between pairs, removals are listed
/// before additions. Large blocks fall back to positional pairing.
pub fn align_block(old: &[&str], new: &[&str], threshold: f32) -> Vec<BlockItem> {
    let (m, n) = (old.len(), new.len());
    let pairs = if m * n > MAX_ALIGN_CELLS {
        (0..m.min(n))
            .filter(|&k| similarity(old[k], new[k]) >= threshold)
            .map(|k| (k, k))
            .collect()
    } else {
        best_pairs(old, new, threshold)
    };

    let mut out = Vec::with_capacity(m + n);
    let (mut i, mut j) = (0, 0);
    for (pi, pj) in pairs.into_iter().chain(std::iter::once((m, n))) {
        out.extend((i..pi).map(BlockItem::Removed));
        out.extend((j..pj).map(BlockItem::Added));
        if pi < m && pj < n {
            out.push(BlockItem::Modified(pi, pj));
        }
        (i, j) = (pi + 1, pj + 1);
    }
    out
}

/// Weighted LCS: ordered pairs maximizing the sum of similarities.
#[allow(
    clippy::float_cmp,
    reason = "backtracking compares values produced by the identical expressions"
)]
fn best_pairs(old: &[&str], new: &[&str], threshold: f32) -> Vec<(usize, usize)> {
    let (m, n) = (old.len(), new.len());
    let sim: Vec<Vec<f32>> = old
        .iter()
        .map(|a| new.iter().map(|b| similarity(a, b)).collect())
        .collect();
    // score[i][j] = best total for old[i..], new[j..]
    let mut score = vec![vec![0.0_f32; n + 1]; m + 1];
    for i in (0..m).rev() {
        for j in (0..n).rev() {
            let mut best = score[i + 1][j].max(score[i][j + 1]);
            if sim[i][j] >= threshold {
                best = best.max(sim[i][j] + score[i + 1][j + 1]);
            }
            score[i][j] = best;
        }
    }
    let mut pairs = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < m && j < n {
        if sim[i][j] >= threshold && score[i][j] == sim[i][j] + score[i + 1][j + 1] {
            pairs.push((i, j));
            i += 1;
            j += 1;
        } else if score[i][j] == score[i + 1][j] {
            i += 1;
        } else {
            j += 1;
        }
    }
    pairs
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;
    use crate::options::WhitespaceMode;
    use BlockItem::{Added as A, Modified as M, Removed as R};

    fn kinds(d: &TextDiff) -> Vec<RowKind> {
        d.rows.iter().map(|r| r.kind).collect()
    }

    #[rstest]
    #[case::pure_delete(&["a"], &[], &[R(0)])]
    #[case::pure_insert(&[], &["a"], &[A(0)])]
    #[case::one_modified(&["the quick fox"], &["the slow fox"], &[M(0, 0)])]
    #[case::dissimilar(&["aaaa"], &["zzzz"], &[R(0), A(0)])]
    #[case::best_match_not_first(
        &["fn main() {"],
        &["// new header", "fn main() -> () {"],
        &[A(0), M(0, 1)],
    )]
    #[case::removals_before_additions(&["x1", "keep me", "y1"], &["keep me!"], &[R(0), M(1, 0), R(2)])]
    fn align(#[case] old: &[&str], #[case] new: &[&str], #[case] expected: &[BlockItem]) {
        assert_eq!(align_block(old, new, 0.5), expected);
    }

    #[test]
    fn threshold_above_one_disables_modified() {
        assert_eq!(align_block(&["abc"], &["abd"], 1.1), [R(0), A(0)]);
    }

    #[rstest]
    #[case::identical("a\nb\n", "a\nb\n", &[RowKind::Equal, RowKind::Equal])]
    #[case::both_empty("", "", &[])]
    #[case::added_at_end("a\n", "a\nb\n", &[RowKind::Equal, RowKind::Added])]
    #[case::removed_at_start("x\na\n", "a\n", &[RowKind::Removed, RowKind::Equal])]
    #[case::modified("let x = 1;\n", "let x = 2;\n", &[RowKind::Modified])]
    #[case::trailing_newline_ignored("a", "a\n", &[RowKind::Equal])]
    #[case::crlf_equal_by_default("a\r\nb\r\n", "a\nb\n", &[RowKind::Equal, RowKind::Equal])]
    fn row_kinds(#[case] left: &str, #[case] right: &str, #[case] expected: &[RowKind]) {
        let d = diff_text(left, right, &DiffOptions::default()).unwrap();
        assert_eq!(kinds(&d), expected);
    }

    #[test]
    fn whitespace_all_makes_reformatted_lines_equal() {
        let opts = DiffOptions {
            whitespace: WhitespaceMode::All,
            ..DiffOptions::default()
        };
        let d = diff_text("if (a) {\n", "if(a){\n", &opts).unwrap();
        assert!(d.is_identical());
        assert_eq!(
            d.rows[0].left.as_ref().unwrap().text,
            "if (a) {",
            "display keeps original"
        );
    }

    #[test]
    fn ignored_lines_are_paired_and_not_diffed() {
        let opts = DiffOptions {
            ignore: vec!["^ts=".into()],
            ..DiffOptions::default()
        };
        let d = diff_text("ts=1\nbody\n", "ts=2\nbody\n", &opts).unwrap();
        assert_eq!(kinds(&d), [RowKind::Ignored, RowKind::Equal]);
        assert!(d.is_identical());
    }

    #[test]
    fn modified_rows_carry_inline_ranges() {
        let d = diff_text("the quick fox\n", "the slow fox\n", &DiffOptions::default()).unwrap();
        let row = &d.rows[0];
        assert_eq!(row.left.as_ref().unwrap().changed, [4..9]);
        assert_eq!(row.right.as_ref().unwrap().changed, [4..8]);
    }

    #[test]
    fn bad_regex_is_an_error() {
        let opts = DiffOptions {
            ignore: vec!["[".into()],
            ..DiffOptions::default()
        };
        assert!(diff_text("", "", &opts).is_err());
    }

    #[test]
    fn large_blocks_use_positional_fallback() {
        let old: Vec<String> = (0..200).map(|i| format!("old line {i}")).collect();
        let new: Vec<String> = (0..200).map(|i| format!("new line {i}")).collect();
        let o: Vec<&str> = old.iter().map(String::as_str).collect();
        let n: Vec<&str> = new.iter().map(String::as_str).collect();
        let items = align_block(&o, &n, 0.5);
        assert_eq!(items.len(), 200);
        assert!(items.iter().all(|it| matches!(it, M(i, j) if i == j)));
    }
}
