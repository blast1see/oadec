//! The commands without the command line.
//!
//! Each function here does what a command does and hands back what it found.
//! Nothing is printed and no process is ended: [`crate::run`] is these calls,
//! or the functions under them, with the printing after. So a test, a fuzz
//! target or a campaign that changes bytes and asks two commands about them
//! gets the verdicts the command line gives, from the code that gives them
//! there.
//!
//! A stream comes as a [`Source`], a file or bytes in memory. A decode to
//! samples writes into a [`PcmSink`], which a file is and a buffer in memory
//! is too; the object outputs are files, because the two formats are closed by
//! going back over what was written. What a decode has to say while it runs,
//! such as the warning for a channel a WAVE mask cannot name, goes line by
//! line to a writer the caller hands in: `std::io::sink()` for nobody.
//!
//! What the command line refuses of an option that does not apply, the call
//! that takes the option refuses in the same words: one that reads as applied
//! while changing nothing is a defect whoever passes it.
//!
//! Every type a call takes or hands back has a name here.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
/// How an AC-3 or E-AC-3 core is decoded on the way to its objects.
pub use oadec_eac3::Options as CoreOptions;
pub use oadec_spatial::{Interpolation, IsfPolicy};
pub use oadec_truehd::Severity;
use oadec_truehd::Tally;
pub use oadec_truehd::disc::Disc;
use serde_json::Value;

pub use crate::damf::{Options as ObjectOptions, WarpArg as Warp, Written as TrueHdObjects};
pub use crate::decode::{
    Decoded as TrueHdPcm, OnError, Order, PcmBytes, PcmFile, PcmSink, WavSample, WavSpec,
};
pub use crate::eac3::Report as Ac3Report;
pub use crate::eac3_objects::{
    JocLowBand, JocOverrides, JocPhase, JocPhaseError, Written as Ac3Objects,
};
pub use crate::emdf::Report as EmdfReport;
pub use crate::info::Report as InfoReport;
pub use crate::integrity::{Findings, Verdict};
pub use crate::oamd::Report as OamdReport;
pub use crate::source::Source;
pub use crate::verify::Report as VerifyReport;
use crate::{damf, decode, eac3, eac3_objects, emdf, info, oamd, verify};

/// What `--census` on an AC-3 family stream is told.
pub(crate) const CENSUS_IS_TRUEHD: &str = "--census counts the TrueHD syntax; the report of an AC-3 \
     or E-AC-3 stream names its coding tools as it is";

/// What the options of the TrueHD checks on an AC-3 family stream are told.
pub(crate) const RULES_ARE_TRUEHD: &str = "--fail-on, --max-per-rule, --trace-fifo and --require-disc \
     apply to a TrueHD stream; the E-AC-3 checks have no rule catalog yet";

/// What `--on-error` on an AC-3 family stream is told.
pub(crate) const ON_ERROR_IS_TRUEHD: &str = "--on-error chooses what a TrueHD decode does with an \
     access unit it cannot read; it does not apply to an E-AC-3 stream";

/// What `--start` on an AC-3 family stream, or with an object output, is told.
pub(crate) const START_IS_TRUEHD_SAMPLES: &str = "--start begins a TrueHD decode at a major sync, which \
     is exact because a restart header re-initialises every substream: it applies to a TrueHD \
     stream with --format pcm or wav";

/// What the JOC measurement overrides on anything but the object decode of an
/// E-AC-3 stream are told.
pub(crate) const JOC_MEASURES_EAC3_OBJECTS: &str = "--joc-lag, --joc-low-band and --joc-phase measure \
     the JOC object decode: they apply to an E-AC-3 stream with --format damf or adm";

/// Whether `source` is an AC-3 or E-AC-3 stream, and not a TrueHD or MLP one.
///
/// # Errors
/// When the source cannot be opened or read.
pub fn is_ac3_family(source: &Source) -> Result<bool> {
    eac3::is_eac3(source)
}

/// What `info` reads of a stream.
#[derive(Debug)]
pub enum Info {
    /// Of a TrueHD or MLP stream.
    TrueHd(info::Report),
    /// Of an AC-3 or E-AC-3 stream.
    Ac3(eac3::Report),
}

