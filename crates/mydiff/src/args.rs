//! Command-line interface definition.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use mydiff_core::options::{Algorithm, WhitespaceMode};
use toml::{Table, Value};

/// Compare files, folders, binary and structured data.
///
/// With no --format and a terminal on stdout, opens the interactive viewer.
/// Exit status: 0 = no differences, 1 = differences, 2 = error.
#[derive(Debug, Parser)]
#[command(name = "mydiff", version, args_conflicts_with_subcommands = true)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Left input (file, directory, or `-` for stdin).
    pub left: Option<PathBuf>,
    /// Right input (file, directory, or `-` for stdin).
    pub right: Option<PathBuf>,

    /// Describe the AI output format (for an agent's instructions) and exit.
    #[arg(long, help_heading = "AI output")]
    pub ai_help: bool,

    #[command(flatten)]
    pub output: OutputArgs,

    #[command(flatten)]
    pub config: ConfigArgs,

    #[command(flatten)]
    pub compare: CompareArgs,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Work with configuration files.
    #[command(subcommand)]
    Config(ConfigCommand),
    /// Use mydiff from git (`git difftool`, `git diff`).
    #[command(subcommand)]
    Git(GitCommand),
}

#[derive(Debug, Subcommand)]
pub enum GitCommand {
    /// Print git config that makes mydiff git's difftool; append it to ~/.gitconfig.
    Setup,
    /// Entry point for `git difftool` (called as: difftool "$LOCAL" "$REMOTE" "$MERGED").
    Difftool {
        /// Old version ($LOCAL): a temp file, a directory with -d, or /dev/null.
        local: PathBuf,
        /// New version ($REMOTE).
        remote: PathBuf,
        /// Path in the repository ($MERGED); empty for `git difftool -d`.
        merged: Option<String>,
        #[command(flatten)]
        run: RunArgs,
    },
    /// Every file a commit changed, in the folder view (vs its first parent).
    ///
    /// Files are listed by their full repository path; renames are one entry.
    /// Piped, it prints a unified diff of each file.
    Show {
        /// The commit [default: HEAD].
        #[arg(value_name = "COMMIT")]
        commit: Option<String>,
        /// Only these paths (after `--`, as in git).
        #[arg(last = true, value_name = "PATH")]
        paths: Vec<String>,
        #[command(flatten)]
        run: RunArgs,
    },
    /// Every changed file of a `git diff`, in the folder view.
    ///
    /// Takes git diff's revisions: none (work tree vs index), `--staged`
    /// (index vs HEAD), `<rev>` (work tree vs rev), `<a> <b>`, `<a>..<b>`
    /// or `<a>...<b>` (b vs the merge base).
    Diff {
        /// Compare the index (what is staged) instead of the work tree.
        #[arg(long, visible_alias = "cached")]
        staged: bool,
        /// Zero, one or two revisions, as for `git diff`.
        #[arg(value_name = "REV", num_args = 0..=2)]
        revs: Vec<String>,
        /// Only these paths (after `--`, as in git).
        #[arg(last = true, value_name = "PATH")]
        paths: Vec<String>,
        #[command(flatten)]
        run: RunArgs,
    },
    /// Entry point for git's external diff (`GIT_EXTERNAL_DIFF` or `diff.external`).
    ///
    /// Prints a unified diff and exits 0 when files differ, because git stops
    /// at any other status. Put mydiff options before the git arguments.
    External {
        /// The arguments git appends: path old-file old-hex old-mode new-file
        /// new-hex new-mode [new-path rename-message].
        #[arg(
            required = true,
            num_args = 7..=9,
            trailing_var_arg = true,
            allow_hyphen_values = true,
            value_name = "GIT_ARGS"
        )]
        args: Vec<String>,
        #[command(flatten)]
        run: RunArgs,
    },
}

