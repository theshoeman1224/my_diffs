//! End-to-end: real `git` driving mydiff through the config printed by
//! `mydiff git setup`. Git runs with an isolated config and `HOME`, so the
//! developer's own git settings cannot leak in.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_mydiff");

struct Repo {
    home: tempfile::TempDir,
    dir: PathBuf,
    gitconfig: PathBuf,
}

impl Repo {
    /// HEAD has f.txt, old.json, data.json, conf.yaml and sub/s.txt. The work
    /// tree (also staged, so renames and new files are tracked) then modifies
    /// f.txt, conf.yaml and "my notes.txt", deletes old.json, adds n.txt, renames sub/s.txt to
    /// sub/u.txt with an edit, and breaks data.json.
    fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join("repo");
        std::fs::create_dir(&dir).unwrap();
        let gitconfig = home.path().join("gitconfig");
        std::fs::write(&gitconfig, setup_snippet()).unwrap();
        let repo = Self {
            home,
            dir,
            gitconfig,
        };
        repo.git(&["init", "-q", "-b", "main"]);
        repo.write("f.txt", "alpha\nbeta\ngamma\n");
        repo.write("old.json", "{\"gone\": true}\n");
        repo.write("data.json", "{\"ok\": 1}\n");
        repo.write("conf.yaml", "name: web\nport: 80\n");
        repo.write("my notes.txt", "draft\n");
        repo.write("sub/s.txt", "one\ntwo\nthree\nfour\n");
        repo.git(&["add", "-A"]);
        repo.git(&["commit", "-q", "-m", "base"]);
        repo.write("f.txt", "alpha\nbeta changed\ngamma\n");
        std::fs::remove_file(repo.dir.join("old.json")).unwrap();
        repo.write("n.txt", "new file\n");
        std::fs::rename(repo.dir.join("sub/s.txt"), repo.dir.join("sub/u.txt")).unwrap();
        repo.write("sub/u.txt", "one\ntwo\nthree\nfour\nfive\n");
        repo.write("data.json", "{\"ok\": \n");
        repo.write("conf.yaml", "port: 8080\nname: web\n");
        repo.write("my notes.txt", "final\n");
        repo.git(&["add", "-A"]);
        repo
    }

    fn write(&self, rel: &str, body: &str) {
        let p = self.dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    fn cmd(&self, program: &str, args: &[&str]) -> Command {
        let mut c = Command::new(program);
        c.args(args).current_dir(&self.dir);
        for (k, _) in std::env::vars_os() {
            let k = k.to_string_lossy().into_owned();
            if k.starts_with("GIT_")
                || k.starts_with("MYDIFF_")
                || k == "NO_COLOR"
                || k == "CLICOLOR_FORCE"
            {
                c.env_remove(k);
            }
        }
        let bin_dir = Path::new(BIN).parent().unwrap();
        let path = std::env::join_paths(std::iter::once(bin_dir.to_path_buf()).chain(
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
        ))
        .unwrap();
        c.env("PATH", path)
            .env("HOME", self.home.path())
            .env("XDG_CONFIG_HOME", "/nonexistent/mydiff-tests")
            .env("XDG_STATE_HOME", "/nonexistent/mydiff-tests")
            .env("GIT_CONFIG_GLOBAL", &self.gitconfig)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .env("LC_ALL", "C")
            .env("COLUMNS", "80");
        c
    }

    fn git(&self, args: &[&str]) -> Output {
        let out = self.cmd("git", args).env("NO_COLOR", "1").output().unwrap();
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        out
    }
}

fn setup_snippet() -> String {
    let out = Command::new(BIN).args(["git", "setup"]).output().unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap()
}

fn stdout(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).unwrap()
}

#[test]
fn difftool_shows_each_file_with_repo_labels() {
    let repo = Repo::new();
    let out = repo.git(&["difftool", "HEAD"]);
    insta::assert_snapshot!(stdout(&out));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("could not parse right input as json")
            && stderr.contains("showing a text diff"),
        "broken JSON falls back to text with a note: {stderr}"
    );
}

