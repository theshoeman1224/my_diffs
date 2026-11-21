//! `mydiff git show` and `mydiff git diff`: every file a commit (or any
//! `git diff`) changed, opened together in the folder view.
//!
//! Git lists the changes (`git diff --raw -z`). Their old and new contents go
//! into two temp dirs, blobs through one `git cat-file --batch` and the work
//! tree side straight from disk. The folder comparison then runs on those dirs
//! with [`Roots::ChangedFiles`], so every file shows by its full path and a
//! rename is a single entry.

use std::collections::{BTreeSet, HashMap};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use mydiff_core::folder::{Rename, Roots};

use crate::app::{Caller, Job};

/// What to compare: the revision arguments for `git diff` and a name per side.
#[derive(Debug, PartialEq, Eq)]
pub struct Spec {
    pub diff_args: Vec<String>,
    pub labels: [String; 2],
}

/// Name of the left side of a root commit, which has no parent.
pub const NO_PARENT: &str = "(empty)";

/// Run git and return its stdout, or fail with what git printed.
fn git(args: &[&str]) -> Result<Vec<u8>> {
    let out = Command::new("git")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .context("running git (is it installed and on PATH?)")?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr);
        bail!("git {} failed: {}", args[0], msg.trim());
    }
    Ok(out.stdout)
}

fn git_line(args: &[&str]) -> Result<String> {
    Ok(String::from_utf8_lossy(&git(args)?).trim_end().to_owned())
}

/// The work tree's top directory; also checks that we are in a repository.
pub fn toplevel() -> Result<PathBuf> {
    git_line(&["rev-parse", "--show-toplevel"]).map(PathBuf::from)
}

/// Revisions are passed to git as arguments, so they must not look like options.
fn check_rev(rev: &str) -> Result<()> {
    if rev.starts_with('-') {
        bail!("{rev:?} is not a revision (put paths after `--`)");
    }
    Ok(())
}

/// `mydiff git show [<commit>]`: the commit against its first parent.
pub fn show_spec(commit: Option<&str>) -> Result<Spec> {
    let name = commit.unwrap_or("HEAD");
    check_rev(name)?;
    let oid = git_line(&[
        "rev-parse",
        "--verify",
        "--quiet",
        &format!("{name}^{{commit}}"),
    ])
    .with_context(|| format!("{name} is not a commit"))?;
    let (parent, left) = match git_line(&["rev-parse", "--verify", "--quiet", &format!("{oid}^")]) {
        Ok(parent) => (parent, format!("{name}^")),
        // A root commit: compare with the empty tree (its id depends on the hash).
        Err(_) => (
            git_line(&["hash-object", "-t", "tree", "--stdin"])?,
            NO_PARENT.to_owned(),
        ),
    };
    Ok(Spec {
        diff_args: vec![parent, oid],
        labels: [left, name.to_owned()],
    })
}

/// Short id of the merge base of `a` and `b`, which `a...b` compares from.
pub fn merge_base_label(a: &str, b: &str) -> Result<String> {
    let base = git_line(&["merge-base", a, b])?;
    git_line(&["rev-parse", "--short", &base])
}

/// `mydiff git diff [--staged] [<revs>]`, read the way `git diff` reads them.
pub fn diff_spec(
    staged: bool,
    revs: &[String],
    merge_base: impl Fn(&str, &str) -> Result<String>,
) -> Result<Spec> {
    for r in revs {
        check_rev(r)?;
        if staged && r.contains("..") {
            bail!("--staged compares the index with one revision, not a range");
        }
    }
    let or_head = |s: &str| if s.is_empty() { "HEAD" } else { s }.to_owned();
    let labels = match revs {
        [] if staged => ["HEAD".to_owned(), "index".to_owned()],
        [] => ["index".to_owned(), "worktree".to_owned()],
        [r] => {
            if let Some((a, b)) = r.split_once("...") {
                [merge_base(&or_head(a), &or_head(b))?, or_head(b)]
            } else if let Some((a, b)) = r.split_once("..") {
                [or_head(a), or_head(b)]
            } else {
                let other = if staged { "index" } else { "worktree" };
                [r.clone(), other.to_owned()]
            }
        }
        [a, b] if !staged => [a.clone(), b.clone()],
        _ if staged => bail!("--staged takes at most one revision"),
        _ => bail!("give at most two revisions"),
    };
    let mut diff_args: Vec<String> = staged.then(|| "--cached".to_owned()).into_iter().collect();
    diff_args.extend(revs.iter().cloned());
    Ok(Spec { diff_args, labels })
}

