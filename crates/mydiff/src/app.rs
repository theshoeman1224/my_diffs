//! Top-level dispatch: resolve inputs and mode, run the comparison, render.

use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use mydiff_core::config::SAMPLE_CONFIG;
use mydiff_core::detect::{Kind, detect};
use mydiff_core::{DiffOptions, diff_text};
use serde::Serialize;

use crate::args::{
    Cli, ColorWhen, Command, CompareArgs, ConfigArgs, ConfigCommand, Format, Mode, OutputArgs,
};
use mydiff_core::structured::{self as st, Format as Format_};

use crate::output::{
    Theme, folder as folder_out, hex, json, side_by_side, structured as st_out, summary, unified,
};
use crate::settings;

/// What the comparison found; maps to the exit status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Same,
    Different,
}

impl Outcome {
    fn from_identical(identical: bool) -> Self {
        if identical {
            Self::Same
        } else {
            Self::Different
        }
    }
}

/// Who asked for the comparison; git entry points change a few defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Caller {
    /// Run directly from the command line.
    User,
    /// `git difftool`: a viewer, like `User`, but unparsable structured files
    /// fall back to a text diff instead of failing.
    GitDifftool,
    /// Git's external diff driver: unified output by default, colored when
    /// git pipes to its pager, and the same text fallback as `GitDifftool`.
    GitExternal,
}

/// One comparison to run.
#[derive(Debug)]
pub struct Job {
    pub left: PathBuf,
    pub right: PathBuf,
    /// Names to show instead of the paths (`-L` still wins).
    pub labels: [Option<String>; 2],
    /// Header printed before unified output (git's `diff --git` lines).
    pub preamble: Option<String>,
    pub caller: Caller,
}

/// Resolved presentation settings.
struct Ctx {
    left_label: String,
    right_label: String,
    format: Format,
    theme: Theme,
    width: usize,
    quiet: bool,
    /// Fall back to a text diff when a structured file does not parse.
    lenient: bool,
}

pub fn run(cli: Cli) -> Result<Outcome> {
    match cli.command {
        Some(Command::Config(cmd)) => {
            config_command(cmd)?;
            Ok(Outcome::Same)
        }
        Some(Command::Git(cmd)) => crate::git::run(cmd),
        None => {
            let (Some(left), Some(right)) = (cli.left, cli.right) else {
                bail!("two inputs are required: mydiff [OPTIONS] <LEFT> <RIGHT> (see --help)");
            };
            let job = Job {
                left,
                right,
                labels: [None, None],
                preamble: None,
                caller: Caller::User,
            };
            compare(job, &cli.output, &cli.config, &cli.compare)
        }
    }
}

/// `-L` values as `[left, right]`.
fn user_labels(labels: &[String]) -> Result<[Option<String>; 2]> {
    match labels {
        [] => Ok([None, None]),
        [l] => Ok([Some(l.clone()), None]),
        [l, r] => Ok([Some(l.clone()), Some(r.clone())]),
        _ => bail!("--label can be given at most twice (left, then right)"),
    }
}

/// Git sets `GIT_PAGER_IN_USE` when our stdout is its pager; color then, as git does.
fn git_pager_wants_color() -> bool {
    std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty())
        && std::env::var("GIT_PAGER_IN_USE").is_ok_and(|v| v == "true" || v == "1")
}

