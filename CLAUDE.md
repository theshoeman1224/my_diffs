# mydiff: instructions for AI agents and contributors

`mydiff` is a Beyond Compare–style diff tool written in Rust. It compares text,
folders, binary files and structured data (JSON/YAML/CSV). The simple mode is a
terminal UI (ratatui). The advanced mode is a CLI with a config file. It is
developed on Linux and viewed from Windows over SSH or X11 (xterm), so the UI
uses only the 16 basic ANSI colors.

**This repo is test-first.** Every behavior change comes with a test that fails
without the change. A change is done only when `just ci` passes.

## Layout

```
crates/mydiff-core/   pure engine: text in → result model out (no terminal I/O)
  src/options.rs      DiffOptions: the single list of every knob
  src/normalize.rs    line → comparison key (whitespace, case, masks, CRLF)
  src/filter.rs       ignore / only / mask regexes
  src/text.rs         line diff + align_block (decides "modified")
  src/inline.rs       word-level highlights + similarity()
  src/model.rs        TextDiff/Row/RowKind + render_review() for snapshots
  src/config.rs       TOML config layering (pure)
  src/folder.rs       directory compare (the only fs walking in core)
  src/binary.rs       aligned byte compare + hex rows
  src/structured/     JSON/YAML/CSV → serde_json::Value → path-based changes
  src/display.rs      tab expansion / width helpers shared by front-ends
  tests/fixtures/     golden corpus (see below)
  tests/fixtures.rs   harness: one test per fixture case
  tests/properties.rs proptest invariants
crates/mydiff-tui/    ratatui app, Elm style: keys.rs → app.rs update() → view.rs render()
  src/folder.rs, src/hex.rs   folder list and hex views
crates/mydiff/        binary: args.rs (clap), settings.rs (config discovery), app.rs (dispatch),
                      git.rs (git difftool / external diff adapters),
                      output/ (unified, side-by-side, json, summary, folder, hex, structured)
  tests/cmd/*.trycmd  CLI transcripts, run with tests/cmd/<name>.in/ as the working directory
  tests/git.rs        end-to-end: real git runs mydiff via the `mydiff git setup` config
```

## Commands

| Command | What it does |
|---|---|
| `just ci` | Everything CI runs: fmt-check, clippy `-D warnings`, tests, snapshot check, coverage floor, cargo-deny |
| `just test` | All tests via nextest, plus doctests |
| `just snap` | Run tests, then review new or changed snapshots interactively |
| `just new-case <mode>/<name> [ext]` | Scaffold a fixture case (`text`, `binary`, `structured`, `folder`) |
| `just cov-html` | Coverage report at `target/llvm-cov/html/index.html` |
| `just run <args>` | Run the app |
| `TRYCMD=overwrite cargo test -p mydiff --test cli` | Regenerate CLI transcripts (then review `git diff`) |

First-time setup: `just setup` installs nextest, cargo-insta, cargo-llvm-cov and cargo-deny.

## Where a test goes

Pick the **first** row that fits:

| You are testing… | Put it in | How |
|---|---|---|
| How two inputs compare (any mode) | `crates/mydiff-core/tests/fixtures/<mode>/<case>/` | `just new-case`, fill `left.*`/`right.*`, optional `options.toml`, `just snap` |
| A pure function's edge cases | `#[cfg(test)] mod tests` in that module | `#[rstest]` + `#[case::name(...)]` table |
| Something true for *all* inputs | `crates/mydiff-core/tests/properties.rs` | `proptest!` |
| Behavior under real git (difftool, external diff) | `crates/mydiff/tests/git.rs` | Extend `Repo::new()`; assert or `insta::assert_snapshot!` on git's stdout |
| CLI flags, exit codes, output formats, errors | `crates/mydiff/tests/cmd/<topic>.trycmd` (+ `<topic>.in/`) | Add a `$ mydiff …` block with `? <status>`, then `TRYCMD=overwrite` |
| Renderer output (unified, side-by-side…) | Module tests in `crates/mydiff/src/output/` | `insta::assert_snapshot!(…, @"…")` inline |
| TUI behavior (keys, scrolling, toggles) | `crates/mydiff-tui/src/{keys,app,folder,hex}.rs` tests | Call `update(Action::…)` and assert on state |
| TUI appearance | `crates/mydiff-tui/src/view.rs` tests | `TestBackend` + `insta::assert_snapshot!`; check colors via `buffer()[(x, y)].fg` |

