//! Golden fixture tests. Each case is a folder under `tests/fixtures/<mode>/`:
//!
//! ```text
//! tests/fixtures/text/<case>/
//!   left.<ext>        required
//!   right.<ext>       required (same extension)
//!   options.toml      optional DiffOptions (same keys as a config [defaults] table)
//!   expected.snap     insta snapshot; created by `cargo insta review`
//! ```
//!
//! Every case shows up as its own test (`cargo nextest list` shows them). Scaffold a
//! new one with `just new-case text/<name>`.

use std::path::{Path, PathBuf};

use mydiff_core::{DiffOptions, diff_text};

fn case_dir(left: &Path) -> PathBuf {
    std::fs::canonicalize(left.parent().expect("fixture has a parent dir")).expect("canonical dir")
}

fn sibling(left: &Path, stem: &str) -> PathBuf {
    let ext = left
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    left.with_file_name(format!("{stem}{ext}"))
}

fn load_options(dir: &Path) -> datatest_stable::Result<DiffOptions> {
    let path = dir.join("options.toml");
    if path.exists() {
        Ok(DiffOptions::from_toml(&std::fs::read_to_string(path)?)?)
    } else {
        Ok(DiffOptions::default())
    }
}

fn snapshot(dir: &Path, left: &Path, body: &str) {
    let mut settings = insta::Settings::clone_current();
    settings.set_snapshot_path(dir);
    settings.set_prepend_module_to_snapshot(false);
    settings.set_input_file(left);
    settings.set_omit_expression(true);
    settings.bind(|| insta::assert_snapshot!("expected", body));
}

fn text_case(left_path: &Path) -> datatest_stable::Result<()> {
    let dir = case_dir(left_path);
    let opts = load_options(&dir)?;
    let left = std::fs::read_to_string(left_path)?;
    let right = std::fs::read_to_string(sibling(left_path, "right"))?;
    let diff = diff_text(&left, &right, &opts)?;
    let s = diff.stats();
    let body = format!(
        "{}-- equal={} removed={} added={} modified={} ignored={}\n",
        diff.render_review(),
        s.equal,
        s.removed,
        s.added,
        s.modified,
        s.ignored
    );
    snapshot(&dir, left_path, &body);
    Ok(())
}

/// Folder cases: `tests/fixtures/folder/<case>/{left/,right/,options.toml}`.
/// `options.toml` is required (it marks the case; it may contain only a comment).
fn folder_case(options_path: &Path) -> datatest_stable::Result<()> {
    let dir = case_dir(options_path);
    let opts = load_options(&dir)?;
    let diff = mydiff_core::folder::diff_folders(&dir.join("left"), &dir.join("right"), &opts)?;
    let s = diff.stats();
    let body = format!(
        "{}-- same={} different={} only_left={} only_right={} type_mismatch={} errors={}\n",
        diff.render_review(),
        s.same,
        s.different,
        s.only_left,
        s.only_right,
        s.type_mismatch,
        s.errors
    );
    snapshot(&dir, options_path, &body);
    Ok(())
}

/// Binary cases: `tests/fixtures/binary/<case>/{left.bin,right.bin}` (any extension).
fn binary_case(left_path: &Path) -> datatest_stable::Result<()> {
    let dir = case_dir(left_path);
    let left = std::fs::read(left_path)?;
    let right = std::fs::read(sibling(left_path, "right"))?;
    let diff = mydiff_core::binary::diff_bytes(left, right);
    snapshot(&dir, left_path, &diff.render_review());
    Ok(())
}

/// Structured cases: `tests/fixtures/structured/<case>/{left,right}.{json,yaml,yml,csv,tsv}`.
fn structured_case(left_path: &Path) -> datatest_stable::Result<()> {
    use mydiff_core::detect::{Kind, kind_from_extension};
    use mydiff_core::structured::{Format, diff_structured};
    let dir = case_dir(left_path);
    let opts = load_options(&dir)?;
    let format = match kind_from_extension(left_path) {
        Some(Kind::Json) => Format::Json,
        Some(Kind::Yaml) => Format::Yaml,
        Some(Kind::Csv(d)) => Format::Csv(d),
        other => return Err(format!("not a structured fixture extension: {other:?}").into()),
    };
    let left = std::fs::read_to_string(left_path)?;
    let right = std::fs::read_to_string(sibling(left_path, "right"))?;
    let diff = diff_structured(&left, &right, format, &opts)?;
    let s = diff.stats();
    let body = format!(
        "{}-- added={} removed={} changed={}\n",
        diff.render_review(),
        s.added,
        s.removed,
        s.changed
    );
    snapshot(&dir, left_path, &body);
    Ok(())
}

datatest_stable::harness! {
    { test = text_case, root = "tests/fixtures/text", pattern = r"(^|/)left\.[^/]+$" },
    { test = binary_case, root = "tests/fixtures/binary", pattern = r"(^|/)left\.[^/]+$" },
    { test = structured_case, root = "tests/fixtures/structured", pattern = r"(^|/)left\.[^/]+$" },
    { test = folder_case, root = "tests/fixtures/folder", pattern = r"^[^/]+/options\.toml$" },
}