/// One side of a change in `git diff --raw` output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blob {
    pub mode: String,
    pub oid: String,
}

impl Blob {
    fn absent(&self) -> bool {
        self.mode.bytes().all(|b| b == b'0')
    }

    /// Git leaves the id zero when the content is in the work tree, not hashed.
    fn in_worktree(&self) -> bool {
        !self.absent() && self.oid.bytes().all(|b| b == b'0')
    }

    /// A gitlink: the id is a commit in another repository.
    fn is_submodule(&self) -> bool {
        self.mode == "160000"
    }
}

/// One line of `git diff --raw`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// `A`, `D`, `M`, `R`, `C`, `T`, `U` or `X` (rename and copy scores dropped).
    pub status: char,
    pub old: Blob,
    pub new: Blob,
    pub old_path: String,
    pub new_path: String,
}

/// Parse `git diff --raw -z` output: `:<old mode> <new mode> <old id> <new id>
/// <status>` NUL `<path>` NUL, with a second path for renames and copies.
pub fn parse_raw(raw: &[u8]) -> Result<Vec<Change>> {
    let mut fields = raw.split(|&b| b == 0);
    let mut out = Vec::new();
    while let Some(head) = fields.next() {
        if head.is_empty() {
            continue;
        }
        let text = String::from_utf8_lossy(head);
        let parts: Vec<&str> = text.strip_prefix(':').unwrap_or("").split(' ').collect();
        let [old_mode, new_mode, old_oid, new_oid, status] = parts[..] else {
            bail!("unexpected `git diff --raw` output: {text:?}");
        };
        let status = status.chars().next().context("empty status")?;
        let mut path = || {
            fields
                .next()
                .filter(|p| !p.is_empty())
                .map(|p| String::from_utf8_lossy(p).into_owned())
                .with_context(|| {
                    format!("`git diff --raw` output is missing a path after {text:?}")
                })
        };
        let old_path = path()?;
        let new_path = if matches!(status, 'R' | 'C') {
            path()?
        } else {
            old_path.clone()
        };
        let blob = |mode: &str, oid: &str| Blob {
            mode: mode.to_owned(),
            oid: oid.to_owned(),
        };
        out.push(Change {
            status,
            old: blob(old_mode, old_oid),
            new: blob(new_mode, new_oid),
            old_path,
            new_path,
        });
    }
    Ok(out)
}

/// Contents of `oids` from `git cat-file --batch`.
fn cat_blobs(oids: &[&str]) -> Result<HashMap<String, Vec<u8>>> {
    if oids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut child = Command::new("git")
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("running git cat-file")?;
    let mut stdin = child.stdin.take().context("git cat-file stdin")?;
    let input: String = oids.iter().map(|o| format!("{o}\n")).collect();
    // Write from another thread: git answers while we are still asking.
    let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let mut out = Vec::new();
    child
        .stdout
        .take()
        .context("git cat-file stdout")?
        .read_to_end(&mut out)
        .context("reading from git cat-file")?;
    writer
        .join()
        .map_err(|_| anyhow::anyhow!("writing to git cat-file panicked"))?
        .context("writing to git cat-file")?;
    if !child.wait()?.success() {
        bail!("git cat-file failed");
    }
    parse_batch(&out, oids)
}

/// Split `git cat-file --batch` output (`<id> <type> <size>` LF `<bytes>` LF,
/// or `<id> missing` LF) into the contents of each requested id.
pub fn parse_batch(mut out: &[u8], oids: &[&str]) -> Result<HashMap<String, Vec<u8>>> {
    let mut blobs = HashMap::new();
    for oid in oids {
        let eol = out
            .iter()
            .position(|&b| b == b'\n')
            .context("git cat-file output ended early")?;
        let header = String::from_utf8_lossy(&out[..eol]).into_owned();
        out = &out[eol + 1..];
        let size: usize = match header.split(' ').collect::<Vec<_>>()[..] {
            [_, _, size] => size
                .parse()
                .with_context(|| format!("bad size in {header:?}"))?,
            _ => bail!("git could not read object {oid}: {header}"),
        };
        let body = out
            .get(..size)
            .with_context(|| format!("git cat-file output for {oid} is truncated"))?;
        blobs.insert((*oid).to_owned(), body.to_vec());
        out = out.get(size + 1..).unwrap_or_default();
    }
    Ok(blobs)
}

