//! `oadec` command-line entry point.

mod compare;
mod damf;
mod decode;
mod eac3;
mod eac3_objects;
mod ecpl_inject;
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
        /// E-AC-3: skip transient pre-noise processing (clause E.3.7), which
        /// the reference decoder applies; for measuring what it changes.
        #[arg(long)]
        no_tpnp: bool,
        /// E-AC-3: decode enhanced coupling with the full complex process of
        /// ATSC A/52:2018 clause E.3.5.5 instead of the amplitude-only one
        /// both Dolby decoders implement.
        #[arg(long)]
        ecpl_spec: bool,
        /// JOC: leave the objects at the level of the coded downmix instead of
        /// restoring the clip gain the encoder took off.
        #[arg(long)]
        no_clip_gain: bool,
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
        /// E-AC-3: skip transient pre-noise processing (clause E.3.7), which
        /// the reference decoder applies; for measuring what it changes.
        #[arg(long)]
        no_tpnp: bool,
        /// E-AC-3: decode enhanced coupling with the full complex process of
        /// ATSC A/52:2018 clause E.3.5.5 instead of the amplitude-only one
        /// both Dolby decoders implement.
        #[arg(long)]
        ecpl_spec: bool,
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
    /// Rewrite the standard coupling of an E-AC-3 stream as enhanced coupling,
    /// which no encoder on hand will emit and no stream in the wild carries.
    #[command(hide = true)]
    Eac3EcplInject {
        /// Raw E-AC-3 elementary stream that uses coupling.
        file: PathBuf,
        /// Where to write the converted stream.
        #[arg(short, long)]
        output: PathBuf,
        /// Highest `ecplchaos` code to write (0 to 7). The random values the
        /// chaos term scales are the decoder's own, so anything above zero
        /// makes two decoders differ by design; zero keeps the stream
        /// comparable.
        #[arg(long, default_value_t = 0)]
        chaos: u32,
        /// Write a zero angle everywhere, leaving only the amplitude path.
        #[arg(long)]
        flat_angle: bool,
        /// Write unity amplitude everywhere, so only the dither can differ.
        #[arg(long)]
        flat_amp: bool,
        /// `ecplangleintrp`: 0 or 1 fixed, 2 to alternate per block.
        #[arg(long, default_value_t = 2)]
        interp: u32,
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
                no_tpnp,
                ecpl_spec,
                no_clip_gain,
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
                        clip_gain: !no_clip_gain,
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
                        tpnp: !no_tpnp,
                        ecpl_full: ecpl_spec,
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
                        // TrueHD carries no JOC and so no clip gain.
                        clip_gain: false,
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
                no_tpnp,
                ecpl_spec,
                worst,
            } => if eac3::is_eac3(&file).unwrap_or(false) {
                eac3::compare(
                    &file,
                    &reference,
                    &eac3::CompareOptions {
                        order,
                        tpnp: !no_tpnp,
                        ecpl_full: ecpl_spec,
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
            Command::Eac3EcplInject {
                file,
                output,
                chaos,
                flat_angle,
                flat_amp,
                interp,
            } => ecpl_inject::run(
                &file,
                &output,
                ecpl_inject::Shape {
                    chaos,
                    flat_angle,
                    flat_amp,
                    interp,
                },
            )
            .map(|()| ExitCode::SUCCESS),
        };
    match result {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(2)
        }
    }
}