/// Output, configuration and comparison options shared by the git entry points.
#[derive(Debug, Args)]
pub struct RunArgs {
    #[command(flatten)]
    pub output: OutputArgs,
    #[command(flatten)]
    pub config: ConfigArgs,
    #[command(flatten)]
    pub compare: CompareArgs,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Write a commented starter config (default: ./.mydiff.toml).
    Init {
        /// Where to write it.
        #[arg(default_value = ".mydiff.toml")]
        path: PathBuf,
        /// Overwrite an existing file.
        #[arg(long)]
        force: bool,
    },
    /// Print the effective options after merging config files, profile and flags.
    Show {
        #[command(flatten)]
        config: ConfigArgs,
        #[command(flatten)]
        compare: CompareArgs,
    },
    /// List the config files that would be loaded, in order.
    Paths {
        #[command(flatten)]
        config: ConfigArgs,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Mode {
    /// Pick from the inputs: directories, binary content, or file extension.
    Auto,
    Text,
    Dir,
    Binary,
    Json,
    Yaml,
    Csv,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// Interactive terminal viewer.
    Tui,
    /// Unified diff (`diff -u` style).
    Unified,
    /// Two columns.
    SideBySide,
    /// Machine-readable JSON.
    Json,
    /// Compact annotated text for AI agents and LLM prompts (see --ai-help).
    Ai,
    /// One-paragraph summary.
    Summary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ColorWhen {
    Auto,
    Always,
    Never,
}

#[derive(Debug, Args)]
#[command(next_help_heading = "Output")]
pub struct OutputArgs {
    /// Comparison mode.
    #[arg(long, value_enum, default_value_t = Mode::Auto)]
    pub mode: Mode,

    /// Output format [default: tui on a terminal, otherwise unified].
    #[arg(short, long, value_enum)]
    pub format: Option<Format>,

    /// When to use colors (also honors NO_COLOR / CLICOLOR_FORCE).
    #[allow(clippy::doc_markdown, reason = "shown verbatim in --help")]
    #[arg(long, value_enum, default_value_t = ColorWhen::Auto)]
    pub color: ColorWhen,

    /// Name to show for the left input, then the right (give it twice, like `diff -L`).
    #[arg(short = 'L', long, value_name = "LABEL")]
    pub label: Vec<String>,

    /// Total width for side-by-side output [default: $COLUMNS or 120].
    #[arg(long)]
    pub width: Option<usize>,

    /// Print nothing; only set the exit status.
    #[arg(short, long)]
    pub quiet: bool,

    /// AI-friendly output: compact annotated text, never interactive (also
    /// MYDIFF_AI=1; format described by --ai-help).
    #[allow(clippy::doc_markdown, reason = "shown verbatim in --help")]
    #[arg(long, help_heading = "AI output")]
    pub ai: bool,

    /// Stop AI output after N lines [default: $MYDIFF_AI_BUDGET, else 2000; 0 = no limit].
    #[allow(clippy::doc_markdown, reason = "shown verbatim in --help")]
    #[arg(long, value_name = "N", help_heading = "AI output")]
    pub budget: Option<usize>,

    /// Show only this hunk of AI output, in full, e.g. `src/lib.rs#2` (repeatable).
    #[arg(long, value_name = "ID", help_heading = "AI output")]
    pub hunk: Vec<String>,
}

#[derive(Debug, Clone, Args)]
#[command(next_help_heading = "Configuration")]
pub struct ConfigArgs {
    /// Extra config file, applied after discovered ones.
    #[arg(long, value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Skip discovered config files (user and ./.mydiff.toml).
    #[arg(long)]
    pub no_config: bool,

    /// Apply the named [profile.NAME] from config files.
    #[arg(short = 'P', long, value_name = "NAME")]
    pub profile: Option<String>,
}

/// Comparison knobs. Each flag maps to a [`mydiff_core::DiffOptions`] field and
/// only overrides config when given.
#[derive(Debug, Clone, Default, Args)]
#[command(next_help_heading = "Comparison")]
#[allow(clippy::struct_excessive_bools, reason = "one bool per CLI flag")]
pub struct CompareArgs {
    /// Ignore all whitespace (whitespace = "all").
    #[arg(short = 'w', long, group = "ws")]
    pub ignore_all_space: bool,
    /// Ignore changes in the amount of whitespace (whitespace = "change").
    #[arg(short = 'b', long, group = "ws")]
    pub ignore_space_change: bool,
    /// Ignore trailing whitespace (whitespace = "trailing").
    #[arg(short = 'Z', long, group = "ws")]
    pub ignore_trailing_space: bool,
    /// Whitespace handling.
    #[arg(long, value_enum, group = "ws", value_name = "MODE")]
    pub whitespace: Option<WhitespaceArg>,

    /// Ignore case differences.
    #[arg(short = 'i', long)]
    pub ignore_case: bool,
    /// Do not compare blank lines.
    #[arg(short = 'B', long)]
    pub ignore_blank_lines: bool,
    /// Treat CRLF and LF as different.
    #[arg(long)]
    pub strict_line_endings: bool,

    /// Do not compare lines matching REGEX (repeatable).
    #[arg(long, value_name = "REGEX")]
    pub ignore: Vec<String>,
    /// Compare ONLY lines matching REGEX (repeatable whitelist).
    #[arg(long, value_name = "REGEX")]
    pub only: Vec<String>,
    /// Replace matches before comparing, as 'REGEX=>REPLACEMENT' (repeatable).
    #[arg(long, value_name = "REGEX=>REPL", value_parser = parse_mask)]
    pub mask: Vec<(String, String)>,

    /// Lines of context in unified output.
    #[arg(short = 'U', long, value_name = "N")]
    pub context: Option<usize>,
    /// Similarity (0..1) needed to show a changed line as modified.
    #[arg(long, value_name = "RATIO")]
    pub modify_threshold: Option<f32>,
    /// Line diff algorithm.
    #[arg(long, value_enum)]
    pub algorithm: Option<AlgorithmArg>,

    /// Folder mode: only compare paths matching GLOB (repeatable).
    #[arg(long, value_name = "GLOB")]
    pub include: Vec<String>,
    /// Folder mode: skip paths matching GLOB (repeatable).
    #[arg(long, value_name = "GLOB")]
    pub exclude: Vec<String>,

    /// CSV: column(s) identifying a row (repeatable).
    #[arg(long, value_name = "NAME")]
    pub key_column: Vec<String>,
    /// JSON/YAML: skip this path, e.g. '$.meta.time' or '$.items[*].id' (repeatable).
    #[arg(long, value_name = "PATH")]
    pub ignore_path: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum WhitespaceArg {
    Exact,
    Trailing,
    Change,
    All,
}

impl From<WhitespaceArg> for WhitespaceMode {
    fn from(w: WhitespaceArg) -> Self {
        match w {
            WhitespaceArg::Exact => Self::Exact,
            WhitespaceArg::Trailing => Self::Trailing,
            WhitespaceArg::Change => Self::Change,
            WhitespaceArg::All => Self::All,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AlgorithmArg {
    Myers,
    Patience,
}

impl From<AlgorithmArg> for Algorithm {
    fn from(a: AlgorithmArg) -> Self {
        match a {
            AlgorithmArg::Myers => Self::Myers,
            AlgorithmArg::Patience => Self::Patience,
        }
    }
}

fn parse_mask(s: &str) -> Result<(String, String), String> {
    s.split_once("=>")
        .map(|(p, r)| (p.to_owned(), r.to_owned()))
        .ok_or_else(|| format!("expected 'REGEX=>REPLACEMENT', got '{s}'"))
}

impl CompareArgs {
    /// The flags that were actually given, as a config layer.
    pub fn to_layer(&self) -> Table {
        fn strings(v: &[String]) -> Value {
            Value::Array(v.iter().cloned().map(Value::String).collect())
        }
        let mut t = Table::new();
        let ws = if self.ignore_all_space {
            Some(WhitespaceMode::All)
        } else if self.ignore_space_change {
            Some(WhitespaceMode::Change)
        } else if self.ignore_trailing_space {
            Some(WhitespaceMode::Trailing)
        } else {
            self.whitespace.map(Into::into)
        };
        if let Some(ws) = ws {
            t.insert("whitespace".into(), ws.label().into());
        }
        if self.ignore_case {
            t.insert("ignore_case".into(), true.into());
        }
        if self.ignore_blank_lines {
            t.insert("ignore_blank_lines".into(), true.into());
        }
        if self.strict_line_endings {
            t.insert("ignore_line_endings".into(), false.into());
        }
        for (key, list) in [
            ("ignore", &self.ignore),
            ("only", &self.only),
            ("include", &self.include),
            ("exclude", &self.exclude),
            ("key_columns", &self.key_column),
            ("ignore_paths", &self.ignore_path),
        ] {
            if !list.is_empty() {
                t.insert(key.into(), strings(list));
            }
        }
        if !self.mask.is_empty() {
            let rules = self
                .mask
                .iter()
                .map(|(p, r)| {
                    let mut rule = Table::new();
                    rule.insert("pattern".into(), p.clone().into());
                    rule.insert("replace".into(), r.clone().into());
                    Value::Table(rule)
                })
                .collect();
            t.insert("mask".into(), Value::Array(rules));
        }
        if let Some(c) = self.context {
            t.insert(
                "context".into(),
                Value::Integer(i64::try_from(c).unwrap_or(i64::MAX)),
            );
        }
        if let Some(m) = self.modify_threshold {
            t.insert("modify_threshold".into(), Value::Float(f64::from(m)));
        }
        if let Some(a) = self.algorithm {
            let name = match a {
                AlgorithmArg::Myers => "myers",
                AlgorithmArg::Patience => "patience",
            };
            t.insert("algorithm".into(), name.into());
        }
        t
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("mydiff").chain(args.iter().copied())).unwrap()
    }

    #[test]
    fn clap_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn git_external_takes_options_then_git_arguments_verbatim() {
        let cli = parse(&[
            "git",
            "external",
            "-w",
            "--color",
            "never",
            "-x.txt",
            "/tmp/blob/-x.txt",
            ".",
            ".",
            "-x.txt",
            "abc",
            "100644",
        ]);
        let Some(Command::Git(GitCommand::External { args, run })) = cli.command else {
            panic!("parsed as git external")
        };
        assert_eq!(args.len(), 7, "{args:?}");
        assert_eq!(args[0], "-x.txt", "hyphenated paths stay positional");
        assert!(run.compare.ignore_all_space);
        assert_eq!(run.output.color, ColorWhen::Never);
    }

    #[test]
    fn git_external_rejects_too_few_arguments() {
        let r = Cli::try_parse_from(["mydiff", "git", "external", "a", "b"]);
        assert!(r.is_err());
    }

    #[test]
    fn git_difftool_merged_is_optional() {
        let cli = parse(&["git", "difftool", "-L", "x", "/tmp/l", "/tmp/r"]);
        let Some(Command::Git(GitCommand::Difftool { merged, run, .. })) = cli.command else {
            panic!("parsed as git difftool")
        };
        assert_eq!(merged, None);
        assert_eq!(run.output.label, ["x"]);
    }

    #[test]
    fn git_show_takes_a_commit_options_and_paths() {
        let cli = parse(&["git", "show", "-w", "HEAD~2", "--", "src", "-odd"]);
        let Some(Command::Git(GitCommand::Show { commit, paths, run })) = cli.command else {
            panic!("parsed as git show")
        };
        assert_eq!(commit.as_deref(), Some("HEAD~2"));
        assert_eq!(paths, ["src", "-odd"]);
        assert!(run.compare.ignore_all_space);
    }

    #[test]
    fn git_diff_takes_up_to_two_revisions() {
        let cli = parse(&["git", "diff", "--cached", "main"]);
        let Some(Command::Git(GitCommand::Diff { staged, revs, .. })) = cli.command else {
            panic!("parsed as git diff")
        };
        assert!(staged);
        assert_eq!(revs, ["main"]);
        assert!(Cli::try_parse_from(["mydiff", "git", "diff", "a", "b", "c"]).is_err());
    }

    #[test]
    fn labels_repeat() {
        assert_eq!(
            parse(&["-L", "old", "--label", "new", "a", "b"])
                .output
                .label,
            ["old", "new"]
        );
    }

    #[test]
    fn no_flags_means_empty_layer() {
        assert!(parse(&["a", "b"]).compare.to_layer().is_empty());
    }

    #[test]
    fn flags_become_option_keys() {
        let layer = parse(&[
            "-w",
            "-i",
            "--ignore",
            "^#",
            "--ignore",
            "^//",
            "--mask",
            r"\d+=>N",
            "-U",
            "1",
            "--strict-line-endings",
            "a",
            "b",
        ])
        .compare
        .to_layer();
        let expected: Table = toml::from_str(
            r#"
            whitespace = "all"
            ignore_case = true
            ignore_line_endings = false
            ignore = ["^#", "^//"]
            mask = [{ pattern = '\d+', replace = "N" }]
            context = 1
            "#,
        )
        .unwrap();
        assert_eq!(layer, expected);
    }

    #[test]
    fn whitespace_flags_conflict() {
        assert!(Cli::try_parse_from(["mydiff", "-w", "-b", "a", "b"]).is_err());
    }

    #[test]
    fn bad_mask_is_rejected() {
        let err = Cli::try_parse_from(["mydiff", "--mask", "nope", "a", "b"]).unwrap_err();
        assert!(err.to_string().contains("REGEX=>REPLACEMENT"), "{err}");
    }

    #[test]
    fn layer_round_trips_into_options() {
        let layer = parse(&[
            "-b",
            "--modify-threshold",
            "0.8",
            "--algorithm",
            "patience",
            "a",
            "b",
        ])
        .compare
        .to_layer();
        let opts = mydiff_core::config::resolve(&[], None, &layer).unwrap();
        assert_eq!(opts.whitespace, WhitespaceMode::Change);
        assert!((opts.modify_threshold - 0.8).abs() < 1e-6);
        assert_eq!(opts.algorithm, Algorithm::Patience);
    }
}
