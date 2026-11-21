//! Git entry points: `mydiff git setup | difftool | external`.
//!
//! Git hands external tools temp files (`/tmp/git-blob-XXXX/name`) and uses
//! `/dev/null` for a missing side. These adapters turn that into a [`Job`]
//! with repository-relative labels (`a/path`, `b/path`), like `git diff`.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use crate::app::{self, Caller, Job, Outcome};
use crate::args::{GitCommand, RunArgs};

/// Printed by `mydiff git setup`; meant to be appended to `~/.gitconfig`.
/// It is also what the end-to-end tests install, so it is known to work.
pub const GITCONFIG: &str = r#"# mydiff as git's diff viewer. Add it to your git config with:
#     mydiff git setup >> ~/.gitconfig
#
#   git difftool [<commit>] [-- <path>]   each changed file in the viewer
#   git difftool -d [<commit>]            all changed files in the folder view
[diff]
	tool = mydiff
[difftool]
	prompt = false
[difftool "mydiff"]
	cmd = mydiff git difftool \"$LOCAL\" \"$REMOTE\" \"$MERGED\"
#
# Optional: make `git diff` and `git show` print through mydiff as well
# (bypass with `git diff --no-ext-diff`). To try it once instead:
#     GIT_EXTERNAL_DIFF='mydiff git external' git diff
#[diff]
#	external = mydiff git external
"#;

const DEV_NULL: &str = "/dev/null";

fn is_dev_null(p: &Path) -> bool {
    p.as_os_str() == DEV_NULL
}

pub fn run(cmd: GitCommand) -> Result<Outcome> {
    match cmd {
        GitCommand::Setup => {
            print!("{GITCONFIG}");
            Ok(Outcome::Same)
        }
        GitCommand::Difftool {
            local,
            remote,
            merged,
            run,
        } => compare(difftool_job(local, remote, merged.as_deref()), &run),
        GitCommand::External { args, run } => {
            compare(external_job(&args)?, &run)?;
            // Git aborts the whole diff ("external diff died") on any non-zero
            // status, so differences are not reported through the exit code.
            Ok(Outcome::Same)
        }
    }
}

fn compare(job: Job, run: &RunArgs) -> Result<Outcome> {
    app::compare(job, &run.output, &run.config, &run.compare)
}

/// `git difftool` passes `$LOCAL $REMOTE $MERGED`. `$MERGED` is the repository
/// path (the old one for renames) and is empty for `git difftool -d`, where
/// `$LOCAL` and `$REMOTE` are temp directories.
pub fn difftool_job(local: PathBuf, remote: PathBuf, merged: Option<&str>) -> Job {
    let merged = merged.filter(|m| !m.is_empty());
    let labels = match merged {
        Some(m) => {
            let side = |p: &Path, prefix: &str| {
                if is_dev_null(p) {
                    DEV_NULL.to_owned()
                } else if prefix == "b" && p.is_relative() {
                    // Comparing with the work tree: $REMOTE is the real (possibly renamed) path.
                    format!("b/{}", p.display())
                } else {
                    format!("{prefix}/{m}")
                }
            };
            [Some(side(&local, "a")), Some(side(&remote, "b"))]
        }
        None if local.is_dir() && remote.is_dir() => [Some("a/".into()), Some("b/".into())],
        None => [None, None],
    };
    Job {
        left: local,
        right: remote,
        labels,
        preamble: None,
        caller: Caller::GitDifftool,
    }
}

