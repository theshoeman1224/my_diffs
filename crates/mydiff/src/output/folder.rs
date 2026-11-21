//! Folder comparison output.

use std::path::Path;

use anstyle::{AnsiColor, Effects, Style};
use mydiff_core::folder::{EntryStatus, FolderDiff, FolderStats};
use mydiff_core::{DiffOptions, RowKind, detect::looks_binary, diff_text};

use super::{Theme, unified};

fn status_style(s: &EntryStatus) -> Style {
    match s {
        EntryStatus::Same => Theme::dim(),
        EntryStatus::Different => Theme::row(RowKind::Modified),
        EntryStatus::OnlyLeft => Theme::row(RowKind::Removed),
        EntryStatus::OnlyRight => Theme::row(RowKind::Added),
        EntryStatus::TypeMismatch => AnsiColor::Magenta.on_default(),
        EntryStatus::Error(_) => AnsiColor::Red.on_default().effects(Effects::BOLD),
    }
}

/// `<marker> path` per entry (`-` only left, `+` only right, `~` different,
/// `!` file vs directory, `?` unreadable). Same files only if `show_same`.
pub fn listing(diff: &FolderDiff, show_same: bool, theme: Theme) -> String {
    let mut out = String::new();
    for e in &diff.entries {
        if !show_same && e.status == EntryStatus::Same {
            continue;
        }
        let slash = if e.is_dir { "/" } else { "" };
        let mut line = format!("{} {}{slash}", e.status.marker(), e.path);
        if let EntryStatus::Error(msg) = &e.status {
            line.push_str(&format!(": {msg}"));
        }
        out.push_str(&theme.paint(status_style(&e.status), &line));
        out.push('\n');
    }
    out
}

/// One-line totals.
pub fn summary_line(left: &str, right: &str, s: &FolderStats) -> String {
    let mut parts = Vec::new();
    for (n, what) in [
        (s.different, "different"),
        (s.only_left, "only in left"),
        (s.only_right, "only in right"),
        (s.type_mismatch, "file/dir mismatch"),
        (s.errors, "unreadable"),
    ] {
        if n > 0 {
            parts.push(format!("{n} {what}"));
        }
    }
    let body = if parts.is_empty() {
        "identical".to_owned()
    } else {
        parts.join(", ")
    };
    format!("{left} vs {right}: {body} ({} same)\n", s.same)
}

