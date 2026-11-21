//! Folder comparison output.

use std::path::Path;

use anstyle::{AnsiColor, Effects, Style};
use mydiff_core::folder::{EntryStatus, FolderDiff, FolderEntry, FolderStats};
use mydiff_core::{DiffOptions, RowKind, detect::looks_binary, diff_text};

use super::{Theme, unified};

const DEV_NULL: &str = "/dev/null";

fn entry_style(e: &FolderEntry) -> Style {
    match &e.status {
        EntryStatus::Same if e.renamed_from.is_some() => AnsiColor::Cyan.on_default(),
        EntryStatus::Same => Theme::dim(),
        EntryStatus::Different => Theme::row(RowKind::Modified),
        EntryStatus::OnlyLeft => Theme::row(RowKind::Removed),
        EntryStatus::OnlyRight => Theme::row(RowKind::Added),
        EntryStatus::TypeMismatch => AnsiColor::Magenta.on_default(),
        EntryStatus::Error(_) => AnsiColor::Red.on_default().effects(Effects::BOLD),
    }
}

/// `<marker> path` per entry (`-` only left, `+` only right, `~` different,
/// `!` file vs directory, `?` unreadable, `>` renamed). A rename reads
/// `old -> new`. Same files only if `show_same`.
pub fn listing(diff: &FolderDiff, show_same: bool, theme: Theme) -> String {
    let mut out = String::new();
    for e in &diff.entries {
        if !show_same && !e.is_change() {
            continue;
        }
        let mut line = format!("{} {}", e.marker(), e.display_path());
        if let EntryStatus::Error(msg) = &e.status {
            line.push_str(&format!(": {msg}"));
        }
        out.push_str(&theme.paint(entry_style(e), &line));
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
        (s.renamed, "renamed"),
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
/// per other difference. A renamed file is diffed from its old path to its new one.
///
/// Files are read under `roots`; `labels` name the roots in the output (they
/// are usually the same paths, or `a/` and `b/` under git). With
/// `whole_files`, a file on one side only is shown in full against
/// `/dev/null`, as git shows an added or deleted file.
pub fn unified_all(
    diff: &FolderDiff,
    [left, right]: [&Path; 2],
    [left_label, right_label]: [&str; 2],
    opts: &DiffOptions,
    theme: Theme,
    whole_files: bool,
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
        let (p, old) = (e.path.as_str(), e.left_path());
        let line = match &e.status {
            EntryStatus::Same if e.renamed_from.is_some() => {
                format!("Renamed {} to {}", lp(old), rp(p))
            }
            EntryStatus::Same => continue,
            EntryStatus::OnlyLeft | EntryStatus::OnlyRight if whole_files && !e.is_dir => {
                let added = e.status == EntryStatus::OnlyRight;
                let (path, name) = if added {
                    (right.join(p), rp(p))
                } else {
                    (left.join(p), lp(p))
                };
                match read(&path) {
                    Ok(b) if !looks_binary(&b) => {
                        let text = String::from_utf8_lossy(&b);
                        let (a, b) = if added { ("", &*text) } else { (&*text, "") };
                        let (la, lb) = if added {
                            (DEV_NULL, name.as_str())
                        } else {
                            (name.as_str(), DEV_NULL)
                        };
                        match diff_text(a, b, opts) {
                            Ok(d) => {
                                out.push_str(&unified::render(&d, la, lb, opts.context, theme));
                                continue;
                            }
                            Err(e) => format!("Cannot compare {p}: {e}"),
                        }
                    }
                    Ok(_) if added => format!("Binary file {name} added"),
                    Ok(_) => format!("Binary file {name} deleted"),
                    Err(e) => format!("Cannot compare {p}: {e}"),
                }
            }
            EntryStatus::OnlyLeft => format!("Only in {left_name}: {p}"),
            EntryStatus::OnlyRight => format!("Only in {right_name}: {p}"),
            EntryStatus::TypeMismatch => {
                format!("{} and {} are a file and a directory", lp(p), rp(p))
            }
            EntryStatus::Error(msg) => format!("Cannot compare {p}: {msg}"),
            EntryStatus::Different => match (read(&left.join(old)), read(&right.join(p))) {
                (Ok(a), Ok(b)) if !looks_binary(&a) && !looks_binary(&b) => {
                    let (a, b) = (String::from_utf8_lossy(&a), String::from_utf8_lossy(&b));
                    match diff_text(&a, &b, opts) {
                        Ok(d) => {
                            out.push_str(&unified::render(
                                &d,
                                &lp(old),
                                &rp(p),
                                opts.context,
                                theme,
                            ));
                            continue;
                        }
                        Err(e) => format!("Cannot compare {p}: {e}"),
                    }
                }
                (Ok(_), Ok(_)) => format!("Binary files {} and {} differ", lp(old), rp(p)),
                (Err(e), _) | (_, Err(e)) => format!("Cannot compare {p}: {e}"),
            },
        };
        out.push_str(&theme.paint(entry_style(e), &line));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use mydiff_core::folder::FolderEntry;

    use super::*;

    fn entry(path: &str, status: EntryStatus) -> FolderEntry {
        FolderEntry::new(path, false, status)
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
            false,
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
            false,
            read,
        );
        assert!(out.starts_with("--- a/a.txt\n+++ b/a.txt\n"), "{out}");
        assert!(
            out.contains("Only in a: gone\n") && out.contains("Only in b: new\n"),
            "{out}"
        );
        assert!(!out.contains("/tmp/x"), "{out}");
    }

    fn renamed(from: &str, to: &str, status: EntryStatus) -> FolderEntry {
        FolderEntry {
            renamed_from: Some(from.into()),
            ..entry(to, status)
        }
    }

    fn with_renames() -> FolderDiff {
        FolderDiff {
            entries: vec![
                renamed("LICENSE", "COPYING", EntryStatus::Same),
                entry("same", EntryStatus::Same),
                renamed("util.rs", "src/util.rs", EntryStatus::Different),
            ],
        }
    }

    #[test]
    fn renames_list_old_and_new_path_even_when_same_is_hidden() {
        insta::assert_snapshot!(listing(&with_renames(), false, Theme::PLAIN), @r"
        > LICENSE -> COPYING
        ~ util.rs -> src/util.rs
        ");
        assert_eq!(
            summary_line("L", "R", &with_renames().stats()),
            "L vs R: 1 different, 2 renamed (2 same)\n"
        );
    }

    #[test]
    fn unified_all_diffs_a_rename_from_old_path_to_new_path() {
        let read = |p: &Path| -> std::io::Result<Vec<u8>> {
            Ok(match p.to_str().unwrap() {
                "L/util.rs" => b"one\n".to_vec(),
                "R/src/util.rs" => b"two\n".to_vec(),
                other => return Err(std::io::Error::other(format!("unexpected read {other}"))),
            })
        };
        let out = unified_all(
            &with_renames(),
            [Path::new("L"), Path::new("R")],
            ["HEAD^", "HEAD"],
            &DiffOptions::default(),
            Theme::PLAIN,
            false,
            read,
        );
        insta::assert_snapshot!(out, @r"
        Renamed HEAD^/LICENSE to HEAD/COPYING
        --- HEAD^/util.rs
        +++ HEAD/src/util.rs
        @@ -1,1 +1,1 @@
        -one
        +two
        ");
    }

    #[test]
    fn whole_files_shows_added_and_deleted_files_against_dev_null() {
        let diff = FolderDiff {
            entries: vec![
                entry("gone.txt", EntryStatus::OnlyLeft),
                entry("img.png", EntryStatus::OnlyRight),
                entry("src/new.rs", EntryStatus::OnlyRight),
            ],
        };
        let read = |p: &Path| -> std::io::Result<Vec<u8>> {
            Ok(match p.to_str().unwrap() {
                "L/gone.txt" => b"bye\n".to_vec(),
                "R/img.png" => b"\x89PNG\0".to_vec(),
                "R/src/new.rs" => b"fn main() {}\nfn x() {}\n".to_vec(),
                other => return Err(std::io::Error::other(format!("unexpected read {other}"))),
            })
        };
        let out = unified_all(
            &diff,
            [Path::new("L"), Path::new("R")],
            ["HEAD^", "HEAD"],
            &DiffOptions::default(),
            Theme::PLAIN,
            true,
            read,
        );
        insta::assert_snapshot!(out, @"
        --- HEAD^/gone.txt
        +++ /dev/null
        @@ -1,1 +0,0 @@
        -bye
        Binary file HEAD/img.png added
        --- /dev/null
        +++ HEAD/src/new.rs
        @@ -0,0 +1,2 @@
        +fn main() {}
        +fn x() {}
        ");
    }
}
