//! `oadec` command-line entry point.

// the `verify --json` report is one big `json!` literal
#![recursion_limit = "256"]

mod author;
mod compare;
mod damf;
mod decode;
mod eac3;
mod eac3_objects;
mod ecpl_inject;
mod emdf;
mod info;
mod input;
mod integrity;
mod joc_config;
mod joc_offset;
mod oamd;
mod scan;
mod thd_demux;
mod verify;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::compare::RefFormat;
use crate::damf::{InterpolationArg, IsfArg};
use crate::decode::{Format, Order};
use crate::eac3_objects::{JocLowBand, JocOverrides, JocPhase};
use crate::integrity::Verdict;

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
        /// TrueHD: also decode every presentation the stream carries and
        /// evaluate the lossless check word of every restart header, which
        /// the integrity pass alone cannot; several times slower. (E-AC-3
        /// verification decodes every frame already.)
        #[arg(long)]
        decode: bool,
    },
    /// Walk an AC-3 or E-AC-3 stream, find the EMDF containers and report the metadata timing.
    Emdf {
        /// Raw AC-3 or E-AC-3 (.ac3/.ec3/.eac3) elementary stream.
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
        /// Raw TrueHD (.thd/.mlp) or AC-3 / E-AC-3 (.ac3/.ec3) elementary stream.
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
        /// Presentation to decode (0 = 2ch, 1 = 6ch, 2 = 8ch, 3 = 16ch objects);
        /// 2 when absent. The object formats (damf, adm) always decode 3 and
        /// refuse any other value.
        #[arg(short, long)]
        presentation: Option<usize>,
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
        /// DAMF/ADM: also write the loss ledger (what the output could not carry of
        /// the programme) as JSON to this file.
        #[arg(long, value_name = "FILE")]
        loss_report: Option<PathBuf>,
        /// DAMF/ADM: intermediate-spatial-format (ISF) objects, which neither
        /// format can represent: refuse the decode, or write the output without
        /// them and declare the loss (exit 4).
        #[arg(long, value_enum, default_value_t = IsfArg::Error)]
        isf: IsfArg,
        /// ADM: write a programme that is not at 48 kHz although the Dolby Atmos
        /// master ADM profile requires 48 kHz; the file is declared outside the
        /// profile (exit 4). Without it such a programme is refused.
        #[arg(long)]
        adm_allow_non_profile_rate: bool,
        /// DAMF: frame rate written to the `.atmos` header (header data for
        /// picture-locked workflows; event timing is in samples).
        #[arg(long, default_value = "24", value_parser = ["23.976", "24", "25", "29.97", "30"])]
        fps: String,
        /// ADM: how interpolation lengths are written: the profile's fixed 250
        /// samples (the default, what Dolby's converters write) or the source
        /// ramps (BS.2076, outside the profile; marked in the file, refuses
        /// --dolby-origin-tag).
        #[arg(long, value_enum, default_value_t = InterpolationArg::Profile)]
        adm_interpolation: InterpolationArg,
        /// E-AC-3: write only the 5.1-compatible channels of the independent
        /// substream instead of the whole programme, which is what a decoder
        /// limited to 5.1 produces (clause E.2.8.2).
        #[arg(long)]
        core_only: bool,
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
        /// JOC: take the 90-degree phase shift of downmix configurations 3
        /// and 4 back out by rotating every subband alike, the lowest one
        /// included, instead of correcting the lowest one the way the Dolby
        /// decoder does; for measuring what the correction changes.
        #[arg(long)]
        flat_quadrature: bool,
        /// JOC: read a sparse matrix exactly as clause 6.6.2 prints it,
        /// rather than as Dolby's decoder reads it; for measuring the
        /// difference the two corrections make.
        #[arg(long)]
        sparse_as_printed: bool,
        /// JOC: put the steep switch of clause 6.6.5 one slot after the
        /// offset names, the way the printed pseudo-code reads, rather than
        /// where Dolby's decoder puts it; for measuring the difference.
        #[arg(long)]
        steep_as_printed: bool,
        /// JOC measurement: hold the matrices back by this many time slots
        /// instead of the measured alignment; announced on stderr and recorded
        /// in the loss report.
        #[arg(long, hide = true, value_name = "SLOTS")]
        joc_lag: Option<usize>,
        /// JOC measurement: how the lowest subband of a phase-shifted channel
        /// is treated; announced on stderr and recorded in the loss report.
        #[arg(long, hide = true, value_enum)]
        joc_low_band: Option<JocLowBand>,
        /// JOC measurement: which downmix channels carry the 90-degree phase
        /// shift and which way it is taken out, instead of what the downmix
        /// configuration says; announced on stderr and recorded in the loss
        /// report.
        #[arg(long, hide = true, value_name = "CH,CH:+|-|none", value_parser = JocPhase::parse)]
        joc_phase: Option<JocPhase>,
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
        /// E-AC-3: compare only the 5.1-compatible channels of the independent
        /// substream instead of the whole programme, which is what a reference
        /// decoder limited to 5.1 produces (clause E.2.8.2).
        #[arg(long)]
        core_only: bool,
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
        /// Index of the frame group (an independent substream and the
        /// dependent substreams that follow it), counted from 0.
        #[arg(long)]
        frame: u64,
        /// Which substream of the group: 0 is the independent one.
        #[arg(long, default_value_t = 0)]
        part: usize,
        /// Also print the exponents and bit allocation of every coded channel.
        #[arg(long)]
        detail: bool,
    },
    /// Write a Dolby Atmos master from a scene description, so a decode can be
    /// checked against authored metadata rather than against another decoder.
    #[command(hide = true)]
    AtmosAuthor {
        /// Scene description (JSON).
        scene: PathBuf,
        /// Output base name; `.atmos`, `.atmos.metadata` and `.atmos.audio`
        /// are appended.
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Split a Blu-ray audio dump that interleaves TrueHD access units with
    /// the AC-3 core frames of the same track into the two streams.
    ThdDemux {
        /// The interleaved dump.
        file: PathBuf,
        /// Where to write the TrueHD elementary stream.
        #[arg(short, long)]
        output: PathBuf,
        /// Where to write the AC-3 core; without it the core is dropped.
        #[arg(long)]
        core: Option<PathBuf>,
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
    /// Rewrite `joc_dmx_config_idx` (table 47) in every JOC payload of an
    /// E-AC-3 stream and change nothing else, to ask a decoder what it does
    /// with the same audio under a different downmix configuration.
    Eac3JocConfig {
        /// Raw E-AC-3 elementary stream carrying JOC.
        file: PathBuf,
        /// Where to write the relabelled stream.
        #[arg(short, long)]
        output: PathBuf,
        /// The configuration to write: 0 and 3 are the ones encoders use.
        #[arg(long)]
        dmx_config: u8,
    },
    /// Rewrite `joc_offset_ts_bits` (clause 6.3.4.4) in every JOC payload of an
    /// E-AC-3 stream and change nothing else, to ask a decoder where it puts
    /// the steep switch when the field says something different.
    Eac3JocOffset {
        /// Raw E-AC-3 elementary stream carrying JOC.
        file: PathBuf,
        /// Where to write the rewritten stream.
        #[arg(short, long)]
        output: PathBuf,
        /// The value to transmit, 0 to 31; the decoder's `joc_offset_ts` is
        /// one more than this (clause 6.3.4.4).
        #[arg(long)]
        offset_ts_bits: u8,
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
            Command::Verify { file, json, decode } => eac3::is_eac3(&file)
                .and_then(|is| {
                    if is {
                        eac3::verify(&file, json)
                    } else {
                        verify::run(&file, json, decode)
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
                loss_report,
                isf,
                adm_allow_non_profile_rate,
                fps,
                adm_interpolation,
                core_only,
                no_dither,
                no_tpnp,
                ecpl_spec,
                no_clip_gain,
                flat_quadrature,
                sparse_as_printed,
                steep_as_printed,
                joc_lag,
                joc_low_band,
                joc_phase,
            } => if let Err(e) = damf::check_presentation(format, presentation).and_then(|()| {
                damf::check_interpolation(adm_interpolation.into(), dolby_origin_tag)
            }) {
                Err(e)
            } else if (joc_lag.is_some() || joc_low_band.is_some() || joc_phase.is_some())
                && !(eac3::is_eac3(&file).unwrap_or(false)
                    && matches!(format, Format::Damf | Format::Adm))
            {
                // A measurement override that reaches no JOC reconstruction would
                // be read as applied while changing nothing.
                Err(anyhow::anyhow!(
                    "--joc-lag, --joc-low-band and --joc-phase measure the JOC object \
                     decode: they apply to an E-AC-3 stream with --format damf or adm"
                ))
            } else if eac3::is_eac3(&file).unwrap_or(false)
                && matches!(format, Format::Damf | Format::Adm)
            {
                if core_only {
                    // The object programme is the whole programme and the JOC
                    // downmix needs every channel of it, so the two cannot be
                    // asked for together.
                    Err(anyhow::anyhow!(
                        "--core-only writes the 5.1-compatible channels of the independent \
                         substream, which is not an object programme; drop it, or ask for \
                         --format wav or pcm"
                    ))
                } else {
                    eac3_objects::run(
                        &file,
                        &output,
                        &damf::Options {
                            keep_duplicates,
                            bed_conform: !no_bed_conform,
                            all_events,
                            adm: format == Format::Adm,
                            dolby_origin_tag,
                            loss_report: loss_report.clone(),
                            isf: isf.into(),
                            allow_non_profile_rate: adm_allow_non_profile_rate,
                            fps: fps.clone(),
                            interpolation: adm_interpolation.into(),
                            clip_gain: !no_clip_gain,
                            flat_quadrature,
                            sparse_as_printed,
                            steep_as_printed,
                            joc: JocOverrides {
                                lag: joc_lag,
                                low_band: joc_low_band,
                                phase: joc_phase,
                            },
                            core: oadec_eac3::Options {
                                dither: !no_dither,
                                tpnp: !no_tpnp,
                                ecpl_full: ecpl_spec,
                            },
                        },
                    )
                }
            } else if eac3::is_eac3(&file).unwrap_or(false) {
                eac3::decode(
                    &file,
                    &output,
                    &eac3::DecodeOptions {
                        format,
                        order,
                        core_only,
                        dither: !no_dither,
                        tpnp: !no_tpnp,
                        ecpl_full: ecpl_spec,
                    },
                )
                .map(Verdict::from_clean)
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
                        loss_report: loss_report.clone(),
                        isf: isf.into(),
                        allow_non_profile_rate: adm_allow_non_profile_rate,
                        fps: fps.clone(),
                        interpolation: adm_interpolation.into(),
                        // TrueHD carries no JOC, so neither of these apply.
                        clip_gain: false,
                        flat_quadrature: false,
                        sparse_as_printed: false,
                        steep_as_printed: false,
                        // TrueHD carries no E-AC-3 core.
                        core: oadec_eac3::Options::default(),
                        joc: JocOverrides::default(),
                    },
                )
            } else {
                decode::run(
                    &file,
                    &output,
                    &decode::Options {
                        presentation: presentation.unwrap_or(2),
                        format,
                        order,
                        keep_duplicates,
                    },
                )
                .map(Verdict::from_clean)
            }
            .map(|verdict| ExitCode::from(verdict.exit_code())),
            Command::Compare {
                file,
                reference,
                presentation,
                reference_format,
                order,
                keep_duplicates,
                report,
                reference_skip,
                core_only,
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
                        core_only,
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
            Command::Eac3Blocks {
                file,
                frame,
                part,
                detail,
            } => eac3::blocks(&file, frame, part, detail).map(|()| ExitCode::SUCCESS),
            Command::AtmosAuthor { scene, output } => {
                author::run(&scene, &output).map(|()| ExitCode::SUCCESS)
            }
            Command::ThdDemux { file, output, core } => {
                thd_demux::run(&file, &output, core.as_deref()).map(|()| ExitCode::SUCCESS)
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
            Command::Eac3JocConfig {
                file,
                output,
                dmx_config,
            } => joc_config::run(&file, &output, dmx_config).map(|()| ExitCode::SUCCESS),
            Command::Eac3JocOffset {
                file,
                output,
                offset_ts_bits,
            } => joc_offset::run(&file, &output, offset_ts_bits).map(|()| ExitCode::SUCCESS),
        };
    match result {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(2)
        }
    }
}