impl Info {
    /// The report as `info --json` prints it. `None` for an AC-3 family
    /// stream of which no frame decoded, which the command says in a line.
    ///
    /// # Errors
    /// When a part of the report does not serialise, which none does.
    pub fn json(&self) -> Result<Option<Value>> {
        match self {
            Self::TrueHd(report) => report.json().map(Some),
            Self::Ac3(report) => Ok(report.json()),
        }
    }

    /// Prints the report on stdout, as JSON or as text.
    ///
    /// # Errors
    /// As [`Info::json`].
    pub fn print(&self, json: bool) -> Result<()> {
        match self {
            Self::TrueHd(report) => report.print(json),
            Self::Ac3(report) => {
                report.print(json);
                Ok(())
            }
        }
    }
}

/// `oadec info`: what a stream declares about itself.
///
/// # Errors
/// When the source cannot be read, or holds no stream.
pub fn info(source: &Source) -> Result<Info> {
    Ok(if eac3::is_eac3(source)? {
        Info::Ac3(eac3::report(source)?)
    } else {
        Info::TrueHd(info::report(source)?)
    })
}

/// What `verify` is asked. Everything but nothing here is of the TrueHD
/// checks, and an AC-3 family stream refuses it.
#[derive(Debug, Clone, Default)]
pub struct VerifyOptions {
    /// Decode every presentation the stream carries and evaluate the lossless
    /// check words. An AC-3 family stream is decoded whatever this says.
    pub decode: bool,
    /// The lightest severity that fails the run; `None` for the default,
    /// which is an error.
    pub fail_on: Option<Severity>,
    /// Findings listed in full for each rule (0 lists every one); `None` for
    /// the default. Counts and verdicts never depend on it.
    pub max_per_rule: Option<usize>,
    /// Where to write the decoder-buffer trace, a row an access unit.
    pub trace_fifo: Option<PathBuf>,
    /// Disc formats the stream must fit for the run to pass.
    pub require_disc: Vec<Disc>,
    /// Also report the census of the syntax.
    pub census: bool,
}

/// What `verify` found.
#[derive(Debug)]
pub enum Verified {
    /// In a TrueHD or MLP stream.
    TrueHd(verify::Report),
    /// In an AC-3 or E-AC-3 stream.
    Ac3(eac3::Report),
}

impl Verified {
    /// Whether the stream is clean: exit code 0, where anything else is 7.
    #[must_use]
    pub fn clean(&self) -> bool {
        match self {
            Self::TrueHd(report) => report.clean(),
            Self::Ac3(report) => report.clean(),
        }
    }

    /// The report as `verify --json` prints it. `None` for an AC-3 family
    /// stream of which no frame decoded, which the command says in a line.
    ///
    /// # Errors
    /// When a part of the report does not serialise, which none does.
    pub fn json(&self) -> Result<Option<Value>> {
        match self {
            Self::TrueHd(report) => report.json().map(Some),
            Self::Ac3(report) => Ok(report.json()),
        }
    }

    /// Prints the report on stdout, as JSON or as text.
    ///
    /// # Errors
    /// As [`Verified::json`].
    pub fn print(&self, json: bool) -> Result<()> {
        match self {
            Self::TrueHd(report) => report.print(json),
            Self::Ac3(report) => {
                report.print(json);
                if !json {
                    report.print_verdict();
                }
                Ok(())
            }
        }
    }
}

/// `oadec verify`: the integrity of every access unit or syncframe.
///
/// # Errors
/// When the source cannot be read or holds no stream, and when an option of
/// the TrueHD checks is given for an AC-3 family stream.
pub fn verify(source: &Source, options: &VerifyOptions) -> Result<Verified> {
    if eac3::is_eac3(source)? {
        if options.census {
            return Err(anyhow!(CENSUS_IS_TRUEHD));
        }
        if options.fail_on.is_some()
            || options.max_per_rule.is_some()
            || options.trace_fifo.is_some()
            || !options.require_disc.is_empty()
        {
            // An option that reads as applied while changing nothing is a
            // defect: the E-AC-3 checks have no rule catalog yet.
            return Err(anyhow!(RULES_ARE_TRUEHD));
        }
        return Ok(Verified::Ac3(eac3::report(source)?));
    }
    Ok(Verified::TrueHd(verify::report(
        source,
        &verify::Options {
            decode: options.decode,
            fail_on: options.fail_on.unwrap_or(Severity::Error),
            max_per_rule: options.max_per_rule.unwrap_or(Tally::DEFAULT_MAX_PER_RULE),
            trace_fifo: options.trace_fifo.clone(),
            require_disc: options.require_disc.clone(),
            census: options.census,
        },
    )?))
}

