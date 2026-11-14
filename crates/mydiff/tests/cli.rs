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
        .env("NO_COLOR", "1")
        .env("COLUMNS", "80")
        .case("tests/cmd/*.trycmd");
}