/// Write `bytes` to `root/rel`. Git paths are relative and never contain
/// `..`, but a path that tries to leave `root` is refused, not followed.
pub fn write_under(root: &Path, rel: &str, bytes: &[u8]) -> Result<()> {
    let rel = Path::new(rel);
    if rel.as_os_str().is_empty() || !rel.components().all(|c| matches!(c, Component::Normal(_))) {
        bail!("refusing to write unsafe path {}", rel.display());
    }
    let path = root.join(rel);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))
}

/// A work tree file as git sees it: a symlink is its target, a submodule a note.
fn read_worktree(path: &Path) -> Result<Vec<u8>> {
    let meta =
        std::fs::symlink_metadata(path).with_context(|| format!("reading {}", path.display()))?;
    if meta.file_type().is_symlink() {
        let target = std::fs::read_link(path)?;
        return Ok(target.to_string_lossy().into_owned().into_bytes());
    }
    std::fs::read(path).with_context(|| format!("reading {}", path.display()))
}

/// Write each change's old content under `left` and new content under `right`
/// (at the old and new path), and return the renames. `fetch` reads blobs by id.
pub fn materialize(
    changes: &[Change],
    worktree: &Path,
    [left, right]: [&Path; 2],
    fetch: impl FnOnce(&[&str]) -> Result<HashMap<String, Vec<u8>>>,
) -> Result<Vec<Rename>> {
    let stored = |b: &Blob| !b.absent() && !b.in_worktree() && !b.is_submodule();
    let mut wanted = BTreeSet::new();
    for c in changes.iter().filter(|c| c.status != 'U') {
        // A copy's source is unchanged, so only its new file is shown.
        if c.status != 'C' && stored(&c.old) {
            wanted.insert(c.old.oid.as_str());
        }
        if stored(&c.new) {
            wanted.insert(c.new.oid.as_str());
        }
    }
    let wanted: Vec<&str> = wanted.into_iter().collect();
    let blobs = fetch(&wanted)?;
    let content = |b: &Blob, path: &str| -> Result<Vec<u8>> {
        if b.is_submodule() {
            let id = if b.in_worktree() {
                "(work tree)"
            } else {
                &b.oid
            };
            return Ok(format!("Subproject commit {id}\n").into_bytes());
        }
        if b.in_worktree() {
            return read_worktree(&worktree.join(path));
        }
        blobs
            .get(&b.oid)
            .cloned()
            .with_context(|| format!("git did not return {} ({path})", b.oid))
    };
    let mut renames = Vec::new();
    for c in changes {
        if c.status == 'U' {
            eprintln!("mydiff: note: skipping unmerged {}", c.new_path);
            continue;
        }
        if c.status != 'C' && !c.old.absent() {
            write_under(left, &c.old_path, &content(&c.old, &c.old_path)?)?;
        }
        if !c.new.absent() {
            write_under(right, &c.new_path, &content(&c.new, &c.new_path)?)?;
        }
        if c.status == 'R' {
            renames.push(Rename {
                from: c.old_path.clone(),
                to: c.new_path.clone(),
            });
        }
    }
    Ok(renames)
}

