//! `mydiff` binary entry point. Exit status: 0 = same, 1 = different, 2 = error.

mod app;
mod args;
mod git;
mod output;
mod settings;

use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    let cli = args::Cli::parse();
    match app::run(cli) {
        Ok(app::Outcome::Same) => ExitCode::from(0),
        Ok(app::Outcome::Different) => ExitCode::from(1),
        Err(e) => {
            eprintln!("mydiff: error: {e:#}");
            ExitCode::from(2)
        }
    }
}
