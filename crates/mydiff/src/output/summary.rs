//! One-line human summaries.

use mydiff_core::Stats;

/// e.g. `a.txt vs b.txt: 1 removed, 2 added, 1 modified (10 equal)`.
pub fn text(left: &str, right: &str, s: &Stats) -> String {
    let changes = s.removed + s.added + s.modified;
    let mut out = format!("{left} vs {right}: ");
    if changes == 0 {
        out.push_str("identical");
    } else {
        out.push_str(&format!(
            "{} removed, {} added, {} modified",
            s.removed, s.added, s.modified
        ));
    }
    out.push_str(&format!(" ({} equal", s.equal));
    if s.ignored > 0 {
        out.push_str(&format!(", {} ignored", s.ignored));
    }
    out.push_str(")\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical() {
        let s = Stats {
            equal: 3,
            ..Stats::default()
        };
        assert_eq!(text("a", "b", &s), "a vs b: identical (3 equal)\n");
    }

    #[test]
    fn different_with_ignored() {
        let s = Stats {
            equal: 1,
            removed: 2,
            added: 3,
            modified: 4,
            ignored: 5,
        };
        assert_eq!(
            text("a", "b", &s),
            "a vs b: 2 removed, 3 added, 4 modified (1 equal, 5 ignored)\n"
        );
    }
}