/// Write out the changes `spec` selects (limited to `paths`) and build the
/// folder comparison job. The job's roots live in the returned temp dir.
pub fn job(top: &Path, spec: Spec, paths: &[String]) -> Result<(tempfile::TempDir, Job)> {
    let mut args = vec![
        "diff",
        "--raw",
        "-z",
        "-M",
        "--no-abbrev",
        "--no-ext-diff",
        "--no-color",
        "--no-relative",
    ];
    args.extend(spec.diff_args.iter().map(String::as_str));
    args.push("--");
    args.extend(paths.iter().map(String::as_str));
    let changes = parse_raw(&git(&args)?)?;
    let tmp = tempfile::Builder::new()
        .prefix("mydiff-git-")
        .tempdir()
        .context("creating a temp dir")?;
    let (left, right) = (tmp.path().join("left"), tmp.path().join("right"));
    for dir in [&left, &right] {
        std::fs::create_dir(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let renames = materialize(&changes, top, [&left, &right], cat_blobs)?;
    let [l, r] = spec.labels;
    let job = Job {
        left,
        right,
        labels: [Some(l), Some(r)],
        preamble: None,
        caller: Caller::GitDifftool,
        roots: Roots::ChangedFiles(renames),
    };
    Ok((tmp, job))
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    const A: &str = "1111111111111111111111111111111111111111";
    const B: &str = "2222222222222222222222222222222222222222";
    const Z: &str = "0000000000000000000000000000000000000000";

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    fn no_merge_base(_: &str, _: &str) -> Result<String> {
        bail!("not asked")
    }

    #[rstest]
    #[case::worktree_vs_index(false, &[], &[], ["index", "worktree"])]
    #[case::staged(true, &[], &["--cached"], ["HEAD", "index"])]
    #[case::staged_vs_rev(true, &["main"], &["--cached", "main"], ["main", "index"])]
    #[case::worktree_vs_rev(false, &["HEAD~2"], &["HEAD~2"], ["HEAD~2", "worktree"])]
    #[case::two_revs(false, &["a", "b"], &["a", "b"], ["a", "b"])]
    #[case::range(false, &["a..b"], &["a..b"], ["a", "b"])]
    #[case::open_range(false, &["main.."], &["main.."], ["main", "HEAD"])]
    fn diff_spec_reads_revisions_like_git(
        #[case] staged: bool,
        #[case] revs: &[&str],
        #[case] args: &[&str],
        #[case] labels: [&str; 2],
    ) {
        let spec = diff_spec(staged, &strings(revs), no_merge_base).unwrap();
        assert_eq!(spec.diff_args, strings(args));
        assert_eq!(spec.labels, labels.map(str::to_owned));
    }

    #[test]
    fn three_dot_range_starts_at_the_merge_base() {
        let spec = diff_spec(false, &strings(&["main...topic"]), |a, b| {
            Ok(format!("base({a},{b})"))
        })
        .unwrap();
        assert_eq!(
            spec.labels,
            ["base(main,topic)", "topic"].map(str::to_owned)
        );
        assert_eq!(spec.diff_args, ["main...topic"]);
    }

    #[rstest]
    #[case::option_like(false, &["--output=x"], "is not a revision")]
    #[case::staged_two(true, &["a", "b"], "at most one revision")]
    #[case::staged_range(true, &["a..b"], "not a range")]
    fn diff_spec_rejects(#[case] staged: bool, #[case] revs: &[&str], #[case] msg: &str) {
        let err = diff_spec(staged, &strings(revs), no_merge_base).unwrap_err();
        assert!(err.to_string().contains(msg), "{err}");
    }

    fn raw(records: &[&str]) -> Vec<u8> {
        records.join("\0").into_bytes()
    }

    #[test]
    fn parse_raw_reads_every_kind_of_change() {
        let out = raw(&[
            &format!(":100644 100644 {A} {B} M"),
            "src/a b.rs",
            &format!(":000000 100644 {Z} {B} A"),
            "new.txt",
            &format!(":100644 000000 {A} {Z} D"),
            "gone.txt",
            &format!(":100644 100644 {A} {B} R087"),
            "old/name.rs",
            "new/name.rs",
            &format!(":100644 100644 {A} {Z} M"),
            "edited-in-worktree.txt",
            "",
        ]);
        let changes = parse_raw(&out).unwrap();
        let summary: Vec<String> = changes
            .iter()
            .map(|c| format!("{} {} -> {}", c.status, c.old_path, c.new_path))
            .collect();
        assert_eq!(
            summary,
            [
                "M src/a b.rs -> src/a b.rs",
                "A new.txt -> new.txt",
                "D gone.txt -> gone.txt",
                "R old/name.rs -> new/name.rs",
                "M edited-in-worktree.txt -> edited-in-worktree.txt",
            ]
        );
        assert!(changes[1].old.absent() && !changes[1].new.absent());
        assert!(changes[4].new.in_worktree() && !changes[4].old.in_worktree());
    }

    #[rstest]
    #[case::not_raw(b"diff --git a/x b/x\0" as &[u8], "unexpected")]
    #[case::missing_path(b":100644 100644 1 2 M\0" as &[u8], "missing a path")]
    #[case::rename_missing_new_path(b":100644 100644 1 2 R100\0old\0" as &[u8], "missing a path")]
    fn parse_raw_rejects(#[case] out: &[u8], #[case] msg: &str) {
        let err = parse_raw(out).unwrap_err();
        assert!(err.to_string().contains(msg), "{err}");
    }

    #[test]
    fn parse_batch_splits_objects_including_newlines_in_content() {
        let out = format!("{A} blob 6\na\nb\nc\n\n{B} blob 0\n\n");
        let blobs = parse_batch(out.as_bytes(), &[A, B]).unwrap();
        assert_eq!(blobs[A], b"a\nb\nc\n");
        assert_eq!(blobs[B], b"");
        let err = parse_batch(format!("{A} missing\n").as_bytes(), &[A]).unwrap_err();
        assert!(err.to_string().contains("could not read object"), "{err}");
    }

    #[rstest]
    #[case::parent("../escape")]
    #[case::absolute("/etc/passwd")]
    #[case::inner_parent("a/../../b")]
    #[case::empty("")]
    fn write_under_refuses_paths_that_leave_the_root(#[case] rel: &str) {
        let dir = tempfile::tempdir().unwrap();
        let err = write_under(dir.path(), rel, b"x").unwrap_err();
        assert!(err.to_string().contains("unsafe path"), "{err}");
    }

    fn change(status: char, old: (&str, &str), new: (&str, &str), paths: [&str; 2]) -> Change {
        let blob = |(mode, oid): (&str, &str)| Blob {
            mode: mode.into(),
            oid: oid.into(),
        };
        Change {
            status,
            old: blob(old),
            new: blob(new),
            old_path: paths[0].into(),
            new_path: paths[1].into(),
        }
    }

    fn tree(root: &Path) -> Vec<String> {
        let mut files: Vec<String> = walk(root)
            .into_iter()
            .map(|p| {
                let rel = p.strip_prefix(root).unwrap().display().to_string();
                format!("{rel}={}", std::fs::read_to_string(&p).unwrap().trim_end())
            })
            .collect();
        files.sort();
        files
    }

    fn walk(dir: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .unwrap()
            .flat_map(|e| {
                let p = e.unwrap().path();
                if p.is_dir() { walk(&p) } else { vec![p] }
            })
            .collect()
    }

    #[test]
    fn materialize_writes_old_and_new_sides_and_returns_renames() {
        let tmp = tempfile::tempdir().unwrap();
        let (wt, l, r) = (
            tmp.path().join("wt"),
            tmp.path().join("l"),
            tmp.path().join("r"),
        );
        for d in [&wt, &l, &r] {
            std::fs::create_dir(d).unwrap();
        }
        std::fs::write(wt.join("edited.txt"), "from disk\n").unwrap();
        let changes = [
            change('M', ("100644", A), ("100644", B), ["src/m.rs", "src/m.rs"]),
            change('A', ("000000", Z), ("100644", B), ["deep/new/n.txt"; 2]),
            change('D', ("100644", A), ("000000", Z), ["gone.txt"; 2]),
            change('R', ("100644", A), ("100644", A), ["old.rs", "lib/new.rs"]),
            change('M', ("100644", A), ("100644", Z), ["edited.txt"; 2]),
            change('M', ("160000", A), ("160000", B), ["vendor/sub"; 2]),
            change('U', ("000000", Z), ("000000", Z), ["conflict.txt"; 2]),
        ];
        let mut asked = Vec::new();
        let renames = materialize(&changes, &wt, [&l, &r], |oids| {
            asked = oids.iter().map(|o| (*o).to_owned()).collect();
            Ok(HashMap::from([
                (A.to_owned(), b"old\n".to_vec()),
                (B.to_owned(), b"new\n".to_vec()),
            ]))
        })
        .unwrap();
        assert_eq!(
            asked,
            [A, B],
            "each blob fetched once; submodules not at all"
        );
        assert_eq!(
            renames,
            [Rename {
                from: "old.rs".into(),
                to: "lib/new.rs".into()
            }]
        );
        assert_eq!(
            tree(&l),
            [
                "edited.txt=old",
                "gone.txt=old",
                "old.rs=old",
                "src/m.rs=old",
                &format!("vendor/sub=Subproject commit {A}"),
            ]
        );
        assert_eq!(
            tree(&r),
            [
                "deep/new/n.txt=new",
                "edited.txt=from disk",
                "lib/new.rs=old",
                "src/m.rs=new",
                &format!("vendor/sub=Subproject commit {B}"),
            ]
        );
    }
}
