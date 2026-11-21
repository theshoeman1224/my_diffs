//! End-to-end CLI tests. Each `tests/cmd/<name>.trycmd` file is a Markdown
//! transcript of commands and their expected output, run with
//! `tests/cmd/<name>.in/` as the working directory.
//!
//! Regenerate expected output after an intended change with
//! `TRYCMD=overwrite cargo test -p mydiff --test cli`, then review the diff.

#[test]
fn cli() {
    trycmd::TestCases::new()
        .default_bin_name("mydiff")
        // Never pick up the developer's real config.
        .env("XDG_CONFIG_HOME", "/nonexistent/mydiff-tests")
        .env("XDG_STATE_HOME", "/nonexistent/mydiff-tests")
        .env("NO_COLOR", "1")
        // An agent's MYDIFF_AI=1 must not change the transcripts (empty = off).
        .env("MYDIFF_AI", "")
        .env("MYDIFF_AI_BUDGET", "")
        .env("COLUMNS", "80")
        .case("tests/cmd/*.trycmd");
}

/// Git cannot store an empty directory, so this case builds its own inputs.
#[test]
fn ai_folder_output_still_reports_an_empty_directory() {
    let (l, r) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::write(l.path().join("same.txt"), "x\n").unwrap();
    std::fs::write(r.path().join("same.txt"), "x\n").unwrap();
    std::fs::create_dir(r.path().join("empty")).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_mydiff"))
        .args(["--ai", "-L", "L", "-L", "R"])
        .arg(l.path())
        .arg(r.path())
        .env("XDG_CONFIG_HOME", "/nonexistent/mydiff-tests")
        .env("XDG_STATE_HOME", "/nonexistent/mydiff-tests")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "mydiff-ai v1\ncompare: L -> R\nsummary: 1 file changed: 1 added; +0 -0 lines\n\
         files:\n  A empty (directory only on the right)\n"
    );
}
