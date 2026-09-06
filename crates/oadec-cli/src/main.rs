//! `oadec` command-line entry point.

mod info;
mod input;
mod scan;
mod verify;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

/// Object-audio decoder engine for Dolby TrueHD Atmos and E-AC-3 JOC streams.
#[derive(Debug, Parser)]
#[command(name = "oadec", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print what a stream declares about itself.
    Info {
        /// Raw TrueHD (.thd/.mlp) elementary stream.
        file: PathBuf,
        /// Machine-readable JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Check the integrity of every access unit and report the failure counts.
    Verify {
        /// Raw TrueHD (.thd/.mlp) elementary stream.
        file: PathBuf,
        /// Machine-readable JSON instead of text.
        #[arg(long)]
        json: bool,
    },
}

/// Exit code when a verification finds non-conformance.
const EXIT_NONCONFORMANT: u8 = 7;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Info { file, json } => info::run(&file, json).map(|()| ExitCode::SUCCESS),
        Command::Verify { file, json } => verify::run(&file, json).map(|clean| {
            if clean {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(EXIT_NONCONFORMANT)
            }
        }),
    };
    match result {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(2)
        }
    }
}