pub fn compare(
    job: Job,
    output: &OutputArgs,
    config: &ConfigArgs,
    compare: &CompareArgs,
) -> Result<Outcome> {
    let [user_left, user_right] = user_labels(&output.label)?;
    let opts = settings::load_options(config, compare)?;
    let stdout_tty = std::io::stdout().is_terminal();
    let format = output
        .format
        .unwrap_or(if stdout_tty && job.caller != Caller::GitExternal {
            Format::Tui
        } else {
            Format::Unified
        });
    let color = match output.color {
        ColorWhen::Always => true,
        ColorWhen::Never => false,
        ColorWhen::Auto => {
            (job.caller == Caller::GitExternal && git_pager_wants_color())
                || anstream::AutoStream::choice(&std::io::stdout()) != anstream::ColorChoice::Never
        }
    };
    let width = output
        .width
        .or_else(|| std::env::var("COLUMNS").ok()?.parse().ok())
        .or_else(|| {
            stdout_tty
                .then(|| {
                    crossterm::terminal::size()
                        .ok()
                        .map(|(w, _)| usize::from(w))
                })
                .flatten()
        })
        .unwrap_or(120);
    let [job_left, job_right] = job.labels;
    let (left, right) = pair_paths(job.left, job.right)?;
    let ctx = Ctx {
        left_label: user_left.or(job_left).unwrap_or_else(|| label(&left)),
        right_label: user_right.or(job_right).unwrap_or_else(|| label(&right)),
        format,
        theme: Theme { color },
        width,
        quiet: output.quiet,
        lenient: job.caller != Caller::User,
    };
    if let Some(pre) = &job.preamble
        && format == Format::Unified
        && !ctx.quiet
    {
        emit(&ctx.theme.paint(Theme::header(), pre.trim_end()))?;
        emit("\n")?;
    }

    if left.is_dir() && right.is_dir() {
        return folder(&ctx, &left, &right, &opts);
    }
    if output.mode == Mode::Dir {
        bail!("--mode dir needs two directories");
    }
    let lb = read_input(&left)?;
    let rb = read_input(&right)?;
    let kind = match output.mode {
        Mode::Auto => detect(&left, &lb, &right, &rb),
        Mode::Text => Kind::Text,
        Mode::Binary => Kind::Binary,
        Mode::Json => Kind::Json,
        Mode::Yaml => Kind::Yaml,
        Mode::Csv => Kind::Csv(if has_ext(&left, "tsv") { b'\t' } else { b',' }),
        Mode::Dir => unreachable!("handled above"),
    };
    match kind {
        Kind::Text => text(&ctx, &lb, &rb, &opts),
        Kind::Binary => binary(&ctx, lb, rb),
        Kind::Json => structured(&ctx, &lb, &rb, Format_::Json, &opts),
        Kind::Yaml => structured(&ctx, &lb, &rb, Format_::Yaml, &opts),
        Kind::Csv(d) => structured(&ctx, &lb, &rb, Format_::Csv(d), &opts),
    }
}

fn has_ext(p: &Path, ext: &str) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

fn label(p: &Path) -> String {
    if p.as_os_str() == "-" {
        "(stdin)".to_owned()
    } else {
        p.display().to_string()
    }
}

/// Like `diff`: comparing a file with a directory compares it with the file of the
/// same name inside that directory.
fn pair_paths(left: PathBuf, right: PathBuf) -> Result<(PathBuf, PathBuf)> {
    if left.as_os_str() == "-" && right.as_os_str() == "-" {
        bail!("only one input can be stdin (`-`)");
    }
    let named = |file: &Path, dir: &Path| -> Result<PathBuf> {
        let name = file.file_name().with_context(|| {
            format!(
                "{} has no file name to look up in {}",
                file.display(),
                dir.display()
            )
        })?;
        Ok(dir.join(name))
    };
    Ok(match (left.is_dir(), right.is_dir()) {
        (false, true) if left.as_os_str() != "-" => {
            let r = named(&left, &right)?;
            (left, r)
        }
        (true, false) if right.as_os_str() != "-" => {
            let l = named(&right, &left)?;
            (l, right)
        }
        _ => (left, right),
    })
}

fn read_input(path: &Path) -> Result<Vec<u8>> {
    if path.as_os_str() == "-" {
        let mut buf = Vec::new();
        std::io::stdin()
            .read_to_end(&mut buf)
            .context("reading stdin")?;
        return Ok(buf);
    }
    std::fs::read(path).with_context(|| format!("reading {}", path.display()))
}

/// Write to stdout, treating a closed pipe (`| head`) as success.
fn emit(s: &str) -> Result<()> {
    let mut out = std::io::stdout().lock();
    match out.write_all(s.as_bytes()).and_then(|()| out.flush()) {
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        r => Ok(r?),
    }
}

#[derive(Serialize)]
struct TextBody<'a> {
    stats: mydiff_core::Stats,
    rows: &'a [mydiff_core::Row],
}

