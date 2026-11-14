//! The `oadec` command line: its arguments and what each command runs.
//!
//! The binary is this crate's [`run`] and nothing else, so that what a command
//! does can be reached without starting a process.

// the `verify --json` report is one big `json!` literal
#![recursion_limit = "256"]
// What a command finds is a value, and printing it is a step of its own. A
// function that prints says so, with an `allow` of its own; no other can,
// which is what lets `api` promise that a call says nothing.
#![deny(clippy::print_stdout, clippy::print_stderr)]
// Whatever a call takes or hands back, its caller can write the type of: one
// that is reachable from outside the crate and has no path there is a
// warning, and the lints of this repository let none through.
#![warn(unnameable_types)]

pub mod api;
mod au_dump;
mod author;
mod catalog;
mod census;
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
mod progress;
mod protection;
mod repair;
mod scan;
pub mod source;
mod thd_demux;
mod verify;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::compare::RefFormat;
use crate::damf::{InterpolationArg, IsfArg, WarpArg};
use crate::decode::{Format, OnError, Order};
use crate::eac3_objects::{JocLowBand, JocOverrides, JocPhase};
use crate::integrity::Verdict;
use crate::source::Source;
use oadec_truehd::Severity;

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
        /// Raw TrueHD (.thd/.mlp) or AC-3 / E-AC-3 (.ac3/.ec3) elementary
        /// stream, or `-` to read one from standard input.
        file: PathBuf,
        /// Machine-readable JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Check the integrity of every access unit or frame and report the failure counts.
    Verify {
        /// Raw TrueHD (.thd/.mlp) or AC-3 / E-AC-3 (.ac3/.ec3) elementary
        /// stream, or `-` to read one from standard input.
        file: PathBuf,
        /// Machine-readable JSON instead of text.
        #[arg(long)]
        json: bool,
        /// TrueHD: also decode every presentation the stream carries and
        /// evaluate the lossless check word of every restart header, which
        /// the integrity pass alone cannot; several times slower. AC-3 and
        /// E-AC-3: also make the samples of every frame. verify reads every
        /// frame to its last bit without it and nothing in such a stream
        /// checks its samples, so the report is the same and takes longer.
        #[arg(long)]
        decode: bool,
        /// TrueHD: the lightest severity that fails the run (exit 7). The
        /// default, error, is the verdict verify has always given; warning
        /// also fails on rules no independent source confirms. `oadec rules`
        /// lists every rule with its severity and source.
        #[arg(long, value_name = "SEVERITY", value_parser = ["info", "warning", "error", "fatal"])]
        fail_on: Option<String>,
        /// TrueHD: list at most N findings of each rule in full (default 20,
        /// 0 for all). Every finding is counted whatever the limit, so the
        /// counts and the verdict never change with it.
        #[arg(long, value_name = "N")]
        max_per_rule: Option<usize>,
        /// TrueHD: write what every access unit puts into the decoder buffer,
        /// and when, as CSV.
        #[arg(long, value_name = "FILE")]
        trace_fifo: Option<PathBuf>,
        /// TrueHD: fail the run (exit 7) unless the stream fits these disc
        /// formats as it is -- bd, hddvd, dvda, comma-separated. Every run
        /// reports the fit (Disc:); this makes it part of the verdict.
        #[arg(long, value_name = "DISC", value_delimiter = ',', value_parser = ["bd", "hddvd", "dvda"])]
        require_disc: Vec<String>,
        /// TrueHD: also report the census the Coding tools line is drawn
        /// from: how often each element of the syntax occurs and with which
        /// values, substream by substream. With --json every counter is
        /// listed, the ones at zero included.
        #[arg(long)]
        census: bool,
    },
    /// Print one access unit of a TrueHD stream field by field, with where each
    /// part sits in the file: the place a finding of verify points to, opened up.
    Au {
        /// Raw TrueHD (.thd/.mlp) elementary stream, or `-` to read one from
        /// standard input.
        file: PathBuf,
        /// Index of the access unit, counted from 0 as verify counts them.
        #[arg(long)]
        index: u64,
        /// Machine-readable JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// List the checks verify makes on a TrueHD stream, with the severity each
    /// carries by default and the source each rests on, or explain one.
    Rules {
        /// The rule to explain, such as segment.parity.
        id: Option<String>,
        /// Machine-readable JSON instead of text.
        #[arg(long)]
        json: bool,
        /// The catalog as the Markdown of docs/conformance-rules.md.
        #[arg(long)]
        markdown: bool,
    },
    /// Walk an AC-3, E-AC-3 or TrueHD stream, find the EMDF containers and report the metadata timing.
    Emdf {
        /// Raw AC-3, E-AC-3 (.ac3/.ec3/.eac3) or TrueHD (.thd/.mlp) elementary
        /// stream, or `-` to read one from standard input.
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
        /// Raw TrueHD (.thd/.mlp) or AC-3 / E-AC-3 (.ac3/.ec3) elementary
        /// stream, or `-` to read one from standard input.
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
        /// Raw TrueHD (.thd/.mlp) or AC-3 / E-AC-3 (.ac3/.ec3) elementary
        /// stream, or `-` to read one from standard input.
        file: PathBuf,
        /// Output file.
        #[arg(short, long)]
        output: PathBuf,
        /// Presentation to decode (0 = 2ch, 1 = 6ch, 2 = 8ch, 3 = 16ch objects);
        /// 2 when absent, or for a DVD-Audio (FBB) stream the widest it
        /// carries. The object formats (damf, adm) always decode 3 and refuse
        /// any other value.
        #[arg(short, long)]
        presentation: Option<usize>,
        /// Output container.
        #[arg(long, value_enum, default_value_t = Format::Wav)]
        format: Format,
        /// PCM/WAV: channel order of the samples; `interchange` when absent.
        /// An object output has the order of its format.
        #[arg(long, value_enum)]
        order: Option<Order>,
        /// Keep access units flagged as duplicates at seamless branches.
        #[arg(long)]
        keep_duplicates: bool,
        /// TrueHD: what to do with an access unit that cannot be read. conceal
        /// (the default): write a unit of silence in its place, keep the
        /// substreams it damaged silent until their next restart header, and
        /// fill a stretch of the file that could not be read with the silence
        /// it took, so everything after stays where it belongs in time; the run
        /// says what it concealed and exits 7. stop: end the decode at the
        /// unit (exit 2) and pass over a stretch that could not be read, which
        /// leaves the output that much shorter.
        #[arg(long, value_enum, value_name = "MODE")]
        on_error: Option<OnError>,
        /// TrueHD: begin the output this many seconds into the stream, at the
        /// first access unit at or after it that carries a major sync. What is
        /// written is the tail of a full decode, sample for sample, because a
        /// restart header re-initialises every substream. E-AC-3 is refused:
        /// its decoder carries enhanced coupling, a held frame and a pre-noise
        /// queue across frames, so a frame is not a clean entry point.
        #[arg(long, value_name = "SECONDS")]
        start: Option<f64>,
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
        /// format can represent: refuse the decode (`error`, and what is done
        /// when this is absent), or write the output without them and declare
        /// the loss (`drop`, exit 4).
        #[arg(long, value_enum)]
        isf: Option<IsfArg>,
        /// ADM: write a programme that is not at 48 kHz although the Dolby Atmos
        /// master ADM profile requires 48 kHz; the file is declared outside the
        /// profile (exit 4). Without it such a programme is refused.
        #[arg(long)]
        adm_allow_non_profile_rate: bool,
        /// DAMF: frame rate written to the `.atmos` header (header data for
        /// picture-locked workflows; event timing is in samples); 24 when
        /// absent.
        #[arg(long, value_parser = ["23.976", "24", "25", "29.97", "30"])]
        fps: Option<String>,
        /// ADM: how interpolation lengths are written: the profile's fixed 250
        /// samples (`profile`, what Dolby's converters write and what is done
        /// when this is absent) or the source ramps (`real`: BS.2076, outside
        /// the profile; marked in the file, refuses --dolby-origin-tag).
        #[arg(long, value_enum)]
        adm_interpolation: Option<InterpolationArg>,
        /// DAMF/ADM: name a warp mode in the output: the one the stream declares
        /// (`stream`; a TrueHD stream declares none) or a given one. Without this
        /// the output names none and Dolby's encoders write warping, which is not
        /// what most E-AC-3 streams declare. See docs/dolby-tools.md.
        #[arg(long, value_enum, value_name = "MODE")]
        warp_mode: Option<WarpArg>,
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
        /// Raw TrueHD (.thd/.mlp) or AC-3 / E-AC-3 (.ac3/.ec3) elementary
        /// stream, or `-` to read one from standard input.
        file: PathBuf,
        /// Reference PCM file (headerless, interleaved).
        #[arg(short, long)]
        reference: PathBuf,
        /// TrueHD: presentation to decode; 2 when absent.
        #[arg(short, long)]
        presentation: Option<usize>,
        /// TrueHD: sample format of the reference; s32le when absent. The
        /// reference of an AC-3 or E-AC-3 stream is 32-bit float.
        #[arg(long, value_enum)]
        reference_format: Option<RefFormat>,
        /// Channel order of the reference.
        #[arg(long, value_enum, default_value_t = Order::Interchange)]
        order: Order,
        /// TrueHD: keep access units flagged as duplicates at seamless branches.
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
        /// Raw E-AC-3 (.ec3/.eac3/.ac3) elementary stream, or `-` to read one
        /// from standard input.
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
    /// which few encoders write and no stream of a library carries.
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
    /// Recompute the checksums of a stream that was changed on purpose, so
    /// that a parser takes the change at its word. The protection of the
    /// metadata and the lossless checks are left as they are.
    #[command(hide = true)]
    Repair {
        /// Raw TrueHD (.thd/.mlp) or AC-3 / E-AC-3 (.ac3/.ec3) elementary stream.
        file: PathBuf,
        /// Where to write the stream with its checksums recomputed.
        #[arg(short, long)]
        output: PathBuf,
        /// Machine-readable JSON instead of text.
        #[arg(long)]
        json: bool,
    },
}

/// Exit code when a verification finds non-conformance.
const EXIT_NONCONFORMANT: u8 = 7;

/// The stream a command line names: a file, or standard input for `-`, which
/// is read to its end here, before the command starts.
fn stream_named(file: &std::path::Path) -> anyhow::Result<Source> {
    Ok(Source::named(file)?)
}

/// Runs the command line `args` spells, the program's own name first, and
/// returns the exit code the process is to end with.
///
/// A command line that does not parse ends the process itself, with the usage
/// message and exit code 2, as it always has.
#[allow(clippy::print_stderr, reason = "printing is what this one is for")]
pub fn run<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let cli = Cli::parse_from(args);
    let result = match cli.command {
        Command::Info { file, json } => stream_named(&file)
            .and_then(|source| api::info(&source))
            .and_then(|report| report.print(json))
            .map(|()| ExitCode::SUCCESS),
        Command::Verify {
            file,
            json,
            decode,
            fail_on,
            max_per_rule,
            trace_fifo,
            require_disc,
            census,
        } => stream_named(&file)
            .and_then(|source| {
                api::verify(
                    &source,
                    &api::VerifyOptions {
                        decode,
                        fail_on: fail_on.as_deref().and_then(Severity::from_name),
                        max_per_rule,
                        trace_fifo,
                        require_disc: require_disc
                            .iter()
                            .filter_map(|d| oadec_truehd::disc::Disc::from_id(d))
                            .collect(),
                        census,
                    },
                )
            })
            .and_then(|report| report.print(json).map(|()| report.clean()))
            .map(|clean| {
                if clean {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::from(EXIT_NONCONFORMANT)
                }
            }),
        Command::Emdf { file, json, dump } => stream_named(&file)
            .and_then(|source| emdf::run(&source, &emdf::Options { json, dump }))
            .map(|clean| {
                if clean {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::from(EXIT_NONCONFORMANT)
                }
            }),
        Command::Oamd { file, json, dump } => stream_named(&file)
            .and_then(|source| oamd::run(&source, &oamd::Options { json, dump }))
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
            on_error,
            start,
            no_bed_conform,
            all_events,
            dolby_origin_tag,
            loss_report,
            isf,
            adm_allow_non_profile_rate,
            fps,
            adm_interpolation,
            warp_mode,
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
        } => {
            // An option that is absent is told from one given at its default,
            // which is what lets one that does not apply be refused.
            let given_for_objects = no_bed_conform
                || all_events
                || loss_report.is_some()
                || isf.is_some()
                || fps.is_some()
                || dolby_origin_tag
                || adm_allow_non_profile_rate
                || adm_interpolation.is_some();
            let given_for_adm =
                dolby_origin_tag || adm_allow_non_profile_rate || adm_interpolation.is_some();
            let given_for_damf = fps.is_some();
            let given_for_samples = order.is_some();
            let order = order.unwrap_or(Order::Interchange);
            let isf = isf.unwrap_or(IsfArg::Error);
            let fps = fps.unwrap_or_else(|| "24".to_string());
            let adm_interpolation = adm_interpolation.unwrap_or(InterpolationArg::Profile);
            let verdict = stream_named(&file).and_then(|source| {
                if let Err(e) = damf::check_presentation(format, presentation).and_then(|()| {
                    damf::check_interpolation(adm_interpolation.into(), dolby_origin_tag)
                }) {
                    Err(e)
                } else if given_for_objects && !matches!(format, Format::Damf | Format::Adm) {
                    // Samples have nowhere to carry any of them.
                    Err(anyhow::anyhow!(api::OBJECT_OPTIONS_ARE_OBJECTS))
                } else if given_for_adm && format != Format::Adm {
                    Err(anyhow::anyhow!(api::ADM_OPTIONS_ARE_ADM))
                } else if given_for_damf && format != Format::Damf {
                    Err(anyhow::anyhow!(api::FPS_IS_DAMF))
                } else if given_for_samples && matches!(format, Format::Damf | Format::Adm) {
                    // The beds and the objects of an object output are in the
                    // order its format gives them, whatever is asked.
                    Err(anyhow::anyhow!(api::ORDER_IS_SAMPLES))
                } else if warp_mode.is_some() && !matches!(format, Format::Damf | Format::Adm) {
                    // Samples have nowhere to say it, and an option that reads as
                    // applied while changing nothing is the defect named below.
                    Err(anyhow::anyhow!(
                        "--warp-mode names a warp mode in an object output: it applies to \
                     --format damf or adm"
                    ))
                } else if (joc_lag.is_some() || joc_low_band.is_some() || joc_phase.is_some())
                    && !(eac3::is_eac3(&source).unwrap_or(false)
                        && matches!(format, Format::Damf | Format::Adm))
                {
                    // A measurement override that reaches no JOC reconstruction would
                    // be read as applied while changing nothing.
                    Err(anyhow::anyhow!(api::JOC_MEASURES_EAC3_OBJECTS))
                } else if on_error.is_some() && eac3::is_eac3(&source).unwrap_or(false) {
                    // The E-AC-3 decoder has its own way with a bad frame; an
                    // option that reads as applied while changing nothing is the
                    // defect named below.
                    Err(anyhow::anyhow!(api::ON_ERROR_IS_TRUEHD))
                } else if start.is_some()
                    && (eac3::is_eac3(&source).unwrap_or(false)
                        || matches!(format, Format::Damf | Format::Adm))
                {
                    // An option that reads as applied while changing nothing is the
                    // defect this round fixed for the measurement overrides. A start
                    // is exact only where a restart header re-initialises the
                    // decoder, which is the TrueHD sample output: an E-AC-3 decoder
                    // carries enhanced coupling, a held frame and a pre-noise queue
                    // across frames, and the object writers walk the whole programme
                    // to build one metadata timeline.
                    Err(anyhow::anyhow!(api::START_IS_TRUEHD_SAMPLES))
                } else if (core_only || no_dither || no_tpnp || ecpl_spec)
                    && !eac3::is_eac3(&source).unwrap_or(false)
                {
                    // The four are about the decoder of the AC-3 family, and a
                    // TrueHD decode has none of what they switch.
                    Err(anyhow::anyhow!(api::CORE_DECODE_IS_AC3))
                } else if (no_clip_gain || flat_quadrature || sparse_as_printed || steep_as_printed)
                    && !(eac3::is_eac3(&source).unwrap_or(false)
                        && matches!(format, Format::Damf | Format::Adm))
                {
                    // A reading of the JOC matrices reaches nothing but the JOC
                    // reconstruction, as the hidden overrides above.
                    Err(anyhow::anyhow!(api::JOC_READINGS_ARE_EAC3_OBJECTS))
                } else if keep_duplicates && eac3::is_eac3(&source).unwrap_or(false) {
                    Err(anyhow::anyhow!(api::KEEP_DUPLICATES_IS_TRUEHD))
                } else if presentation.is_some()
                    && eac3::is_eac3(&source).unwrap_or(false)
                    && !matches!(format, Format::Damf | Format::Adm)
                {
                    // With an object output the presentation is checked above,
                    // for both families alike: it is the object programme.
                    Err(anyhow::anyhow!(api::PRESENTATION_IS_TRUEHD))
                } else if eac3::is_eac3(&source).unwrap_or(false)
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
                            &source,
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
                                warp: warp_mode,
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
                                    block_info: false,
                                    samples: true,
                                },
                                // refused above for E-AC-3
                                on_error: None,
                            },
                        )
                    }
                } else if eac3::is_eac3(&source).unwrap_or(false) {
                    eac3::decode(
                        &source,
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
                        &source,
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
                            warp: warp_mode,
                            // TrueHD carries no JOC, so neither of these apply.
                            clip_gain: false,
                            flat_quadrature: false,
                            sparse_as_printed: false,
                            steep_as_printed: false,
                            // TrueHD carries no E-AC-3 core.
                            core: oadec_eac3::Options::default(),
                            joc: JocOverrides::default(),
                            on_error,
                        },
                    )
                } else {
                    decode::run(
                        &source,
                        &output,
                        &decode::Options {
                            presentation,
                            format,
                            order,
                            keep_duplicates,
                            start,
                            on_error: on_error.unwrap_or(OnError::Conceal),
                        },
                    )
                    .map(Verdict::from_clean)
                }
            });
            verdict.map(|verdict| {
                // Said once, here, where every decode path's verdict becomes an
                // exit code -- TrueHD and E-AC-3, samples and objects -- and only
                // for a run that ended clean. A decode that wrote its output and
                // then failed a check must not end on a word that reads as
                // success.
                if matches!(verdict, Verdict::Clean) {
                    eprintln!("done");
                }
                ExitCode::from(verdict.exit_code())
            })
        }
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
        } => {
            let equal = stream_named(&file).and_then(|source| {
                let ac3 = eac3::is_eac3(&source).unwrap_or(false);
                // An option that reads as applied while changing nothing is a
                // defect in a measurement more than anywhere: the number that
                // comes out is taken for the number of the option.
                if ac3 && presentation.is_some() {
                    Err(anyhow::anyhow!(api::PRESENTATION_IS_TRUEHD))
                } else if ac3 && reference_format.is_some() {
                    Err(anyhow::anyhow!(api::REFERENCE_IS_FLOAT))
                } else if ac3 && keep_duplicates {
                    Err(anyhow::anyhow!(api::KEEP_DUPLICATES_IS_TRUEHD))
                } else if !ac3 && (core_only || no_dither || no_tpnp || ecpl_spec) {
                    Err(anyhow::anyhow!(api::CORE_DECODE_IS_AC3))
                } else if !ac3 && worst > 0 {
                    Err(anyhow::anyhow!(api::WORST_IS_AC3))
                } else if ac3 {
                    eac3::compare(
                        &source,
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
                        &source,
                        &reference,
                        &compare::Options {
                            presentation: presentation.unwrap_or(2),
                            format: reference_format.unwrap_or(RefFormat::S32le),
                            order,
                            keep_duplicates,
                            report,
                            skip: reference_skip,
                        },
                    )
                }
            });
            equal.map(|equal| {
                if equal {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::from(EXIT_NONCONFORMANT)
                }
            })
        }
        Command::Au { file, index, json } => stream_named(&file)
            .and_then(|source| au_dump::run(&source, index, json))
            .map(|()| ExitCode::SUCCESS),
        Command::Rules { id, json, markdown } => {
            catalog::run(id.as_deref(), json, markdown).map(|()| ExitCode::SUCCESS)
        }
        Command::Eac3Blocks {
            file,
            frame,
            part,
            detail,
        } => stream_named(&file)
            .and_then(|source| eac3::blocks(&source, frame, part, detail))
            .map(|()| ExitCode::SUCCESS),
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
        Command::Repair { file, output, json } => {
            repair::run(&file, &output, json).map(|()| ExitCode::SUCCESS)
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