/// `oadec oamd`: the object audio metadata a stream carries. The payloads
/// that `--dump` lists in full are printed as the walk comes to them, so
/// that listing is the command's and is not offered here.
///
/// # Errors
/// When the source cannot be read, or holds no stream.
pub fn oamd(source: &Source) -> Result<oamd::Report> {
    oamd::report(
        source,
        &oamd::Options {
            json: false,
            dump: None,
        },
    )
}

/// `oadec emdf`: the EMDF containers a stream carries. The containers that
/// `--dump` lists in full are printed as the walk comes to them, so that
/// listing is the command's and is not offered here.
///
/// # Errors
/// When the source cannot be read, or holds no stream.
pub fn emdf(source: &Source) -> Result<emdf::Report> {
    emdf::report(
        source,
        &emdf::Options {
            json: false,
            dump: None,
        },
    )
}

/// What a decode to samples is asked.
#[derive(Debug, Clone)]
pub struct PcmOptions {
    /// TrueHD: the presentation to decode; `None` for the default, which is
    /// the 8-channel one, or the widest a DVD-Audio stream carries.
    pub presentation: Option<usize>,
    /// The order the channels are written in.
    pub order: Order,
    /// TrueHD: keep the access units flagged as duplicates at seamless
    /// branches.
    pub keep_duplicates: bool,
    /// TrueHD: begin the output this many seconds into the stream.
    pub start: Option<f64>,
    /// TrueHD: what to do with an access unit that cannot be read; `None`
    /// for the default, which is to conceal it.
    pub on_error: Option<OnError>,
    /// AC-3 family: only the channels of the independent substream.
    pub core_only: bool,
    /// AC-3 family: dither for mantissas without bits.
    pub dither: bool,
    /// AC-3 family: transient pre-noise processing.
    pub tpnp: bool,
    /// AC-3 family: enhanced coupling by the full process of A/52:2018.
    pub ecpl_full: bool,
}

impl Default for PcmOptions {
    /// What `decode` does when it is given no option.
    fn default() -> Self {
        Self {
            presentation: None,
            order: Order::Interchange,
            keep_duplicates: false,
            start: None,
            on_error: None,
            core_only: false,
            dither: true,
            tpnp: true,
            ecpl_full: false,
        }
    }
}

/// What a decode to samples found.
#[derive(Debug)]
pub enum Pcm {
    /// Of a TrueHD or MLP stream: 24-bit integers.
    TrueHd(Box<decode::Decoded>),
    /// Of an AC-3 or E-AC-3 stream: 32-bit floats.
    Ac3(eac3::Report),
}

impl Pcm {
    /// Prints what `decode` prints of the decode: on stderr for a TrueHD
    /// stream, and the report of the pass on stdout for an AC-3 family one.
    pub fn print_summary(&self) {
        match self {
            Self::TrueHd(decoded) => decoded.print_summary(),
            Self::Ac3(report) => report.print(false),
        }
    }

    /// Everything found wrong with what was delivered: its verdict is the
    /// exit code of `decode`. For a TrueHD stream this waits for the checks of
    /// `verify`, which ran beside the decode.
    ///
    /// # Errors
    /// When those checks could not read the stream.
    pub fn findings(self) -> Result<Findings> {
        match self {
            Self::TrueHd(decoded) => decoded.findings(),
            Self::Ac3(report) => Ok(report.findings()),
        }
    }
}

