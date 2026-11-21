//! AI output (`--ai`, `-f ai` or `MYDIFF_AI=1`): compact annotated text for
//! coding agents and LLM prompts. See [`HELP`] for the format.
//!
//! * The summary and file list come first, so a reader knows the size of the
//!   change before any content.
//! * Every diff line carries both line numbers and the exact text (no inline
//!   markers), so an agent can quote it in an edit.
//! * Output stops at a line budget and always says what it left out and how to
//!   get it (`--hunk <id>`, `--include <path>`, `--budget 0`). Nothing is cut
//!   silently.
//! * Lockfiles and minified files are listed but not expanded unless asked for.

use std::fmt::Write as _;
use std::path::Path;

use mydiff_core::binary::{HexStats, diff_bytes};
use mydiff_core::detect::looks_binary;
use mydiff_core::folder::{EntryStatus, FolderDiff};
use mydiff_core::structured::{StructDiff, describe};
use mydiff_core::{DiffOptions, Row, RowKind, TextDiff, WhitespaceMode, diff_text};

/// First line of every AI report; bumped when the format changes incompatibly.
pub const VERSION_LINE: &str = "mydiff-ai v1";
/// Lines printed when neither `--budget` nor `MYDIFF_AI_BUDGET` says otherwise.
pub const DEFAULT_BUDGET: usize = 2000;
/// Files that are listed but not expanded unless asked for (`--include`).
const LOCKFILES: &[&str] = &[
    "Cargo.lock",
    "package-lock.json",
    "npm-shrinkwrap.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "bun.lockb",
    "poetry.lock",
    "uv.lock",
    "Pipfile.lock",
    "Gemfile.lock",
    "composer.lock",
    "go.sum",
    "flake.lock",
];
/// Longest structured value shown before it is cut with `…`.
const VALUE_CHARS: usize = 200;

/// Printed by `mydiff --ai-help`: the format, for an agent's instructions.
pub const HELP: &str = "\
mydiff AI output (`mydiff --ai A B`, or set MYDIFF_AI=1)

Read-only, never interactive, no color. Exit status: 0 identical, 1 different, 2 error.
Works for files, folders and git: `mydiff --ai git show [<commit>]`,
`mydiff --ai git diff [--staged] [<rev>...]`.

Layout:
  mydiff-ai v1                      format version
  compare: <left> -> <right>
  summary: <what changed, in counts>
  files:                            (folders and git only)
    <S> <path> [+added -removed] [(tags)]
  === <S> <path>                    one section per file with content to show
  @@ <id> -<start>,<count> +<start>,<count> @@
  <left no> <right no> <marker> <text>

S (file status): M modified, A added, D deleted, R renamed (`old -> new`),
  T file vs directory, B binary, ? unreadable.
marker: ' ' unchanged, '-' left only, '+' right only, '.' ignored by options.
  A changed line is a '-' line followed by a '+' line. Text is exact.
Tags: (whitespace only), (content unchanged), (lockfile|minified; collapsed).
<id> is <path>#<n> (just #<n> for two files); pass it to --hunk.

Budget: output stops after MYDIFF_AI_BUDGET lines (default 2000) and ends
with a `truncated:` line naming what was left out. To see more, rerun with
  --hunk <id>        those hunks only, in full (repeatable)
  --include <path>   only matching files; collapsed files are then expanded
  --budget <n>       another limit; 0 = no limit
Comparison options work too: -w (ignore whitespace), -U <n> (context lines).
";

/// What to print and how much.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiOptions {
    /// Most lines to print; 0 means no limit.
    pub budget: usize,
    /// `--hunk` ids; when given, only those hunks are shown (in full).
    pub hunks: Vec<String>,
    /// Context lines around changes.
    pub context: usize,
    /// The comparison was narrowed with `--include`: expand collapsed files.
    pub filtered: bool,
}

impl Default for AiOptions {
    fn default() -> Self {
        Self {
            budget: DEFAULT_BUDGET,
            hunks: Vec::new(),
            context: 3,
            filtered: false,
        }
    }
}

/// How a file differs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Modified,
    Added,
    Deleted,
    Renamed,
    /// A file on one side, a directory on the other.
    TypeChanged,
    Unreadable,
}

