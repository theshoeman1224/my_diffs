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
            if k.starts_with("GIT_") || k == "NO_COLOR" || k == "CLICOLOR_FORCE" {
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