fn text(ctx: &Ctx, lb: &[u8], rb: &[u8], opts: &DiffOptions) -> Result<Outcome> {
    let lt = String::from_utf8_lossy(lb);
    let rt = String::from_utf8_lossy(rb);
    let diff = diff_text(&lt, &rt, opts)?;
    let outcome = Outcome::from_identical(diff.is_identical());
    if ctx.quiet {
        return Ok(outcome);
    }
    let (l, r) = (ctx.left_label.as_str(), ctx.right_label.as_str());
    match ctx.format {
        Format::Tui => crate::state::with_sidebar(|sidebar| {
            mydiff_tui::run_text(
                mydiff_tui::TextSession {
                    left_label: l.to_owned(),
                    right_label: r.to_owned(),
                    left: lt.into_owned(),
                    right: rt.into_owned(),
                    options: opts.clone(),
                },
                sidebar,
            )
        })?,
        Format::Unified => emit(&unified::render(&diff, l, r, opts.context, ctx.theme))?,
        Format::SideBySide => emit(&side_by_side::render(&diff, l, r, ctx.width, ctx.theme))?,
        Format::Summary => emit(&summary::text(l, r, &diff.stats()))?,
        Format::Json => emit(&json::render(&json::Report {
            mode: "text",
            left: l,
            right: r,
            identical: diff.is_identical(),
            body: TextBody {
                stats: diff.stats(),
                rows: &diff.rows,
            },
        }))?,
    }
    Ok(outcome)
}

#[derive(Serialize)]
struct StructBody<'a> {
    stats: st::StructStats,
    changes: &'a [st::StructChange],
}

fn structured(
    ctx: &Ctx,
    lb: &[u8],
    rb: &[u8],
    format: Format_,
    opts: &DiffOptions,
) -> Result<Outcome> {
    let (lt, rt) = (String::from_utf8_lossy(lb), String::from_utf8_lossy(rb));
    let parsed = st::parse(&lt, format, "left", opts)
        .and_then(|lv| Ok((lv, st::parse(&rt, format, "right", opts)?)));
    let (lv, rv) = match parsed {
        Ok(v) => v,
        Err(e) if ctx.lenient => {
            eprintln!("mydiff: note: {e}; showing a text diff");
            return text(ctx, lb, rb, opts);
        }
        Err(e) => return Err(e.into()),
    };
    let diff = st::diff_values(&lv, &rv, opts)?;
    let outcome = Outcome::from_identical(diff.is_identical());
    if ctx.quiet {
        return Ok(outcome);
    }
    let name = match format {
        Format_::Json => "json",
        Format_::Yaml => "yaml",
        Format_::Csv(_) => "csv",
    };
    let (l, r) = (ctx.left_label.as_str(), ctx.right_label.as_str());
    // Views that show whole documents use a canonical pretty-print of both sides.
    let canonical = || -> Result<(String, String)> {
        Ok((
            st::canonical_text(&lv, opts)?,
            st::canonical_text(&rv, opts)?,
        ))
    };
    match ctx.format {
        Format::Tui => {
            let (left, right) = canonical()?;
            crate::state::with_sidebar(|sidebar| {
                mydiff_tui::run_text(
                    mydiff_tui::TextSession {
                        left_label: format!("{l} ({name}, canonical)"),
                        right_label: format!("{r} ({name}, canonical)"),
                        left,
                        right,
                        options: opts.clone(),
                    },
                    sidebar,
                )
            })?;
        }
        Format::SideBySide => {
            let (left, right) = canonical()?;
            let d = diff_text(&left, &right, opts)?;
            emit(&side_by_side::render(&d, l, r, ctx.width, ctx.theme))?;
        }
        Format::Unified => emit(&st_out::changes(&diff, l, r, ctx.theme))?,
        Format::Summary => emit(&st_out::summary(&diff, name, l, r))?,
        Format::Json => emit(&json::render(&json::Report {
            mode: name,
            left: l,
            right: r,
            identical: diff.is_identical(),
            body: StructBody {
                stats: diff.stats(),
                changes: &diff.changes,
            },
        }))?,
    }
    Ok(outcome)
}

#[derive(Serialize)]
struct HexRowJson {
    offset: usize,
    left: String,
    right: String,
}

#[derive(Serialize)]
struct BinaryBody {
    stats: mydiff_core::binary::HexStats,
    changed_rows: Vec<HexRowJson>,
}