impl Status {
    fn letter(self) -> char {
        match self {
            Self::Modified => 'M',
            Self::Added => 'A',
            Self::Deleted => 'D',
            Self::Renamed => 'R',
            Self::TypeChanged => 'T',
            Self::Unreadable => '?',
        }
    }
}

/// The content difference of one file.
#[derive(Debug, Clone)]
pub enum Body {
    Text {
        diff: TextDiff,
        /// The files are equal once whitespace is ignored.
        whitespace_only: bool,
    },
    Structured {
        format: &'static str,
        diff: StructDiff,
    },
    Binary(HexStats),
    /// One line of explanation (directories, errors).
    Note(String),
    /// A rename with the same content.
    Unchanged,
}

/// One compared file. `path` is empty when two files were compared directly.
#[derive(Debug, Clone)]
pub struct FileDiff {
    pub status: Status,
    pub path: String,
    pub old_path: Option<String>,
    pub body: Body,
}

impl FileDiff {
    /// Two files compared directly (no folder).
    pub fn single(body: Body) -> Self {
        Self {
            status: Status::Modified,
            path: String::new(),
            old_path: None,
            body,
        }
    }

    fn display(&self) -> String {
        match &self.old_path {
            Some(old) => format!("{old} -> {}", self.path),
            None => self.path.clone(),
        }
    }

    fn letter(&self) -> char {
        match (&self.body, self.status) {
            (Body::Binary(_), Status::Modified) => 'B',
            (_, s) => s.letter(),
        }
    }

    /// Lines added and removed.
    fn counts(&self) -> (usize, usize) {
        match &self.body {
            Body::Text { diff, .. } => {
                let s = diff.stats();
                (s.added + s.modified, s.removed + s.modified)
            }
            _ => (0, 0),
        }
    }

    /// Why this file's content is not expanded by default.
    fn noise(&self) -> Option<&'static str> {
        let name = self.path.rsplit('/').next().unwrap_or(&self.path);
        if self.path.is_empty() {
            None
        } else if LOCKFILES.contains(&name) {
            Some("lockfile")
        } else if [".min.js", ".min.css", ".js.map", ".css.map"]
            .iter()
            .any(|ext| name.ends_with(ext))
        {
            Some("minified")
        } else {
            None
        }
    }

    fn has_content(&self) -> bool {
        !matches!(self.body, Body::Unchanged)
    }
}

/// Diff two texts for AI output, noting whether only whitespace differs.
pub fn text_body(left: &str, right: &str, opts: &DiffOptions) -> mydiff_core::Result<Body> {
    let diff = diff_text(left, right, opts)?;
    let whitespace_only = !diff.is_identical()
        && opts.whitespace != WhitespaceMode::All
        && diff_text(
            left,
            right,
            &DiffOptions {
                whitespace: WhitespaceMode::All,
                ignore_blank_lines: true,
                ..opts.clone()
            },
        )
        .is_ok_and(|d| d.is_identical());
    Ok(Body::Text {
        diff,
        whitespace_only,
    })
}

