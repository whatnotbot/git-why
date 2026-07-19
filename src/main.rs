use std::io::{self, Write};
use std::process::ExitCode;

use clap::Parser;
use git_why::{git, output, AppError, Result};

#[derive(Parser)]
#[command(
    name = "git-why",
    version,
    about = "Show the recorded evidence behind a line of code"
)]
struct Cli {
    /// Emit the report as JSON
    #[arg(long)]
    json: bool,

    /// File and line to explain, for example src/auth.rs:42
    #[arg(
        value_name = "FILE:LINE",
        allow_hyphen_values = true,
        value_parser = git::validate_target
    )]
    target: String,
}

fn run(cli: Cli) -> Result<()> {
    let report = git::analyze(&cli.target)?;
    let rendered = if cli.json {
        output::json(&report)?
    } else {
        output::human(&report)
    };

    io::stdout()
        .lock()
        .write_all(rendered.as_bytes())
        .map_err(|error| AppError(format!("could not write output: {error}")))
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("git-why: {}", output::sanitize(&error.to_string()));
            ExitCode::from(1)
        }
    }
}
