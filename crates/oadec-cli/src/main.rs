//! `oadec` command-line entry point.

mod compare;
mod damf;
mod decode;
mod eac3;
mod eac3_objects;
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
        /// Raw TrueHD (.thd/.mlp) or AC-3 / E-AC-3 (.ac3/.ec3) elementary stream.
        file: PathBuf,
        /// Machine-readable JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Check the integrity of every access unit or frame and report the failure counts.
    Verify {
        /// Raw TrueHD (.thd/.mlp) or AC-3 / E-AC-3 (.ac3/.ec3) elementary stream.
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
    /// Decode a TrueHD presentation (24-bit PCM) or an E-AC-3 stream (32-bit float), or
    /// write the object program as a DAMF set or ADM BWF file.
    Decode {
        /// Raw TrueHD (.thd/.mlp) or AC-3 / E-AC-3 (.ac3/.ec3) elementary stream.
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
        /// ADM: write the creator string that the Dolby validators and encoders
        /// require before they accept an ADM BWF file ("Created using Dolby
        /// equipment"); the DAMF output needs no such marker.
        #[arg(long)]
        dolby_origin_tag: bool,
        /// E-AC-3: substitute zeros instead of dither for zero-bit mantissas.
        #[arg(long)]
        no_dither: bool,
    },
    /// Decode and compare sample by sample with a reference PCM file (TrueHD: integer
    /// formats; E-AC-3: 32-bit float, judged on the SNR because decoders dither).
    Compare {
        /// Raw TrueHD (.thd/.mlp) or AC-3 / E-AC-3 (.ac3/.ec3) elementary stream.
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
        /// E-AC-3: substitute zeros instead of dither for zero-bit mantissas.
        #[arg(long)]
        no_dither: bool,
        /// E-AC-3: list the N blocks with the largest deviation.
        #[arg(long, default_value_t = 0)]
        worst: usize,
    },
    /// Print the side information of one E-AC-3 frame, block by block.
    #[command(hide = true)]
    Eac3Blocks {
        /// Raw E-AC-3 (.ec3/.eac3/.ac3) elementary stream.
        file: PathBuf,
        /// Index of the decoded frame (independent substream 0).
        #[arg(long)]
        frame: u64,
    },
}

/// Exit code when a verification finds non-conformance.
const EXIT_NONCONFORMANT: u8 = 7;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result =
        match cli.command {
            Command::Info { file, json } => eac3::is_eac3(&file)
                .and_then(|is| {
                    if is {
                        eac3::info(&file, json)
                    } else {
                        info::run(&file, json)
                    }
                })
                .map(|()| ExitCode::SUCCESS),
            Command::Verify { file, json } => eac3::is_eac3(&file)
                .and_then(|is| {
                    if is {
                        eac3::verify(&file, json)
                    } else {
                        verify::run(&file, json)
                    }
                })
                .map(|clean| {
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
                dolby_origin_tag,
                no_dither,
            } => if eac3::is_eac3(&file).unwrap_or(false)
                && matches!(format, Format::Damf | Format::Adm)
            {
                eac3_objects::run(
                    &file,
                    &output,
                    &damf::Options {
                        keep_duplicates,
                        bed_conform: !no_bed_conform,
                        all_events,
                        adm: format == Format::Adm,
                        dolby_origin_tag,
                    },
                )
            } else if eac3::is_eac3(&file).unwrap_or(false) {
                eac3::decode(
                    &file,
                    &output,
                    &eac3::DecodeOptions {
                        format,
                        order,
                        dither: !no_dither,
                    },
                )
            } else if matches!(format, Format::Damf | Format::Adm) {
                damf::run(
                    &file,
                    &output,
                    &damf::Options {
                        keep_duplicates,
                        bed_conform: !no_bed_conform,
                        all_events,
                        adm: format == Format::Adm,
                        dolby_origin_tag,
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
                no_dither,
                worst,
            } => if eac3::is_eac3(&file).unwrap_or(false) {
                eac3::compare(
                    &file,
                    &reference,
                    &eac3::CompareOptions {
                        order,
                        report,
                        skip: reference_skip,
                        dither: !no_dither,
                        worst,
                    },
                )
            } else {
                compare::run(
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
            }
            .map(|equal| {
                if equal {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::from(EXIT_NONCONFORMANT)
                }
            }),
            Command::Eac3Blocks { file, frame } => {
                eac3::blocks(&file, frame).map(|()| ExitCode::SUCCESS)
            }
        };
    match result {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(2)
        }
    }
}