#[test]
fn difftool_dir_diff_compares_whole_trees() {
    let repo = Repo::new();
    // Against the work tree, git fills the right-hand temp dir with symlinks.
    let out = repo.git(&["difftool", "-d", "HEAD"]);
    let text = stdout(&out);
    assert!(
        !text.contains("git-difftool"),
        "temp dirs are labelled a/ and b/:\n{text}"
    );
    insta::assert_snapshot!(text);
}

#[test]
fn external_diff_covers_every_file_and_exits_zero() {
    let repo = Repo::new();
    let out = repo
        .cmd("git", &["diff", "HEAD"])
        .env("NO_COLOR", "1")
        .env("GIT_EXTERNAL_DIFF", "mydiff git external")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git must not see a failing external diff: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    insta::assert_snapshot!(stdout(&out));
}

#[test]
fn optional_diff_external_lines_work_when_uncommented() {
    let repo = Repo::new();
    let snippet = setup_snippet();
    assert!(snippet.contains("#[diff]\n#\texternal = mydiff git external\n"));
    let enabled = snippet.replace("#[diff]\n#\texternal", "[diff]\n\texternal");
    std::fs::write(&repo.gitconfig, enabled).unwrap();
    let text = stdout(&repo.git(&["diff", "HEAD", "--", "f.txt"]));
    assert!(
        text.starts_with("diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n"),
        "{text}"
    );
    let plain = stdout(&repo.git(&["diff", "--no-ext-diff", "HEAD", "--", "f.txt"]));
    assert!(
        plain.contains("index "),
        "--no-ext-diff bypasses mydiff: {plain}"
    );
}

#[test]
fn external_diff_colors_when_git_uses_a_pager() {
    let repo = Repo::new();
    let run = |pager: bool| {
        let mut c = repo.cmd(
            BIN,
            &[
                "git",
                "external",
                "f.txt",
                "/dev/null",
                ".",
                ".",
                "f.txt",
                "0",
                "100644",
            ],
        );
        if pager {
            c.env("GIT_PAGER_IN_USE", "true");
        }
        let out = c.output().unwrap();
        assert!(out.status.success());
        stdout(&out)
    };
    assert!(run(true).contains("\x1b["), "colored for git's pager");
    assert!(!run(false).contains("\x1b["), "plain when piped elsewhere");
}

impl Repo {
    /// Run mydiff in the repository; any exit status.
    fn mydiff(&self, args: &[&str]) -> Output {
        self.cmd(BIN, args).env("NO_COLOR", "1").output().unwrap()
    }

    /// Commit what `new()` staged: HEAD is then the commit with every kind of change.
    fn committed() -> Self {
        let repo = Self::new();
        repo.git(&["commit", "-q", "-m", "change everything"]);
        repo
    }
}

fn status_and_stdout(out: &Output) -> (Option<i32>, String) {
    (out.status.code(), stdout(out))
}

