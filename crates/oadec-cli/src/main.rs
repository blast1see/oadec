//! `oadec` command-line entry point.

use std::process::ExitCode;

fn main() -> ExitCode {
    oadec_cli::run(std::env::args_os())
}
