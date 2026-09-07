//! `oadec` command-line entry point.

mod compare;
mod damf;
mod decode;
mod emdf;
mod info;
mod input;
mod oamd;
mod scan;
mod verify;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::compare::RefFormat;
use crate::decode::{Format, Order};

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
    /// Walk an E-AC-3 stream, find the EMDF containers and report the metadata timing.
    Emdf {
        /// Raw E-AC-3 (.ec3/.eac3/.ac3) elementary stream.
        file: PathBuf,
        /// Machine-readable JSON instead of text.
        #[arg(long)]
        json: bool,
        /// Print the first N metadata-carrying frames.
        #[arg(long)]
        dump: Option<usize>,
    },
    /// Parse every Object Audio Metadata payload and report what it carries.
    Oamd {
        /// Raw TrueHD (.thd/.mlp) elementary stream.
        file: PathBuf,
        /// Machine-readable JSON instead of text.
        #[arg(long)]
        json: bool,
        /// Print the first N payloads in full.
        #[arg(long)]
        dump: Option<usize>,
    },
    /// Decode one presentation to 24-bit PCM.
    Decode {
        /// Raw TrueHD (.thd/.mlp) elementary stream.
        file: PathBuf,
        /// Output file.
        #[arg(short, long)]
        output: PathBuf,
        /// Presentation to decode (0 = 2ch, 1 = 6ch, 2 = 8ch, 3 = 16ch objects).
        #[arg(short, long, default_value_t = 2)]
        presentation: usize,
        /// Output container.
        #[arg(long, value_enum, default_value_t = Format::Wav)]
        format: Format,
        /// Channel order of the output.
        #[arg(long, value_enum, default_value_t = Order::Interchange)]
        order: Order,
        /// Keep access units flagged as duplicates at seamless branches.
        #[arg(long)]
        keep_duplicates: bool,
        /// DAMF: write the coded bed channels only instead of the full 7.1.2 bed.
        #[arg(long)]
        no_bed_conform: bool,
        /// DAMF: write every metadata update as an event, even unchanged ones.
        #[arg(long)]
        all_events: bool,
    },
    /// Decode one presentation and compare it sample by sample with a reference PCM file.
    Compare {
        /// Raw TrueHD (.thd/.mlp) elementary stream.
        file: PathBuf,
        /// Reference PCM file (headerless, interleaved).
        #[arg(short, long)]
        reference: PathBuf,
        /// Presentation to decode.
        #[arg(short, long, default_value_t = 2)]
        presentation: usize,
        /// Sample format of the reference.
        #[arg(long, value_enum, default_value_t = RefFormat::S32le)]
        reference_format: RefFormat,
        /// Channel order of the reference.
        #[arg(long, value_enum, default_value_t = Order::Interchange)]
        order: Order,
        /// Keep access units flagged as duplicates at seamless branches.
        #[arg(long)]
        keep_duplicates: bool,
        /// How many mismatches to list.
        #[arg(long, default_value_t = 10)]
        report: usize,
        /// Bytes to skip at the start of the reference (a container header).
        #[arg(long, default_value_t = 0)]
        reference_skip: u64,
    },
}

/// Exit code when a verification finds non-conformance.
const EXIT_NONCONFORMANT: u8 = 7;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result =
        match cli.command {
            Command::Info { file, json } => info::run(&file, json).map(|()| ExitCode::SUCCESS),
            Command::Verify { file, json } => verify::run(&file, json).map(|clean| {
                if clean {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::from(EXIT_NONCONFORMANT)
                }
            }),
            Command::Emdf { file, json, dump } => emdf::run(&file, &emdf::Options { json, dump })
                .map(|clean| {
                    if clean {
                        ExitCode::SUCCESS
                    } else {
                        ExitCode::from(EXIT_NONCONFORMANT)
                    }
                }),
            Command::Oamd { file, json, dump } => oamd::run(&file, &oamd::Options { json, dump })
                .map(|clean| {
                    if clean {
                        ExitCode::SUCCESS
                    } else {
                        ExitCode::from(EXIT_NONCONFORMANT)
                    }
                }),
            Command::Decode {
                file,
                output,
                presentation,
                format,
                order,
                keep_duplicates,
                no_bed_conform,
                all_events,
            } => if format == Format::Damf {
                damf::run(
                    &file,
                    &output,
                    &damf::Options {
                        keep_duplicates,
                        bed_conform: !no_bed_conform,
                        all_events,
                    },
                )
            } else {
                decode::run(
                    &file,
                    &output,
                    &decode::Options {
                        presentation,
                        format,
                        order,
                        keep_duplicates,
                    },
                )
            }
            .map(|()| ExitCode::SUCCESS),
            Command::Compare {
                file,
                reference,
                presentation,
                reference_format,
                order,
                keep_duplicates,
                report,
                reference_skip,
            } => compare::run(
                &file,
                &reference,
                &compare::Options {
                    presentation,
                    format: reference_format,
                    order,
                    keep_duplicates,
                    report,
                    skip: reference_skip,
                },
            )
            .map(|equal| {
                if equal {
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
