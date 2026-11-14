//! Recursive directory comparison.
//!
//! Directory walking is the only filesystem access in the core; the comparison
//! of the two listings ([`compare_listings`]) is pure.
//!
//! * Files whose bytes differ but are both text are re-compared with the text
//!   options, so `whitespace`, `ignore`, masks etc. also apply in folder mode.
//! * A directory present on only one side is reported once (not its contents).
//! * Globs without a `/` match at any depth (`*.log` == `**/*.log`), like
//!   `.gitignore`. Excluded directories are not descended into.
//! * With include/exclude filters active, directories are listed only if they
//!   contain at least one compared file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::Serialize;

use crate::detect::looks_binary;
use crate::options::DiffOptions;
use crate::text::diff_text;
use crate::{Error, Result};

/// Result for one relative path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "status", content = "detail")]
pub enum EntryStatus {
    /// Byte-identical, or equal under the text options.
    Same,
    /// Contents differ.
    Different,
    /// Only on the left.
    OnlyLeft,
    /// Only on the right.
    OnlyRight,
    /// A file on one side, a directory on the other.
    TypeMismatch,
    /// Could not be read.
    Error(String),
}

impl EntryStatus {
    /// Is this a difference?
    pub fn is_change(&self) -> bool {
        !matches!(self, Self::Same)
    }

    /// One-character marker, consistent with text rows.
    pub fn marker(&self) -> char {
        match self {
            Self::Same => ' ',
            Self::Different => '~',
            Self::OnlyLeft => '-',
            Self::OnlyRight => '+',
            Self::TypeMismatch => '!',
            Self::Error(_) => '?',
        }
    }
}

/// One compared path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FolderEntry {
    /// Path relative to the compared roots, `/`-separated.
    pub path: String,
    /// `true` if this is a directory (only reported when on one side only).
    pub is_dir: bool,
    /// Comparison result.
    #[serde(flatten)]
    pub status: EntryStatus,
}

/// Counts per status.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct FolderStats {
    pub same: usize,
    pub different: usize,
    pub only_left: usize,
    pub only_right: usize,
    pub type_mismatch: usize,
    pub errors: usize,
}

/// Full folder comparison result, sorted by path.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FolderDiff {
    pub entries: Vec<FolderEntry>,
}

impl FolderDiff {
    pub fn is_identical(&self) -> bool {
        !self.entries.iter().any(|e| e.status.is_change())
    }

    pub fn stats(&self) -> FolderStats {
        let mut s = FolderStats::default();
        for e in &self.entries {
            match e.status {
                EntryStatus::Same => s.same += 1,
                EntryStatus::Different => s.different += 1,
                EntryStatus::OnlyLeft => s.only_left += 1,
                EntryStatus::OnlyRight => s.only_right += 1,
                EntryStatus::TypeMismatch => s.type_mismatch += 1,
                EntryStatus::Error(_) => s.errors += 1,
            }
        }
        s
    }

    /// Stable text form used by snapshot tests: `<marker> <path>[/]`.
    pub fn render_review(&self) -> String {
        let mut out = String::new();
        for e in &self.entries {
            let slash = if e.is_dir { "/" } else { "" };
            out.push_str(&format!("{} {}{slash}", e.status.marker(), e.path));
            if let EntryStatus::Error(msg) = &e.status {
                out.push_str(&format!("  ({msg})"));
            }
            out.push('\n');
        }
        out
    }
}

/// What a relative path is on one side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    File,
    Dir,
}

/// Compiled include/exclude globs.
#[derive(Debug, Clone)]
pub struct PathFilter {
    include: Option<GlobSet>,
    exclude: GlobSet,
    active: bool,
}

fn glob_set(patterns: &[String]) -> Result<GlobSet> {
    let mut b = GlobSetBuilder::new();
    for p in patterns {
        let anchored = if p.contains('/') {
            p.trim_start_matches('/').to_owned()
        } else {
            format!("**/{p}")
        };
        let glob = GlobBuilder::new(&anchored)
            .literal_separator(true)
            .build()
            .map_err(|e| Error::Glob {
                pattern: p.clone(),
                source: Box::new(e),
            })?;
        b.add(glob);
    }
    b.build().map_err(|e| Error::Glob {
        pattern: patterns.join(", "),
        source: Box::new(e),
    })
}