/// The changed entries of a folder comparison as [`FileDiff`]s. A file on one
/// side only is shown in full against nothing. `read` reads a file.
pub fn folder_files(
    diff: &FolderDiff,
    [left, right]: [&Path; 2],
    opts: &DiffOptions,
    read: impl Fn(&Path) -> std::io::Result<Vec<u8>>,
) -> Vec<FileDiff> {
    let mut files = Vec::new();
    for e in diff.entries.iter().filter(|e| e.is_change()) {
        let (status, l, r) = match &e.status {
            EntryStatus::OnlyLeft => (Status::Deleted, Some(e.left_path()), None),
            EntryStatus::OnlyRight => (Status::Added, None, Some(e.path.as_str())),
            EntryStatus::TypeMismatch => (Status::TypeChanged, None, None),
            EntryStatus::Error(_) => (Status::Unreadable, None, None),
            EntryStatus::Same | EntryStatus::Different => {
                let s = if e.renamed_from.is_some() {
                    Status::Renamed
                } else {
                    Status::Modified
                };
                (s, Some(e.left_path()), Some(e.path.as_str()))
            }
        };
        let body = if let EntryStatus::Error(msg) = &e.status {
            Body::Note(format!("cannot compare: {msg}"))
        } else if status == Status::TypeChanged {
            Body::Note("a file on one side, a directory on the other".to_owned())
        } else if e.is_dir {
            let side = if status == Status::Added {
                "right"
            } else {
                "left"
            };
            Body::Note(format!("directory only on the {side}"))
        } else if e.status == EntryStatus::Same {
            Body::Unchanged
        } else {
            let load = |root: &Path, rel: Option<&str>| match rel {
                Some(rel) => read(&root.join(rel)),
                None => Ok(Vec::new()),
            };
            match (load(left, l), load(right, r)) {
                (Ok(a), Ok(b)) if looks_binary(&a) || looks_binary(&b) => {
                    Body::Binary(diff_bytes(a, b).stats())
                }
                (Ok(a), Ok(b)) => {
                    let (a, b) = (String::from_utf8_lossy(&a), String::from_utf8_lossy(&b));
                    text_body(&a, &b, opts).unwrap_or_else(|e| Body::Note(e.to_string()))
                }
                (Err(err), _) | (_, Err(err)) => Body::Note(format!("cannot read: {err}")),
            }
        };
        files.push(FileDiff {
            status,
            path: e.path.clone(),
            old_path: e.renamed_from.clone(),
            body,
        });
    }
    files
}

/// Output under construction, counting lines against the budget.
struct Out {
    text: String,
    lines: usize,
    budget: usize,
}

impl Out {
    fn line(&mut self, s: &str) {
        self.text.push_str(s);
        self.text.push('\n');
        self.lines += 1;
    }

    fn fits(&self, n: usize) -> bool {
        self.budget == 0 || self.lines + n <= self.budget
    }
}

/// Render the AI report for `files` compared between `left` and `right`.
pub fn render(left: &str, right: &str, files: &[FileDiff], o: &AiOptions) -> String {
    let mut out = Out {
        text: String::new(),
        lines: 0,
        budget: o.budget,
    };
    let single = matches!(files, [f] if f.path.is_empty());
    out.line(VERSION_LINE);
    out.line(&format!("compare: {left} -> {right}"));
    out.line(&format!("summary: {}", summary(files, single, o.context)));
    let selecting = !o.hunks.is_empty();
    if !single && !files.is_empty() && !selecting {
        file_list(&mut out, files, o);
    }

    let mut found = vec![false; o.hunks.len()];
    let mut cut: Option<Cut> = None;
    for (fi, f) in files.iter().enumerate() {
        if cut.is_some() {
            break;
        }
        if !f.has_content() || (f.noise().is_some() && !o.filtered && !selecting) {
            continue;
        }
        let blocks = blocks(f, single, o.context);
        let wanted: Vec<usize> = (0..blocks.len())
            .filter(|&i| {
                !selecting
                    || blocks[i].id.as_ref().is_some_and(|id| {
                        let hit = o.hunks.iter().position(|h| hunk_matches(h, id, single));
                        if let Some(h) = hit {
                            found[h] = true;
                        }
                        hit.is_some()
                    })
            })
            .collect();
        if wanted.is_empty() {
            continue;
        }
        let mut header = (!single).then(|| format!("=== {} {}", f.letter(), f.display()));
        for (n, &bi) in wanted.iter().enumerate() {
            let b = &blocks[bi];
            let extra = header.as_ref().map_or(0, |_| 2);
            if !selecting && !out.fits(b.lines.len() + extra) {
                cut = Some(Cut {
                    file: fi,
                    ids: wanted[n..]
                        .iter()
                        .filter_map(|&i| blocks[i].id.clone())
                        .collect(),
                    first_block: n == 0,
                });
                break;
            }
            if let Some(h) = header.take() {
                out.line("");
                out.line(&h);
            }
            for l in &b.lines {
                out.line(l);
            }
        }
    }
    for (h, hit) in o.hunks.iter().zip(&found) {
        if !hit {
            out.line(&format!("not found: --hunk {h}"));
        }
    }
    if let Some(cut) = cut {
        truncated(&mut out, files, &cut, o.budget);
    }
    out.text
}