/// `GIT_EXTERNAL_DIFF` gets `path old-file old-hex old-mode new-file new-hex
/// new-mode`, plus `new-path message` for renames and copies.
pub fn external_job(args: &[String]) -> Result<Job> {
    let (path, old, new, new_path, message) = match args {
        [path, old, _, _, new, _, _] => (path, old, new, path, None),
        [path, old, _, _, new, _, _, new_path, message] => {
            (path, old, new, new_path, Some(message))
        }
        _ => bail!(
            "expected the 7 or 9 arguments git passes to an external diff, got {}",
            args.len()
        ),
    };
    let label = |file: &str, prefix: &str, p: &str| {
        if file == DEV_NULL {
            DEV_NULL.to_owned()
        } else {
            format!("{prefix}/{p}")
        }
    };
    let mut preamble = format!("diff --git a/{path} b/{new_path}\n");
    if let Some(m) = message {
        preamble.push_str(m);
    }
    Ok(Job {
        left: PathBuf::from(old),
        right: PathBuf::from(new),
        labels: [Some(label(old, "a", path)), Some(label(new, "b", new_path))],
        preamble: Some(preamble),
        caller: Caller::GitExternal,
    })
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[rstest]
    #[case::modified(
        &["f.txt", "/tmp/git-blob-1/f.txt", "7898192", "100644", "f.txt", "0000000", "100644"],
        "a/f.txt", "b/f.txt", "diff --git a/f.txt b/f.txt\n"
    )]
    #[case::added(
        &["n.txt", "/dev/null", ".", ".", "n.txt", "3e75765", "100644"],
        "/dev/null", "b/n.txt", "diff --git a/n.txt b/n.txt\n"
    )]
    #[case::deleted(
        &["old.json", "/tmp/git-blob-2/old.json", "9e26dfe", "100644", "/dev/null", ".", "."],
        "a/old.json", "/dev/null", "diff --git a/old.json b/old.json\n"
    )]
    #[case::renamed(
        &["sub/s.txt", "/tmp/git-blob-3/s.txt", "587be6b", "100644", "sub/u.txt", "0000000", "100644",
          "sub/u.txt", "similarity index 50%\nrename from sub/s.txt\nrename to sub/u.txt\n"],
        "a/sub/s.txt", "b/sub/u.txt",
        "diff --git a/sub/s.txt b/sub/u.txt\nsimilarity index 50%\nrename from sub/s.txt\nrename to sub/u.txt\n"
    )]
    fn external_labels_and_preamble(
        #[case] args: &[&str],
        #[case] left: &str,
        #[case] right: &str,
        #[case] preamble: &str,
    ) {
        let job = external_job(&strings(args)).unwrap();
        assert_eq!(job.labels, [Some(left.to_owned()), Some(right.to_owned())]);
        assert_eq!(job.preamble.as_deref(), Some(preamble));
        assert_eq!(job.left, Path::new(args[1]));
        assert_eq!(job.right, Path::new(args[4]));
        assert_eq!(job.caller, Caller::GitExternal);
    }

    #[test]
    fn setup_quotes_git_variables_for_the_shell() {
        // Inside a git config value, `\"` is a literal quote, so the shell
        // sees "$LOCAL" and paths with spaces survive.
        assert!(
            GITCONFIG.contains(r#"cmd = mydiff git difftool \"$LOCAL\" \"$REMOTE\" \"$MERGED\""#)
        );
    }

    #[test]
    fn external_rejects_eight_arguments() {
        let err = external_job(&strings(&["a"; 8])).unwrap_err();
        assert!(err.to_string().contains("got 8"), "{err}");
    }

    #[rstest]
    #[case::against_worktree(
        "/tmp/git-blob-1/f.txt",
        "src/f.txt",
        Some("src/f.txt"),
        "a/src/f.txt",
        "b/src/f.txt"
    )]
    #[case::renamed_in_worktree(
        "/tmp/git-blob-1/s.txt",
        "sub/t.txt",
        Some("sub/s.txt"),
        "a/sub/s.txt",
        "b/sub/t.txt"
    )]
    #[case::between_commits(
        "/tmp/git-blob-1/f.txt",
        "/tmp/git-blob-2/f.txt",
        Some("f.txt"),
        "a/f.txt",
        "b/f.txt"
    )]
    #[case::added("/dev/null", "n.txt", Some("n.txt"), "/dev/null", "b/n.txt")]
    #[case::deleted(
        "/tmp/git-blob-1/old.json",
        "/dev/null",
        Some("old.json"),
        "a/old.json",
        "/dev/null"
    )]
    fn difftool_labels(
        #[case] local: &str,
        #[case] remote: &str,
        #[case] merged: Option<&str>,
        #[case] left: &str,
        #[case] right: &str,
    ) {
        let job = difftool_job(local.into(), remote.into(), merged);
        assert_eq!(job.labels, [Some(left.to_owned()), Some(right.to_owned())]);
        assert_eq!(job.caller, Caller::GitDifftool);
    }

    #[test]
    fn difftool_dir_mode_labels_roots_a_and_b() {
        let (l, r) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let job = difftool_job(l.path().into(), r.path().into(), Some(""));
        assert_eq!(job.labels, [Some("a/".to_owned()), Some("b/".to_owned())]);
    }

    #[test]
    fn difftool_without_merged_keeps_paths() {
        let job = difftool_job("x.txt".into(), "y.txt".into(), None);
        assert_eq!(job.labels, [None, None]);
    }
}