/// `oadec decode --format pcm|wav`: the samples of a stream, into `sink`.
/// What the decode has to say while it runs goes to `remarks`.
///
/// # Errors
/// When the source cannot be read or holds no stream, when the decode cannot
/// go on, when the sink cannot be written, and when an option of the TrueHD
/// decode is given for an AC-3 family stream.
pub fn decode_pcm(
    source: &Source,
    options: &PcmOptions,
    sink: &mut dyn PcmSink,
    remarks: &mut dyn Write,
) -> Result<Pcm> {
    if eac3::is_eac3(source).unwrap_or(false) {
        if options.on_error.is_some() {
            bail!(ON_ERROR_IS_TRUEHD);
        }
        if options.start.is_some() {
            bail!(START_IS_TRUEHD_SAMPLES);
        }
        return Ok(Pcm::Ac3(eac3::decode_into(
            source,
            &eac3::DecodeOptions {
                // the sink is what decides the container
                format: decode::Format::Pcm,
                order: options.order,
                core_only: options.core_only,
                dither: options.dither,
                tpnp: options.tpnp,
                ecpl_full: options.ecpl_full,
            },
            sink,
            remarks,
        )?));
    }
    Ok(Pcm::TrueHd(Box::new(decode::decode(
        source,
        &decode::Options {
            presentation: options.presentation,
            format: decode::Format::Pcm,
            order: options.order,
            keep_duplicates: options.keep_duplicates,
            start: options.start,
            on_error: options.on_error.unwrap_or(OnError::Conceal),
        },
        sink,
        remarks,
    )?)))
}

/// What a decode to an object output wrote.
#[derive(Debug)]
pub enum Objects {
    /// From the object presentation of a TrueHD stream.
    TrueHd(Box<damf::Written>),
    /// From the JOC objects of an E-AC-3 stream.
    Ac3(Box<eac3_objects::Written>),
}

impl Objects {
    /// Says on stderr what was decoded and written, and what the output
    /// could not carry.
    pub fn print_summary(&self) {
        match self {
            Self::TrueHd(written) => written.print_summary(),
            Self::Ac3(written) => written.print_summary(),
        }
    }

    /// Writes the loss ledger as JSON where the options ask for it.
    ///
    /// # Errors
    /// When the file cannot be written.
    pub fn write_loss_report(&self) -> Result<()> {
        match self {
            Self::TrueHd(written) => written.write_loss_report(),
            Self::Ac3(written) => written.write_loss_report(),
        }
    }

    /// Everything found wrong with what was delivered, and whether it was
    /// written with a declared loss: its verdict is the exit code of
    /// `decode`. For a TrueHD stream this waits for the checks of `verify`,
    /// which ran beside the decode.
    ///
    /// # Errors
    /// When those checks could not read the stream.
    pub fn findings(self) -> Result<Findings> {
        match self {
            Self::TrueHd(written) => written.findings(),
            Self::Ac3(written) => Ok(written.findings()),
        }
    }
}

/// `oadec decode --format damf|adm`: the object programme of a stream as a
/// DAMF set or an ADM BWF file at `base`, the output path without its
/// extension. What the decode has to say while it runs goes to `remarks`.
///
/// # Errors
/// When the source cannot be read or holds no object programme, when the
/// decode cannot go on, when the files cannot be written, and when an option
/// is given that the stream has no use for: the JOC measurement overrides for
/// a TrueHD stream, `on_error` for an E-AC-3 one.
pub fn decode_objects(
    source: &Source,
    base: &Path,
    options: &ObjectOptions,
    remarks: &mut dyn Write,
) -> Result<Objects> {
    damf::check_interpolation(options.interpolation, options.dolby_origin_tag)?;
    let ac3 = eac3::is_eac3(source).unwrap_or(false);
    if !ac3 && options.joc != JocOverrides::default() {
        bail!(JOC_MEASURES_EAC3_OBJECTS);
    }
    if ac3 && options.on_error.is_some() {
        bail!(ON_ERROR_IS_TRUEHD);
    }
    Ok(if ac3 {
        Objects::Ac3(Box::new(eac3_objects::write(
            source, base, options, remarks,
        )?))
    } else {
        Objects::TrueHd(Box::new(damf::write(source, base, options, remarks)?))
    })
}