/// Where output stopped: in `files[file]`, before the hunks `ids`.
struct Cut {
    file: usize,
    ids: Vec<String>,
    /// Nothing of this file was printed.
    first_block: bool,
}

fn truncated(out: &mut Out, files: &[FileDiff], cut: &Cut, budget: usize) {
    let later = files[cut.file + 1..]
        .iter()
        .filter(|f| f.has_content() && f.noise().is_none())
        .count();
    let f = &files[cut.file];
    let mut parts = Vec::new();
    match cut.ids.as_slice() {
        [] => {}
        [one] => parts.push(one.clone()),
        [first, .., last] => parts.push(format!("{first} to {last}")),
    }
    if cut.ids.is_empty() && cut.first_block && !f.path.is_empty() {
        parts.push(f.path.clone());
    }
    if later > 0 {
        parts.push(format!(
            "{later} more file{}",
            if later == 1 { "" } else { "s" }
        ));
    }
    out.text.push('\n');
    out.text.push_str(&format!(
        "truncated: {budget}-line budget reached; not shown: {}\n",
        parts.join(", ")
    ));
    let example = cut.ids.first().map_or_else(
        || format!("--include '{}'", f.path),
        |id| format!("--hunk {id}"),
    );
    // Two files compared directly have no paths to narrow to.
    let include = if f.path.is_empty() {
        ""
    } else {
        ", --include '<path>'"
    };
    out.text.push_str(&format!(
        "more: rerun with {example}{include}, or --budget 0 for everything\n"
    ));
}

/// Does `--hunk wanted` name the hunk `id`? Two-file ids also match as `n` or `#n`.
fn hunk_matches(wanted: &str, id: &str, single: bool) -> bool {
    wanted == id || (single && format!("#{}", wanted.trim_start_matches('#')) == id)
}

fn summary(files: &[FileDiff], single: bool, context: usize) -> String {
    if files.is_empty() {
        return "identical".to_owned();
    }
    if single {
        return match &files[0].body {
            Body::Text {
                diff,
                whitespace_only,
            } => {
                if diff.is_identical() {
                    return "identical".to_owned();
                }
                let (plus, minus) = files[0].counts();
                let n = diff.hunks(context).len();
                let ws = if *whitespace_only {
                    " (whitespace only)"
                } else {
                    ""
                };
                format!(
                    "text, +{plus} -{minus} lines in {n} hunk{}{ws}",
                    if n == 1 { "" } else { "s" }
                )
            }
            Body::Structured { format, diff } => {
                if diff.is_identical() {
                    return format!("identical {format}");
                }
                let s = diff.stats();
                format!(
                    "{format}, {} added, {} removed, {} changed",
                    s.added, s.removed, s.changed
                )
            }
            Body::Binary(s) => binary_line(s),
            Body::Note(n) => n.clone(),
            Body::Unchanged => "identical".to_owned(),
        };
    }
    let count = |s: Status| files.iter().filter(|f| f.status == s).count();
    let mut parts = Vec::new();
    for (s, what) in [
        (Status::Modified, "modified"),
        (Status::Added, "added"),
        (Status::Deleted, "deleted"),
        (Status::Renamed, "renamed"),
        (Status::TypeChanged, "file/dir mismatch"),
        (Status::Unreadable, "unreadable"),
    ] {
        let n = count(s);
        if n > 0 {
            parts.push(format!("{n} {what}"));
        }
    }
    let (plus, minus) = files.iter().fold((0, 0), |(p, m), f| {
        let (a, b) = f.counts();
        (p + a, m + b)
    });
    format!(
        "{} file{} changed: {}; +{plus} -{minus} lines",
        files.len(),
        if files.len() == 1 { "" } else { "s" },
        parts.join(", ")
    )
}

fn binary_line(s: &HexStats) -> String {
    match s.first_difference {
        None => format!("binary, identical ({} bytes)", s.left_len),
        Some(first) => format!(
            "binary, {} differing byte{}, first at 0x{first:x}, size {} -> {}",
            s.differing_bytes,
            if s.differing_bytes == 1 { "" } else { "s" },
            s.left_len,
            s.right_len
        ),
    }
}