#[test]
fn git_show_lists_every_file_of_the_commit_with_renames_paired() {
    let repo = Repo::committed();
    let (code, text) = status_and_stdout(&repo.mydiff(&["git", "show", "-f", "summary"]));
    assert_eq!(code, Some(1));
    insta::assert_snapshot!(text, @"
    ~ conf.yaml
    ~ data.json
    ~ f.txt
    ~ my notes.txt
    + n.txt
    - old.json
    ~ sub/s.txt -> sub/u.txt
    HEAD^ vs HEAD: 5 different, 1 only in left, 1 only in right, 1 renamed (0 same)
    ");
}

#[test]
fn git_show_piped_prints_a_patch_per_file() {
    let repo = Repo::committed();
    let (code, text) = status_and_stdout(&repo.mydiff(&["git", "show"]));
    assert_eq!(code, Some(1));
    insta::assert_snapshot!(text, @r#"
    --- HEAD^/conf.yaml
    +++ HEAD/conf.yaml
    @@ -1,2 +1,2 @@
    +port: 8080
     name: web
    -port: 80
    --- HEAD^/data.json
    +++ HEAD/data.json
    @@ -1,1 +1,1 @@
    -{"ok": 1}
    +{"ok": 
    --- HEAD^/f.txt
    +++ HEAD/f.txt
    @@ -1,3 +1,3 @@
     alpha
    -beta
    +beta changed
     gamma
    --- HEAD^/my notes.txt
    +++ HEAD/my notes.txt
    @@ -1,1 +1,1 @@
    -draft
    +final
    --- /dev/null
    +++ HEAD/n.txt
    @@ -0,0 +1,1 @@
    +new file
    --- HEAD^/old.json
    +++ /dev/null
    @@ -1,1 +0,0 @@
    -{"gone": true}
    --- HEAD^/sub/s.txt
    +++ HEAD/sub/u.txt
    @@ -2,3 +2,4 @@
     two
     three
     four
    +five
    "#);
}

#[test]
fn git_show_json_names_the_old_path_of_a_rename() {
    let repo = Repo::committed();
    let out = repo.mydiff(&["git", "show", "-f", "json"]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["left"], "HEAD^");
    assert_eq!(v["right"], "HEAD");
    let renamed: Vec<_> = v["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e.get("renamed_from").is_some())
        .map(|e| format!("{} -> {} ({})", e["renamed_from"], e["path"], e["status"]))
        .collect();
    assert_eq!(renamed, [r#""sub/s.txt" -> "sub/u.txt" ("different")"#]);
    assert_eq!(v["stats"]["renamed"], 1);
}

#[test]
fn git_show_of_the_root_commit_adds_every_file() {
    let repo = Repo::committed();
    let (code, text) = status_and_stdout(&repo.mydiff(&["git", "show", "HEAD~1", "-f", "summary"]));
    assert_eq!(code, Some(1));
    insta::assert_snapshot!(text, @"
    + conf.yaml
    + data.json
    + f.txt
    + my notes.txt
    + old.json
    + sub/s.txt
    (empty) vs HEAD~1: 6 only in right (0 same)
    ");
}

#[test]
fn git_show_limits_to_paths_after_double_dash() {
    let repo = Repo::committed();
    let (_, text) = status_and_stdout(&repo.mydiff(&["git", "show", "-f", "summary", "--", "sub"]));
    insta::assert_snapshot!(text, @"
    ~ sub/s.txt -> sub/u.txt
    HEAD^ vs HEAD: 1 different, 1 renamed (0 same)
    ");
}

#[test]
fn git_diff_forms_select_the_same_changes_as_git_diff() {
    let repo = Repo::new();
    let summary = |args: &[&str]| {
        let mut all = vec!["git", "diff", "-f", "summary"];
        all.extend(args);
        let out = repo.mydiff(&all);
        assert!(
            out.status.code() != Some(2),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        stdout(&out)
    };
    let staged = summary(&["--staged"]);
    assert!(
        staged.ends_with(
            "HEAD vs index: 5 different, 1 only in left, 1 only in right, 1 renamed (0 same)\n"
        ),
        "{staged}"
    );
    assert_eq!(summary(&["--cached"]), staged);
    assert_eq!(
        summary(&[]),
        "index vs worktree: identical (0 same)\n",
        "everything is staged"
    );
    repo.write("f.txt", "unstaged edit\n");
    insta::assert_snapshot!(summary(&[]), @"
    ~ f.txt
    index vs worktree: 1 different (0 same)
    ");
    repo.git(&["commit", "-q", "-m", "c2"]);
    let show = repo.mydiff(&["git", "show", "-f", "summary"]);
    let body = |s: &str| {
        s.lines()
            .filter(|l| !l.contains(" vs "))
            .collect::<Vec<_>>()
            .join("\n")
    };
    for range in [
        &["HEAD~1", "HEAD"][..],
        &["HEAD~1..HEAD"],
        &["HEAD~1...HEAD"],
    ] {
        assert_eq!(body(&summary(range)), body(&stdout(&show)), "{range:?}");
    }
    assert!(summary(&["HEAD~1...HEAD"]).contains(" vs HEAD:"));
}

#[test]
fn git_diff_reads_the_work_tree_side_from_disk() {
    let repo = Repo::committed();
    repo.write("f.txt", "alpha\nbeta changed\ngamma\ndelta\n");
    let (code, text) = status_and_stdout(&repo.mydiff(&["git", "diff"]));
    assert_eq!(code, Some(1));
    insta::assert_snapshot!(text, @"
    --- index/f.txt
    +++ worktree/f.txt
    @@ -1,3 +1,4 @@
     alpha
     beta changed
     gamma
    +delta
    ");
}

#[test]
fn git_diff_without_changes_exits_zero() {
    let repo = Repo::committed();
    let (code, text) = status_and_stdout(&repo.mydiff(&["git", "diff"]));
    assert_eq!((code, text.as_str()), (Some(0), ""));
}

#[test]
fn git_show_and_diff_report_errors() {
    let repo = Repo::committed();
    let err = |args: &[&str], dir: &Path| {
        let out = repo.cmd(BIN, args).current_dir(dir).output().unwrap();
        assert_eq!(out.status.code(), Some(2));
        String::from_utf8_lossy(&out.stderr).into_owned()
    };
    let msg = err(&["git", "show", "nope"], &repo.dir);
    assert!(msg.contains("nope is not a commit"), "{msg}");
    let msg = err(&["git", "diff", "--staged", "a", "b"], &repo.dir);
    assert!(msg.contains("at most one revision"), "{msg}");
    let outside = repo.home.path();
    let msg = err(&["git", "show"], outside);
    assert!(msg.contains("not a git repository"), "{msg}");
    let msg = err(&["git", "show", "--ignore", "("], &repo.dir);
    assert!(msg.contains("regex"), "options are checked first: {msg}");
}

#[test]
fn agents_get_ai_output_for_a_commit_from_the_environment() {
    let repo = Repo::committed();
    let out = repo
        .cmd(BIN, &["git", "show"])
        .env("MYDIFF_AI", "1")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    insta::assert_snapshot!(stdout(&out), @r#"
    mydiff-ai v1
    compare: HEAD^ -> HEAD
    summary: 7 files changed: 4 modified, 1 added, 1 deleted, 1 renamed; +6 -5 lines
    files:
      M conf.yaml +1 -1
      M data.json +1 -1
      M f.txt +1 -1
      M my notes.txt +1 -1
      A n.txt +1
      D old.json -1
      R sub/s.txt -> sub/u.txt +1

    === M conf.yaml
    @@ conf.yaml#1 -1,2 +1,2 @@
      1 + port: 8080
    1 2   name: web
    2   - port: 80

    === M data.json
    @@ data.json#1 -1,1 +1,1 @@
    1   - {"ok": 1}
      1 + {"ok": 

    === M f.txt
    @@ f.txt#1 -1,3 +1,3 @@
    1 1   alpha
    2   - beta
      2 + beta changed
    3 3   gamma

    === M my notes.txt
    @@ my notes.txt#1 -1,1 +1,1 @@
    1   - draft
      1 + final

    === A n.txt
    @@ n.txt#1 -0,0 +1,1 @@
      1 + new file

    === D old.json
    @@ old.json#1 -1,1 +0,0 @@
    1   - {"gone": true}

    === R sub/s.txt -> sub/u.txt
    @@ sub/u.txt#1 -2,3 +2,4 @@
    2 2   two
    3 3   three
    4 4   four
      5 + five
    "#);
    let out = repo
        .cmd(BIN, &["git", "show", "--hunk", "sub/u.txt#1"])
        .env("MYDIFF_AI_BUDGET", "3")
        .output()
        .unwrap();
    assert!(
        stdout(&out).ends_with(
            "@@ sub/u.txt#1 -2,3 +2,4 @@\n2 2   two\n3 3   three\n4 4   four\n  5 + five\n"
        ),
        "an asked-for hunk is shown in full whatever the budget:\n{}",
        stdout(&out)
    );
}

#[test]
fn git_external_diff_ignores_mydiff_ai_so_git_diff_stays_a_patch() {
    let repo = Repo::new();
    let out = repo
        .cmd("git", &["diff", "--cached"])
        .env("GIT_EXTERNAL_DIFF", "mydiff git external")
        .env("MYDIFF_AI", "1")
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    let text = stdout(&out);
    assert!(text.starts_with("diff --git "), "{text}");
    assert!(!text.contains("mydiff-ai"), "{text}");
}