Fixture case layouts:
- `text/<case>/left.txt right.txt [options.toml]`
- `binary/<case>/left.bin right.bin`
- `structured/<case>/left.json right.json [options.toml]` (or `.yaml`, `.csv`, `.tsv`)
- `folder/<case>/left/ right/ options.toml` (`options.toml` is **required** because it marks the case; git does not track empty dirs)

`options.toml` uses the same keys as a config `[defaults]` table, for example
`whitespace = "all"`, `ignore = ['^#']`, `key_columns = ["id"]`.

## Snapshot rules

- **Read every new or changed snapshot before accepting it.** A snapshot is a claim
  about correct behavior; accepting a wrong one bakes in the bug. If the output
  is wrong, fix the code, not the snapshot.
- Never set `INSTA_FORCE_PASS` and never hand-edit `.snap` files to make tests pass.
- Snapshot formats are designed to be read: `TextDiff::render_review()` marks
  changed spans as `[-old-]` / `{+new+}`; folder, binary and structured diffs have
  their own `render_review()`.
- CI sets `CI=true`, so a mismatched or missing snapshot fails instead of writing `.snap.new`.
- `just snap-check` also rejects unreferenced (orphaned) snapshots. Delete snapshot
  files for tests you remove.

## Architecture rules

1. **The core is pure.** It takes strings or bytes in and returns models out.
   Terminal, color and file-discovery code live in `mydiff` and `mydiff-tui`.
   Folder walking (`folder.rs`) is the single exception.
2. **Options change the comparison key, never the displayed text.** Toggling
   whitespace or case must not alter what is shown.
3. **"Modified" (yellow) is defined in one place:** `text::align_block` together with
   `inline::similarity`. Do not add a second heuristic elsewhere.
4. **The TUI is Elm style.** `keys::action_for` (event → `Action`) and
   `App::update` are pure, and so is `view::render(&App, &mut Frame)`. Only
   `mydiff_tui::run*` touches the terminal.
5. **Colors and markers.** Red = removed (`-`), green = added (`+`), yellow =
   modified (`~`), dim = ignored (`.`). Every colored thing also has a marker so
   output reads without color. Use only the basic 16 colors.
6. **Exit codes.** 0 = identical, 1 = different, 2 = error (like `diff`).
   Exception: `mydiff git external` exits 0 on differences, because git aborts on
   any non-zero status from an external diff.
7. Errors: `thiserror` (`mydiff_core::Error`) in libraries, `anyhow` in the binary.
   Validate options early (`DiffOptions::validate`) so bad input fails before files are read.

## Adding a new comparison knob (checklist)

1. Add a field to `DiffOptions` (`crates/mydiff-core/src/options.rs`) with a default and a doc comment.
2. Use it in the core and add rstest cases plus at least one fixture case.
3. Add a CLI flag in `CompareArgs` and map it in `CompareArgs::to_layer` (`crates/mydiff/src/args.rs`), and extend the `flags_become_option_keys` test.
4. Document it in `SAMPLE_CONFIG` (`crates/mydiff-core/src/config.rs`). `sample_config_parses_and_equals_defaults` must keep passing.
5. If it should be toggleable in the TUI, add an `Action`, a key in `keys.rs` plus a `COMMANDS` entry (the sidebar and help are built from it; tests fail if a key is missing), handle it in `App::update`, and give it a live value in `option_value`.
6. Add a `.trycmd` example and update the README.

## Conventions

- Run `cargo fmt`. Clippy pedantic is on, so fix warnings rather than allowing them;
  local `#[allow]`s need a `reason = "…"`.
- Name fixture case folders `snake_case` after the behavior (`whitespace_trailing`, `csv_keyed`).
- Name test functions after the behavior, not the function (`ignored_lines_stay_patch_correct`).
- Keep tests deterministic: no network, no clock, no dependence on `$HOME`
  (CLI tests point `XDG_CONFIG_HOME` at a nonexistent dir; git tests also isolate
  `HOME`, `GIT_CONFIG_GLOBAL` and strip `GIT_*` variables).
- trycmd normalizes `\` to `/` in output, so a transcript cannot show
  backslashes faithfully. Test such output with a Rust test instead.
- Do not commit or push unless asked.