/// The `files:` section. Long lists stop at half the budget.
fn file_list(out: &mut Out, files: &[FileDiff], o: &AiOptions) {
    out.line("files:");
    let cap = if o.budget == 0 {
        usize::MAX
    } else {
        (o.budget / 2).max(20)
    };
    for f in files.iter().take(cap) {
        let mut line = format!("  {} {}", f.letter(), f.display());
        let (plus, minus) = f.counts();
        if plus > 0 {
            let _ = write!(line, " +{plus}");
        }
        if minus > 0 {
            let _ = write!(line, " -{minus}");
        }
        match &f.body {
            Body::Text {
                whitespace_only: true,
                ..
            } => line.push_str(" (whitespace only)"),
            Body::Unchanged => line.push_str(" (content unchanged)"),
            Body::Binary(s) => {
                let _ = write!(line, " ({})", binary_line(s));
            }
            Body::Note(n) => {
                let _ = write!(line, " ({n})");
            }
            _ => {}
        }
        if let Some(why) = f.noise().filter(|_| !o.filtered) {
            let _ = write!(line, " ({why}; collapsed)");
        }
        out.line(&line);
    }
    if files.len() > cap {
        out.line(&format!(
            "  ... {} more files (narrow with --include '<path>')",
            files.len() - cap
        ));
    }
}

/// A unit of content that is printed whole or not at all.
struct Block {
    /// Hunk id (`path#n`), if this is a hunk.
    id: Option<String>,
    lines: Vec<String>,
}

fn blocks(f: &FileDiff, single: bool, context: usize) -> Vec<Block> {
    let plain = |lines: Vec<String>| Block { id: None, lines };
    match &f.body {
        Body::Text { diff, .. } => {
            let prefix = if single { "" } else { f.path.as_str() };
            hunk_blocks(diff, prefix, context)
        }
        Body::Structured { diff, .. } => diff
            .changes
            .iter()
            .map(|c| plain(vec![describe(c, VALUE_CHARS)]))
            .collect(),
        // Already in the summary or the file list.
        Body::Binary(_) | Body::Note(_) | Body::Unchanged => Vec::new(),
    }
}