/// `diff -r`-style output: a unified diff per differing text file, and one line
/// per other difference.
///
/// Files are read under `roots`; `labels` name the roots in the output (they
/// are usually the same paths, or `a/` and `b/` under git).
pub fn unified_all(
    diff: &FolderDiff,
    [left, right]: [&Path; 2],
    [left_label, right_label]: [&str; 2],
    opts: &DiffOptions,
    theme: Theme,
    read: impl Fn(&Path) -> std::io::Result<Vec<u8>>,
) -> String {
    let mut out = String::new();
    let lp = |rel: &str| Path::new(left_label).join(rel).display().to_string();
    let rp = |rel: &str| Path::new(right_label).join(rel).display().to_string();
    let (left_name, right_name) = (
        left_label.trim_end_matches('/'),
        right_label.trim_end_matches('/'),
    );
    for e in &diff.entries {
        let p = e.path.as_str();
        let line = match &e.status {
            EntryStatus::Same => continue,
            EntryStatus::OnlyLeft => format!("Only in {left_name}: {p}"),
            EntryStatus::OnlyRight => format!("Only in {right_name}: {p}"),
            EntryStatus::TypeMismatch => {
                format!("{} and {} are a file and a directory", lp(p), rp(p))
            }
            EntryStatus::Error(msg) => format!("Cannot compare {p}: {msg}"),
            EntryStatus::Different => match (read(&left.join(p)), read(&right.join(p))) {
                (Ok(a), Ok(b)) if !looks_binary(&a) && !looks_binary(&b) => {
                    let (a, b) = (String::from_utf8_lossy(&a), String::from_utf8_lossy(&b));
                    match diff_text(&a, &b, opts) {
                        Ok(d) => {
                            out.push_str(&unified::render(&d, &lp(p), &rp(p), opts.context, theme));
                            continue;
                        }
                        Err(e) => format!("Cannot compare {p}: {e}"),
                    }
                }
                (Ok(_), Ok(_)) => format!("Binary files {} and {} differ", lp(p), rp(p)),
                (Err(e), _) | (_, Err(e)) => format!("Cannot compare {p}: {e}"),
            },
        };
        out.push_str(&theme.paint(status_style(&e.status), &line));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use mydiff_core::folder::FolderEntry;

    use super::*;

    fn entry(path: &str, status: EntryStatus) -> FolderEntry {
        FolderEntry {
            path: path.into(),
            is_dir: false,
            status,
        }
    }

    fn sample() -> FolderDiff {
        FolderDiff {
            entries: vec![
                entry("a.txt", EntryStatus::Different),
                entry("bin", EntryStatus::Different),
                entry("gone", EntryStatus::OnlyLeft),
                entry("new", EntryStatus::OnlyRight),
                entry("same", EntryStatus::Same),
                entry("x", EntryStatus::TypeMismatch),
                entry("y", EntryStatus::Error("permission denied".into())),
            ],
        }
    }

    #[test]
    fn listing_hides_same_by_default() {
        insta::assert_snapshot!(listing(&sample(), false, Theme::PLAIN), @r"
        ~ a.txt
        ~ bin
        - gone
        + new
        ! x
        ? y: permission denied
        ");
        assert!(listing(&sample(), true, Theme::PLAIN).contains("  same\n"));
    }

    #[test]
    fn summary_counts() {
        let s = sample().stats();
        assert_eq!(
            summary_line("L", "R", &s),
            "L vs R: 2 different, 1 only in left, 1 only in right, 1 file/dir mismatch, 1 unreadable (1 same)\n"
        );
        assert_eq!(
            summary_line("L", "R", &FolderStats::default()),
            "L vs R: identical (0 same)\n"
        );
    }

    #[test]
    fn unified_all_mixes_patches_and_notes() {
        let read = |p: &Path| -> std::io::Result<Vec<u8>> {
            Ok(match p.to_str().unwrap() {
                "L/a.txt" => b"one\n".to_vec(),
                "R/a.txt" => b"two\n".to_vec(),
                "L/bin" => b"\0\x01".to_vec(),
                "R/bin" => b"\0\x02".to_vec(),
                _ => return Err(std::io::Error::other("unexpected read")),
            })
        };
        let out = unified_all(
            &sample(),
            [Path::new("L"), Path::new("R")],
            ["L", "R"],
            &DiffOptions::default(),
            Theme::PLAIN,
            read,
        );
        insta::assert_snapshot!(out, @r"
        --- L/a.txt
        +++ R/a.txt
        @@ -1,1 +1,1 @@
        -one
        +two
        Binary files L/bin and R/bin differ
        Only in L: gone
        Only in R: new
        L/x and R/x are a file and a directory
        Cannot compare y: permission denied
        ");
    }

    #[test]
    fn unified_all_names_files_by_label_but_reads_roots() {
        let read = |p: &Path| -> std::io::Result<Vec<u8>> {
            Ok(match p.to_str().unwrap() {
                "/tmp/x/left/a.txt" => b"one\n".to_vec(),
                "/tmp/x/right/a.txt" => b"two\n".to_vec(),
                _ => b"\0".to_vec(),
            })
        };
        let out = unified_all(
            &sample(),
            [Path::new("/tmp/x/left"), Path::new("/tmp/x/right")],
            ["a/", "b/"],
            &DiffOptions::default(),
            Theme::PLAIN,
            read,
        );
        assert!(out.starts_with("--- a/a.txt\n+++ b/a.txt\n"), "{out}");
        assert!(
            out.contains("Only in a: gone\n") && out.contains("Only in b: new\n"),
            "{out}"
        );
        assert!(!out.contains("/tmp/x"), "{out}");
    }
}