impl PathFilter {
    pub fn new(opts: &DiffOptions) -> Result<Self> {
        Ok(Self {
            include: if opts.include.is_empty() {
                None
            } else {
                Some(glob_set(&opts.include)?)
            },
            exclude: glob_set(&opts.exclude)?,
            active: !(opts.include.is_empty() && opts.exclude.is_empty()),
        })
    }

    /// Should this directory be walked? (`target/**` excludes `target` itself.)
    pub fn walk_dir(&self, rel: &str) -> bool {
        !(self.exclude.is_match(rel) || self.exclude.is_match(format!("{rel}/")))
    }

    /// Should this file be compared?
    pub fn keep_file(&self, rel: &str) -> bool {
        !self.exclude.is_match(rel) && self.include.as_ref().is_none_or(|g| g.is_match(rel))
    }
}

/// Walk a directory into `relative path -> kind`, applying the filter.
pub fn list_dir(root: &Path, filter: &PathFilter) -> Result<BTreeMap<String, NodeKind>> {
    let mut out = BTreeMap::new();
    let walker = walkdir::WalkDir::new(root)
        .min_depth(1)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if !e.file_type().is_dir() {
                return true;
            }
            rel_path(root, e.path()).is_none_or(|rel| filter.walk_dir(&rel))
        });
    for entry in walker {
        let entry = entry.map_err(|e| {
            let path = e
                .path()
                .map_or_else(|| root.to_path_buf(), Path::to_path_buf);
            Error::io(path, e.into())
        })?;
        let Some(rel) = rel_path(root, entry.path()) else {
            continue;
        };
        if entry.file_type().is_dir() {
            out.insert(rel, NodeKind::Dir);
        } else if filter.keep_file(&rel) {
            out.insert(rel, NodeKind::File);
        }
    }
    if filter.active {
        // With filters, a directory only matters if it holds a compared file.
        let files: Vec<String> = out
            .iter()
            .filter(|(_, k)| **k == NodeKind::File)
            .map(|(p, _)| format!("{p}/"))
            .collect();
        out.retain(|p, k| {
            *k == NodeKind::File || {
                let prefix = format!("{p}/");
                files.iter().any(|f| f.starts_with(&prefix))
            }
        });
    }
    Ok(out)
}