fn hunk_blocks(diff: &TextDiff, prefix: &str, context: usize) -> Vec<Block> {
    let mut out = Vec::new();
    for (i, h) in diff.hunks(context).into_iter().enumerate() {
        let id = format!("{prefix}#{}", i + 1);
        let before = &diff.rows[..h.rows.start];
        let rows = &diff.rows[h.rows];
        let side = |rows: &[Row], left: bool| {
            rows.iter()
                .filter(|r| {
                    if left {
                        r.left.is_some()
                    } else {
                        r.right.is_some()
                    }
                })
                .count()
        };
        let (lc, rc) = (side(rows, true), side(rows, false));
        let start = |before: usize, count: usize| if count == 0 { before } else { before + 1 };
        let mut lines = vec![format!(
            "@@ {id} -{},{lc} +{},{rc} @@",
            start(side(before, true), lc),
            start(side(before, false), rc)
        )];
        let widest = rows
            .iter()
            .flat_map(|r| [r.left.as_ref(), r.right.as_ref()])
            .flatten()
            .map(|l| l.number)
            .max()
            .unwrap_or(0);
        let w = widest.to_string().len();
        let line = |l: Option<usize>, r: Option<usize>, m: char, text: &str| {
            let n = |x: Option<usize>| x.map_or_else(String::new, |x| x.to_string());
            let mut s = format!("{:>w$} {:>w$} {m}", n(l), n(r));
            if !text.is_empty() {
                s.push(' ');
                s.push_str(text);
            }
            s
        };
        for r in rows {
            let (l, rt) = (r.left.as_ref(), r.right.as_ref());
            match (r.kind, l, rt) {
                (RowKind::Equal, Some(l), Some(rt)) => {
                    // The right side is the current text, which an edit must match.
                    lines.push(line(Some(l.number), Some(rt.number), ' ', &rt.text));
                }
                (RowKind::Ignored, Some(l), Some(rt)) if l.text == rt.text => {
                    lines.push(line(Some(l.number), Some(rt.number), '.', &rt.text));
                }
                (kind, l, rt) => {
                    let (lm, rm) = if kind == RowKind::Ignored {
                        ('.', '.')
                    } else {
                        ('-', '+')
                    };
                    if let Some(l) = l {
                        lines.push(line(Some(l.number), None, lm, &l.text));
                    }
                    if let Some(rt) = rt {
                        lines.push(line(None, Some(rt.number), rm, &rt.text));
                    }
                }
            }
        }
        out.push(Block {
            id: Some(id),
            lines,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use mydiff_core::folder::{FolderEntry, Rename, Roots, diff_roots};

    use super::*;

    fn text(left: &str, right: &str) -> Body {
        text_body(left, right, &DiffOptions::default()).unwrap()
    }

    fn file(status: Status, path: &str, body: Body) -> FileDiff {
        FileDiff {
            status,
            path: path.into(),
            old_path: None,
            body,
        }
    }

    fn opts() -> AiOptions {
        AiOptions::default()
    }

    /// `n` numbered lines, with `changed` lines (1-based) edited on the right.
    fn numbered(n: usize, changed: &[usize]) -> (String, String) {
        let l: String = (1..=n).map(|i| format!("line {i}\n")).collect();
        let r: String = (1..=n)
            .map(|i| {
                if changed.contains(&i) {
                    format!("LINE {i} edited\n")
                } else {
                    format!("line {i}\n")
                }
            })
            .collect();
        (l, r)
    }

    #[test]
    fn two_files_show_both_line_numbers_and_exact_text() {
        let body = text(
            "alpha\nbeta\ngamma\n\tdelta  \n",
            "alpha\nbeta two\ngamma\n\tdelta  \nnew\n",
        );
        insta::assert_snapshot!(render("a.txt", "b.txt", &[FileDiff::single(body)], &opts()), @"
        mydiff-ai v1
        compare: a.txt -> b.txt
        summary: text, +2 -1 lines in 1 hunk
        @@ #1 -1,4 +1,5 @@
        1 1   alpha
        2   - beta
          2 + beta two
        3 3   gamma
        4 4   	delta  
          5 + new
        ");
    }

    #[test]
    fn identical_inputs_say_so_and_stop() {
        let out = render("a", "b", &[FileDiff::single(text("x\n", "x\n"))], &opts());
        assert_eq!(out, "mydiff-ai v1\ncompare: a -> b\nsummary: identical\n");
        assert_eq!(
            render("a/", "b/", &[], &opts()),
            "mydiff-ai v1\ncompare: a/ -> b/\nsummary: identical\n"
        );
    }

    #[test]
    fn whitespace_only_changes_are_tagged() {
        let out = render(
            "a",
            "b",
            &[FileDiff::single(text("a  b\n", "a b\n"))],
            &opts(),
        );
        assert!(
            out.contains("summary: text, +1 -1 lines in 1 hunk (whitespace only)\n"),
            "{out}"
        );
    }

    fn sample_folder() -> Vec<FileDiff> {
        vec![
            file(
                Status::Modified,
                "src/lib.rs",
                text("fn a() {}\n", "fn b() {}\n"),
            ),
            file(Status::Added, "docs/new.md", text("", "# New\n\nhello\n")),
            file(Status::Deleted, "old.txt", text("bye\n", "")),
            FileDiff {
                old_path: Some("LICENSE".into()),
                ..file(Status::Renamed, "COPYING", Body::Unchanged)
            },
            file(Status::Modified, "Cargo.lock", text("v = 1\n", "v = 2\n")),
            file(
                Status::Modified,
                "logo.png",
                Body::Binary(diff_bytes(b"\x89PNG\0a".to_vec(), b"\x89PNG\0b".to_vec()).stats()),
            ),
        ]
    }

    #[test]
    fn folders_list_files_first_then_expand_each() {
        insta::assert_snapshot!(render("HEAD^", "HEAD", &sample_folder(), &opts()), @"
        mydiff-ai v1
        compare: HEAD^ -> HEAD
        summary: 6 files changed: 3 modified, 1 added, 1 deleted, 1 renamed; +5 -3 lines
        files:
          M src/lib.rs +1 -1
          A docs/new.md +3
          D old.txt -1
          R LICENSE -> COPYING (content unchanged)
          M Cargo.lock +1 -1 (lockfile; collapsed)
          B logo.png (binary, 1 differing byte, first at 0x5, size 6 -> 6)

        === M src/lib.rs
        @@ src/lib.rs#1 -1,1 +1,1 @@
        1   - fn a() {}
          1 + fn b() {}

        === A docs/new.md
        @@ docs/new.md#1 -0,0 +1,3 @@
          1 + # New
          2 +
          3 + hello

        === D old.txt
        @@ old.txt#1 -1,1 +0,0 @@
        1   - bye
        ");
    }

    #[test]
    fn include_expands_collapsed_files() {
        let files: Vec<FileDiff> = sample_folder()
            .into_iter()
            .filter(|f| f.path == "Cargo.lock")
            .collect();
        let o = AiOptions {
            filtered: true,
            ..opts()
        };
        let out = render("HEAD^", "HEAD", &files, &o);
        assert!(out.contains("=== M Cargo.lock\n@@ Cargo.lock#1"), "{out}");
        assert!(!out.contains("collapsed"), "{out}");
    }

    #[test]
    fn budget_stops_between_hunks_and_says_what_is_missing() {
        let (l, r) = numbered(60, &[5, 25, 45]);
        let (l2, r2) = numbered(3, &[2]);
        let files = vec![
            file(Status::Modified, "big.txt", text(&l, &r)),
            file(Status::Modified, "next.txt", text(&l2, &r2)),
        ];
        let o = AiOptions {
            budget: 20,
            ..opts()
        };
        let out = render("L", "R", &files, &o);
        insta::assert_snapshot!(out, @"
        mydiff-ai v1
        compare: L -> R
        summary: 2 files changed: 2 modified; +4 -4 lines
        files:
          M big.txt +3 -3
          M next.txt +1 -1

        === M big.txt
        @@ big.txt#1 -2,7 +2,7 @@
        2 2   line 2
        3 3   line 3
        4 4   line 4
        5   - line 5
          5 + LINE 5 edited
        6 6   line 6
        7 7   line 7
        8 8   line 8

        truncated: 20-line budget reached; not shown: big.txt#2 to big.txt#3, 1 more file
        more: rerun with --hunk big.txt#2, --include '<path>', or --budget 0 for everything
        ");
        assert!(out.lines().count() <= 20 + 3, "budget plus the trailer");
    }

    #[test]
    fn budget_zero_means_everything() {
        let (l, r) = numbered(200, &(1..=200).step_by(10).collect::<Vec<_>>());
        let o = AiOptions {
            budget: 0,
            ..opts()
        };
        let out = render("L", "R", &[FileDiff::single(text(&l, &r))], &o);
        assert!(!out.contains("truncated"));
        assert!(out.contains("@@ #20 "), "all 20 hunks");
    }

    #[test]
    fn a_first_hunk_bigger_than_the_budget_points_at_its_id() {
        let (l, r) = numbered(40, &(1..=40).collect::<Vec<_>>());
        let o = AiOptions {
            budget: 10,
            ..opts()
        };
        let out = render("L", "R", &[FileDiff::single(text(&l, &r))], &o);
        assert!(
            out.ends_with(
                "truncated: 10-line budget reached; not shown: #1\n\
                 more: rerun with --hunk #1, or --budget 0 for everything\n"
            ),
            "{out}"
        );
    }

    #[rstest::rstest]
    #[case::by_full_id(&["big.txt#2"], &["big.txt#2"])]
    #[case::several(&["big.txt#3", "next.txt#1"], &["big.txt#3", "next.txt#1"])]
    fn hunk_selects_only_those_hunks_in_full(#[case] want: &[&str], #[case] shown: &[&str]) {
        let (l, r) = numbered(60, &[5, 25, 45]);
        let (l2, r2) = numbered(3, &[2]);
        let files = vec![
            file(Status::Modified, "big.txt", text(&l, &r)),
            file(Status::Modified, "next.txt", text(&l2, &r2)),
        ];
        let o = AiOptions {
            budget: 5,
            hunks: want.iter().map(ToString::to_string).collect(),
            ..opts()
        };
        let out = render("L", "R", &files, &o);
        let ids: Vec<&str> = out
            .lines()
            .filter_map(|l| l.strip_prefix("@@ ")?.split(' ').next())
            .collect();
        assert_eq!(ids, shown, "{out}");
        assert!(
            !out.contains("files:") && !out.contains("truncated"),
            "{out}"
        );
    }

    #[rstest::rstest]
    #[case::bare_number("2")]
    #[case::hash_number("#2")]
    fn two_file_hunks_can_be_named_by_number(#[case] want: &str) {
        let (l, r) = numbered(60, &[5, 25, 45]);
        let o = AiOptions {
            hunks: vec![want.into()],
            ..opts()
        };
        let out = render("L", "R", &[FileDiff::single(text(&l, &r))], &o);
        assert!(out.contains("@@ #2 -22,7 +22,7 @@"), "{out}");
        assert!(!out.contains("@@ #1"), "{out}");
    }

    #[test]
    fn unknown_hunk_ids_are_reported() {
        let o = AiOptions {
            hunks: vec!["nope#9".into()],
            ..opts()
        };
        let out = render("L", "R", &sample_folder(), &o);
        assert!(out.ends_with("not found: --hunk nope#9\n"), "{out}");
    }

    #[test]
    fn long_file_lists_stop_at_half_the_budget() {
        let files: Vec<FileDiff> = (0..100)
            .map(|i| file(Status::Added, &format!("f{i:03}.txt"), text("", "x\n")))
            .collect();
        let o = AiOptions {
            budget: 60,
            ..opts()
        };
        let out = render("L", "R", &files, &o);
        assert!(
            out.contains("  A f029.txt +1\n  ... 70 more files"),
            "{out}"
        );
    }

    #[test]
    fn folder_files_read_renames_from_the_old_path() {
        let l = tempfile::tempdir().unwrap();
        let r = tempfile::tempdir().unwrap();
        let write = |root: &Path, rel: &str, body: &[u8]| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        write(l.path(), "util.rs", b"one\ntwo\n");
        write(r.path(), "src/util.rs", b"one\n2\n");
        write(l.path(), "same.txt", b"x\n");
        write(r.path(), "same.txt", b"x\n");
        write(r.path(), "img.bin", b"\0\x01");
        let roots = Roots::ChangedFiles(vec![Rename {
            from: "util.rs".into(),
            to: "src/util.rs".into(),
        }]);
        let opts = DiffOptions::default();
        let diff = diff_roots(l.path(), r.path(), &roots, &opts).unwrap();
        let files = folder_files(&diff, [l.path(), r.path()], &opts, |p| std::fs::read(p));
        let out = render("a", "b", &files, &AiOptions::default());
        insta::assert_snapshot!(out, @"
        mydiff-ai v1
        compare: a -> b
        summary: 2 files changed: 1 added, 1 renamed; +1 -1 lines
        files:
          A img.bin (binary, 2 differing bytes, first at 0x0, size 0 -> 2)
          R util.rs -> src/util.rs +1 -1

        === R util.rs -> src/util.rs
        @@ src/util.rs#1 -1,2 +1,2 @@
        1 1   one
        2   - two
          2 + 2
        ");
    }

    #[test]
    fn folder_files_describe_directories_and_errors() {
        let diff = FolderDiff {
            entries: vec![
                FolderEntry::new("build", true, EntryStatus::OnlyRight),
                FolderEntry::new("x", false, EntryStatus::TypeMismatch),
                FolderEntry::new("y", false, EntryStatus::Error("denied".into())),
            ],
        };
        let files = folder_files(
            &diff,
            [Path::new("L"), Path::new("R")],
            &DiffOptions::default(),
            |_| panic!("nothing to read"),
        );
        let out = render("L", "R", &files, &AiOptions::default());
        assert!(
            out.contains(
                "  A build (directory only on the right)\n  \
                 T x (a file on one side, a directory on the other)\n  \
                 ? y (cannot compare: denied)\n"
            ),
            "{out}"
        );
    }
}