fn binary(ctx: &Ctx, lb: Vec<u8>, rb: Vec<u8>) -> Result<Outcome> {
    let diff = mydiff_core::binary::diff_bytes(lb, rb);
    let outcome = Outcome::from_identical(diff.is_identical());
    if ctx.quiet {
        return Ok(outcome);
    }
    let (l, r) = (ctx.left_label.as_str(), ctx.right_label.as_str());
    match ctx.format {
        Format::Tui => crate::state::with_sidebar(|sidebar| {
            mydiff_tui::run_hex(
                mydiff_tui::HexSession {
                    left_label: l.to_owned(),
                    right_label: r.to_owned(),
                    diff,
                },
                sidebar,
            )
        })?,
        Format::Unified => emit(&hex::unified(&diff, l, r, ctx.theme))?,
        Format::SideBySide => emit(&hex::side_by_side(&diff, ctx.theme))?,
        Format::Summary => emit(&hex::summary(&diff, l, r))?,
        Format::Json => {
            let to_hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
            let changed_rows = diff
                .changed_rows
                .iter()
                .map(|&i| {
                    let row = diff.row(i);
                    HexRowJson {
                        offset: row.offset,
                        left: to_hex(row.left),
                        right: to_hex(row.right),
                    }
                })
                .collect();
            emit(&json::render(&json::Report {
                mode: "binary",
                left: l,
                right: r,
                identical: diff.is_identical(),
                body: BinaryBody {
                    stats: diff.stats(),
                    changed_rows,
                },
            }))?;
        }
    }
    Ok(outcome)
}

#[derive(Serialize)]
struct FolderBody<'a> {
    stats: mydiff_core::folder::FolderStats,
    entries: &'a [mydiff_core::folder::FolderEntry],
}

fn folder(ctx: &Ctx, left: &Path, right: &Path, opts: &DiffOptions) -> Result<Outcome> {
    if ctx.format == Format::Tui && !ctx.quiet {
        crate::state::with_sidebar(|sidebar| {
            mydiff_tui::run_folder(
                mydiff_tui::FolderSession {
                    left_root: left.to_path_buf(),
                    right_root: right.to_path_buf(),
                    left_label: ctx.left_label.clone(),
                    right_label: ctx.right_label.clone(),
                    options: opts.clone(),
                },
                sidebar,
            )
        })?;
        // Exit status reflects the comparison with the options as given.
    }
    let diff = mydiff_core::folder::diff_folders(left, right, opts)?;
    let outcome = Outcome::from_identical(diff.is_identical());
    if ctx.quiet || ctx.format == Format::Tui {
        return Ok(outcome);
    }
    let (l, r) = (ctx.left_label.as_str(), ctx.right_label.as_str());
    match ctx.format {
        Format::Tui => unreachable!("handled above"),
        Format::Unified => emit(&folder_out::unified_all(
            &diff,
            [left, right],
            [l, r],
            opts,
            ctx.theme,
            |p| std::fs::read(p),
        ))?,
        Format::SideBySide => emit(&folder_out::listing(&diff, true, ctx.theme))?,
        Format::Summary => {
            let mut s = folder_out::listing(&diff, false, ctx.theme);
            s.push_str(&folder_out::summary_line(l, r, &diff.stats()));
            emit(&s)?;
        }
        Format::Json => emit(&json::render(&json::Report {
            mode: "folder",
            left: l,
            right: r,
            identical: diff.is_identical(),
            body: FolderBody {
                stats: diff.stats(),
                entries: &diff.entries,
            },
        }))?,
    }
    Ok(outcome)
}

fn config_command(cmd: ConfigCommand) -> Result<()> {
    match cmd {
        ConfigCommand::Init { path, force } => {
            if path.exists() && !force {
                bail!(
                    "{} already exists (use --force to overwrite)",
                    path.display()
                );
            }
            std::fs::write(&path, SAMPLE_CONFIG)
                .with_context(|| format!("writing {}", path.display()))?;
            emit(&format!("wrote {}\n", path.display()))
        }
        ConfigCommand::Show { config, compare } => {
            let opts = settings::load_options(&config, &compare)?;
            emit(&toml::to_string(&opts).context("serializing options")?)
        }
        ConfigCommand::Paths { config } => {
            let mut out = String::new();
            let loaded = settings::config_paths(&config);
            let mut candidates: Vec<PathBuf> = settings::user_config_path().into_iter().collect();
            candidates.push(PathBuf::from(settings::LOCAL_CONFIG));
            candidates.extend(config.config.clone());
            for p in candidates {
                let status = if loaded.contains(&p) {
                    "loaded"
                } else if config.no_config {
                    "skipped"
                } else {
                    "not found"
                };
                out.push_str(&format!("{status:>9}  {}\n", p.display()));
            }
            emit(&out)
        }
    }
}
