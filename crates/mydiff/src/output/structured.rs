//! Output for JSON/YAML/CSV comparisons.

use std::fmt::Write as _;

use mydiff_core::RowKind;
use mydiff_core::structured::{ChangeKind, StructDiff, describe};

use super::Theme;

fn style(kind: ChangeKind) -> anstyle::Style {
    Theme::row(match kind {
        ChangeKind::Added => RowKind::Added,
        ChangeKind::Removed => RowKind::Removed,
        ChangeKind::Changed => RowKind::Modified,
    })
}

/// Path-based change list (`~ $.a: 1 → 2`). Empty when identical.
pub fn changes(diff: &StructDiff, left: &str, right: &str, theme: Theme) -> String {
    if diff.is_identical() {
        return String::new();
    }
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{}",
        theme.paint(Theme::header(), &format!("--- {left}"))
    );
    let _ = writeln!(
        out,
        "{}",
        theme.paint(Theme::header(), &format!("+++ {right}"))
    );
    for c in &diff.changes {
        let _ = writeln!(out, "{}", theme.paint(style(c.kind), &describe(c, 120)));
    }
    out
}

/// One-line totals.
pub fn summary(diff: &StructDiff, format: &str, left: &str, right: &str) -> String {
    let s = diff.stats();
    if diff.is_identical() {
        format!("{left} vs {right}: identical {format}\n")
    } else {
        format!(
            "{left} vs {right}: {format}, {} added, {} removed, {} changed\n",
            s.added, s.removed, s.changed
        )
    }
}

#[cfg(test)]
mod tests {
    use mydiff_core::DiffOptions;
    use mydiff_core::structured::{Format, diff_structured};

    use super::*;

    #[test]
    fn change_list_and_summary() {
        let d = diff_structured(
            r#"{"a":1,"b":true}"#,
            r#"{"a":2,"c":null}"#,
            Format::Json,
            &DiffOptions::default(),
        )
        .unwrap();
        insta::assert_snapshot!(changes(&d, "l.json", "r.json", Theme::PLAIN), @r"
        --- l.json
        +++ r.json
        ~ $.a: 1 → 2
        - $.b: true
        + $.c: null
        ");
        assert_eq!(
            summary(&d, "json", "l", "r"),
            "l vs r: json, 1 added, 1 removed, 1 changed\n"
        );
        let same = StructDiff::default();
        assert_eq!(changes(&same, "l", "r", Theme::PLAIN), "");
        assert_eq!(summary(&same, "yaml", "l", "r"), "l vs r: identical yaml\n");
    }
}