fn rel_path(root: &Path, p: &Path) -> Option<String> {
    let rel = p.strip_prefix(root).ok()?;
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Compare two directory trees on disk.
pub fn diff_folders(left: &Path, right: &Path, opts: &DiffOptions) -> Result<FolderDiff> {
    let filter = PathFilter::new(opts)?;
    let l = list_dir(left, &filter)?;
    let r = list_dir(right, &filter)?;
    let read = |side: &Path, rel: &str| -> std::result::Result<Vec<u8>, String> {
        let p: PathBuf = side.join(rel);
        std::fs::read(&p).map_err(|e| e.to_string())
    };
    Ok(compare_listings(&l, &r, |rel| {
        let a = read(left, rel)?;
        let b = read(right, rel)?;
        Ok(files_equal(&a, &b, opts))
    }))
}

/// Are two files equal: identical bytes, or both text and equal under `opts`?
pub fn files_equal(a: &[u8], b: &[u8], opts: &DiffOptions) -> bool {
    if a == b {
        return true;
    }
    if looks_binary(a) || looks_binary(b) {
        return false;
    }
    let (a, b) = (String::from_utf8_lossy(a), String::from_utf8_lossy(b));
    diff_text(&a, &b, opts).is_ok_and(|d| d.is_identical())
}

/// Merge two listings. `same_content(rel)` is called for paths that are files on
/// both sides and returns whether they are equal (or an error message).
pub fn compare_listings(
    left: &BTreeMap<String, NodeKind>,
    right: &BTreeMap<String, NodeKind>,
    mut same_content: impl FnMut(&str) -> std::result::Result<bool, String>,
) -> FolderDiff {
    let mut entries = Vec::new();
    let mut skip_under: Option<String> = None; // a one-sided directory being reported once
    let all: std::collections::BTreeSet<&String> = left.keys().chain(right.keys()).collect();
    for path in all {
        if let Some(prefix) = &skip_under {
            if path.starts_with(prefix.as_str()) {
                continue;
            }
            skip_under = None;
        }
        let (lk, rk) = (left.get(path).copied(), right.get(path).copied());
        let (status, is_dir) = match (lk, rk) {
            (Some(NodeKind::Dir), Some(NodeKind::Dir)) => continue,
            (Some(NodeKind::File), Some(NodeKind::File)) => {
                let status = match same_content(path) {
                    Ok(true) => EntryStatus::Same,
                    Ok(false) => EntryStatus::Different,
                    Err(msg) => EntryStatus::Error(msg),
                };
                (status, false)
            }
            (Some(_), Some(_)) => (EntryStatus::TypeMismatch, false),
            (Some(k), None) => (EntryStatus::OnlyLeft, k == NodeKind::Dir),
            (None, Some(k)) => (EntryStatus::OnlyRight, k == NodeKind::Dir),
            (None, None) => unreachable!("path came from one of the maps"),
        };
        if is_dir || status == EntryStatus::TypeMismatch {
            skip_under = Some(format!("{path}/"));
        }
        entries.push(FolderEntry {
            path: path.clone(),
            is_dir,
            status,
        });
    }
    FolderDiff { entries }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn listing(items: &[(&str, NodeKind)]) -> BTreeMap<String, NodeKind> {
        items.iter().map(|(p, k)| ((*p).to_owned(), *k)).collect()
    }

    use NodeKind::{Dir as D, File as F};

    #[test]
    fn merge_classifies_every_case() {
        let l = listing(&[
            ("a", F),
            ("b", F),
            ("c", F),
            ("only_l", F),
            ("x", F),
            ("dl", D),
            ("dl/f", F),
        ]);
        let r = listing(&[
            ("a", F),
            ("b", F),
            ("c", F),
            ("only_r", F),
            ("x", D),
            ("x/inner", F),
        ]);
        let d = compare_listings(&l, &r, |p| match p {
            "a" => Ok(true),
            "b" => Ok(false),
            _ => Err("denied".into()),
        });
        insta::assert_snapshot!(d.render_review(), @r"
          a
        ~ b
        ? c  (denied)
        - dl/
        - only_l
        + only_r
        ! x
        ");
    }

    #[test]
    fn swap_symmetry() {
        let l = listing(&[("a", F), ("l", F), ("d", D), ("d/x", F)]);
        let r = listing(&[("a", F), ("r", F)]);
        let ab = compare_listings(&l, &r, |_| Ok(false)).stats();
        let ba = compare_listings(&r, &l, |_| Ok(false)).stats();
        assert_eq!((ab.only_left, ab.only_right), (ba.only_right, ba.only_left));
        assert_eq!(ab.different, ba.different);
    }

    #[test]
    fn sibling_with_common_prefix_is_not_skipped() {
        let l = listing(&[("d", D), ("d/x", F), ("d2", F)]);
        let r = listing(&[("d2", F)]);
        let d = compare_listings(&l, &r, |_| Ok(true));
        assert_eq!(d.render_review(), "- d/\n  d2\n");
    }

    fn filter(include: &[&str], exclude: &[&str]) -> PathFilter {
        PathFilter::new(&DiffOptions {
            include: include.iter().map(ToString::to_string).collect(),
            exclude: exclude.iter().map(ToString::to_string).collect(),
            ..DiffOptions::default()
        })
        .unwrap()
    }

    #[rstest]
    #[case::no_filters(&[], &[], "a/b.rs", true)]
    #[case::name_glob_any_depth(&["*.rs"], &[], "a/b/c.rs", true)]
    #[case::name_glob_top(&["*.rs"], &[], "c.rs", true)]
    #[case::include_miss(&["*.rs"], &[], "c.txt", false)]
    #[case::exclude_name(&[], &["*.log"], "x/y.log", false)]
    #[case::exclude_beats_include(&["*.rs"], &["gen_*.rs"], "src/gen_a.rs", false)]
    #[case::path_glob_anchored(&["src/*.rs"], &[], "lib/src/a.rs", false)]
    #[case::path_glob_match(&["src/*.rs"], &[], "src/a.rs", true)]
    #[case::star_no_cross_dir(&["src/*.rs"], &[], "src/x/a.rs", false)]
    fn keep_file(
        #[case] inc: &[&str],
        #[case] exc: &[&str],
        #[case] path: &str,
        #[case] expected: bool,
    ) {
        assert_eq!(filter(inc, exc).keep_file(path), expected);
    }

    #[rstest]
    #[case::dir_star_star("target/**", "target", false)]
    #[case::bare_name(".git", ".git", false)]
    #[case::bare_name_nested("node_modules", "web/node_modules", false)]
    #[case::other_dir("target/**", "src", true)]
    fn walk_dir(#[case] exc: &str, #[case] dir: &str, #[case] expected: bool) {
        assert_eq!(filter(&[], &[exc]).walk_dir(dir), expected);
    }

    #[test]
    fn bad_glob_is_an_error() {
        let err = PathFilter::new(&DiffOptions {
            exclude: vec!["a[".into()],
            ..DiffOptions::default()
        })
        .unwrap_err();
        assert!(err.to_string().contains("invalid glob `a[`"), "{err}");
    }

    #[rstest]
    #[case::same_bytes(b"a\n", b"a\n", false, true)]
    #[case::different_text(b"a\n", b"b\n", false, false)]
    #[case::equal_ignoring_ws(b"a  b\n", b"a b\n", true, true)]
    #[case::binary_never_text_equal(b"a\0 b", b"a\0b", true, false)]
    fn file_equality(
        #[case] a: &[u8],
        #[case] b: &[u8],
        #[case] ws_all: bool,
        #[case] expected: bool,
    ) {
        let opts = DiffOptions {
            whitespace: if ws_all {
                crate::WhitespaceMode::All
            } else {
                crate::WhitespaceMode::Exact
            },
            ..DiffOptions::default()
        };
        assert_eq!(files_equal(a, b, &opts), expected);
    }

    #[test]
    fn walks_real_directories() {
        let l = tempfile::tempdir().unwrap();
        let r = tempfile::tempdir().unwrap();
        let write = |root: &Path, rel: &str, body: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        write(l.path(), "same.txt", "x\n");
        write(r.path(), "same.txt", "x\n");
        write(l.path(), "src/main.rs", "fn main() {}\n");
        write(r.path(), "src/main.rs", "fn main() { }\n");
        write(l.path(), "target/out.bin", "junk");
        write(r.path(), "new/file.txt", "n\n");
        let opts = DiffOptions {
            exclude: vec!["target/**".into()],
            ..DiffOptions::default()
        };
        let d = diff_folders(l.path(), r.path(), &opts).unwrap();
        assert_eq!(d.render_review(), "+ new/\n  same.txt\n~ src/main.rs\n");
    }

    #[test]
    fn filtered_out_directories_are_not_listed() {
        let l = tempfile::tempdir().unwrap();
        let r = tempfile::tempdir().unwrap();
        for (root, rel) in [
            (l.path(), "a.rs"),
            (r.path(), "a.rs"),
            (r.path(), "docs/x.md"),
            (r.path(), "src/b.rs"),
        ] {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "x").unwrap();
        }
        let opts = DiffOptions {
            include: vec!["*.rs".into()],
            ..DiffOptions::default()
        };
        let d = diff_folders(l.path(), r.path(), &opts).unwrap();
        assert_eq!(d.render_review(), "  a.rs\n+ src/\n");
    }
}
