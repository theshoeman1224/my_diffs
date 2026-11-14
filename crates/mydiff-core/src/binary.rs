//! Byte-level comparison with a hex-dump view.
//!
//! Comparison is *aligned*: byte `i` on the left is compared with byte `i` on the
//! right. Inserted or deleted bytes therefore show as differences from that point
//! on (no shifting is detected in v1). Rows are computed on demand from the two
//! buffers, so large files cost only their own memory.

use std::fmt::Write as _;

use serde::Serialize;

/// Bytes per hex row.
pub const ROW_BYTES: usize = 16;

/// Both inputs and where they differ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HexDiff {
    pub left: Vec<u8>,
    pub right: Vec<u8>,
    /// Indices of rows containing at least one difference (ascending).
    pub changed_rows: Vec<usize>,
}

/// One row of up to [`ROW_BYTES`] bytes per side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HexRow<'a> {
    pub offset: usize,
    pub left: &'a [u8],
    pub right: &'a [u8],
    /// `diff[k]` is true when byte `k` of the row differs (or exists on one side only).
    pub diff: [bool; ROW_BYTES],
}

impl HexRow<'_> {
    pub fn is_change(&self) -> bool {
        self.diff.iter().any(|&d| d)
    }
}

/// Totals for summaries and JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct HexStats {
    pub left_len: usize,
    pub right_len: usize,
    /// Positions that differ, counting bytes present on one side only.
    pub differing_bytes: usize,
    pub first_difference: Option<usize>,
}

/// Compare two byte buffers.
pub fn diff_bytes(left: Vec<u8>, right: Vec<u8>) -> HexDiff {
    let rows = left.len().max(right.len()).div_ceil(ROW_BYTES);
    let mut changed_rows = Vec::new();
    for r in 0..rows {
        let span = r * ROW_BYTES..(r + 1) * ROW_BYTES;
        if slice(&left, span.clone()) != slice(&right, span) {
            changed_rows.push(r);
        }
    }
    HexDiff {
        left,
        right,
        changed_rows,
    }
}

fn slice(b: &[u8], r: std::ops::Range<usize>) -> &[u8] {
    &b[r.start.min(b.len())..r.end.min(b.len())]
}

impl HexDiff {
    pub fn is_identical(&self) -> bool {
        self.changed_rows.is_empty()
    }

    pub fn row_count(&self) -> usize {
        self.left.len().max(self.right.len()).div_ceil(ROW_BYTES)
    }

    pub fn row(&self, r: usize) -> HexRow<'_> {
        let span = r * ROW_BYTES..(r + 1) * ROW_BYTES;
        let left = slice(&self.left, span.clone());
        let right = slice(&self.right, span);
        let mut diff = [false; ROW_BYTES];
        for (k, d) in diff.iter_mut().enumerate() {
            *d = left.get(k) != right.get(k);
        }
        HexRow {
            offset: r * ROW_BYTES,
            left,
            right,
            diff,
        }
    }

    pub fn stats(&self) -> HexStats {
        let mut differing = 0;
        let mut first = None;
        for &r in &self.changed_rows {
            let row = self.row(r);
            for (k, &d) in row.diff.iter().enumerate() {
                if d {
                    differing += 1;
                    first.get_or_insert(row.offset + k);
                }
            }
        }
        HexStats {
            left_len: self.left.len(),
            right_len: self.right.len(),
            differing_bytes: differing,
            first_difference: first,
        }
    }

    /// Stable text used by snapshot tests: changed rows only, differing bytes in
    /// brackets, `..` for bytes missing on that side.
    pub fn render_review(&self) -> String {
        let mut out = String::new();
        for &r in &self.changed_rows {
            let row = self.row(r);
            let _ = writeln!(
                out,
                "{:08x} - {}",
                row.offset,
                review_bytes(row.left, &row.diff)
            );
            let _ = writeln!(
                out,
                "{:08x} + {}",
                row.offset,
                review_bytes(row.right, &row.diff)
            );
        }
        let s = self.stats();
        let _ = writeln!(
            out,
            "-- sizes {} vs {}, {} differing byte(s), first at {}",
            s.left_len,
            s.right_len,
            s.differing_bytes,
            s.first_difference
                .map_or_else(|| "-".to_owned(), |o| format!("0x{o:x}"))
        );
        out
    }
}

fn review_bytes(bytes: &[u8], diff: &[bool; ROW_BYTES]) -> String {
    let mut parts = Vec::new();
    for (k, &d) in diff.iter().enumerate() {
        let b = bytes
            .get(k)
            .map_or_else(|| "..".to_owned(), |b| format!("{b:02x}"));
        if k >= bytes.len() && !d {
            break;
        }
        parts.push(if d { format!("[{b}]") } else { b });
    }
    parts.join(" ")
}

/// Printable ASCII, `.` otherwise (for the text column of a hex dump).
pub fn ascii(b: u8) -> char {
    if b.is_ascii_graphic() || b == b' ' {
        char::from(b)
    } else {
        '.'
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical() {
        let d = diff_bytes(b"abc".to_vec(), b"abc".to_vec());
        assert!(d.is_identical());
        assert_eq!(d.stats().first_difference, None);
    }

    #[test]
    fn single_byte_change() {
        let d = diff_bytes(b"hello world".to_vec(), b"hello_world".to_vec());
        insta::assert_snapshot!(d.render_review(), @r"
        00000000 - 68 65 6c 6c 6f [20] 77 6f 72 6c 64
        00000000 + 68 65 6c 6c 6f [5f] 77 6f 72 6c 64
        -- sizes 11 vs 11, 1 differing byte(s), first at 0x5
        ");
    }

    #[test]
    fn length_difference_counts_missing_bytes() {
        let d = diff_bytes(vec![0; 18], vec![0; 16]);
        assert_eq!(d.changed_rows, [1]);
        let s = d.stats();
        assert_eq!((s.differing_bytes, s.first_difference), (2, Some(16)));
        insta::assert_snapshot!(d.render_review(), @r"
        00000010 - [00] [00]
        00000010 + [..] [..]
        -- sizes 18 vs 16, 2 differing byte(s), first at 0x10
        ");
    }

    #[test]
    fn rows_are_aligned_and_only_changed_rows_listed() {
        let mut a = vec![0u8; 64];
        let mut b = a.clone();
        a[20] = 1;
        b[50] = 2;
        let d = diff_bytes(a, b);
        assert_eq!(d.changed_rows, [1, 3]);
        assert_eq!(d.row_count(), 4);
        assert_eq!(d.row(3).offset, 48);
    }

    #[test]
    fn empty_inputs() {
        let d = diff_bytes(Vec::new(), Vec::new());
        assert!(d.is_identical());
        assert_eq!(d.row_count(), 0);
        let d = diff_bytes(Vec::new(), vec![1]);
        assert_eq!(d.changed_rows, [0]);
    }

    #[test]
    fn ascii_column() {
        let s: String = b"Az 0~\x00\x7f\xff".iter().map(|&b| ascii(b)).collect();
        assert_eq!(s, "Az 0~...");
    }
}
