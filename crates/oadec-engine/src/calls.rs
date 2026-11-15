//! One call for each thing the engine does, whatever the stream.
//!
//! [`api`](crate::api) is the commands as calls: it hands back, family by
//! family, the report a command prints, and what it could not do in the
//! words the command says. The calls here are the same work behind one
//! shape, for a caller that is not the command line: a call says what kind
//! of stream it read, the report of the checks has the same fields whichever
//! family the stream is of, and what a call could not do is an [`Error`]
//! that says which kind of thing went wrong.
//!
//! | The call | What it does |
//! |---|---|
//! | [`probe`] | what the stream declares of itself |
//! | [`verify`] | the checks, and one report of them |
//! | [`decode`] | the samples of a presentation, into a sink |
//! | [`decode_objects`] | the object programme, as a DAMF set or an ADM BWF file |
//! | [`metadata`] | the object audio metadata and the containers that carry it |

use std::fmt;
use std::io::Write;
use std::path::Path;

use serde_json::Value;

use crate::api::{
    self, EmdfReport, Info, OamdReport, ObjectOptions, Objects, Pcm, PcmOptions, PcmSink, Verified,
    VerifyOptions,
};
use crate::eac3;
use crate::source::Source;

/// The syntax of a stream: what its first access unit or syncframe is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    /// Dolby TrueHD, the FBA syntax.
    TrueHd,
    /// Meridian Lossless Packing as DVD-Audio carries it, the FBB syntax.
    Mlp,
    /// AC-3.
    Ac3,
    /// Enhanced AC-3.
    Eac3,
}

impl Format {
    /// What the format is called.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::TrueHd => "TrueHD",
            Self::Mlp => "MLP",
            Self::Ac3 => "AC-3",
            Self::Eac3 => "E-AC-3",
        }
    }

    /// Whether it is AC-3 or E-AC-3, and not TrueHD or MLP.
    #[must_use]
    pub const fn is_ac3_family(self) -> bool {
        matches!(self, Self::Ac3 | Self::Eac3)
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// What a call could not do, by the kind of thing that went wrong.
///
/// Each kind holds what went wrong with everything that led to it, and is
/// written as the command line writes it after `error: `: the whole of it in
/// one line. So an `Error` names no source, and whoever writes an error and
/// then its sources writes this one once; what lies under it is in
/// [`cause`](Error::cause). The command line ends with exit code 2 for every
/// kind (`docs/exit-codes.md`); a caller that wants to tell a file that is
/// not there from a stream that cannot be decoded has the kind.
#[derive(Debug, thiserror::Error)]
#[error("{0:#}")]
pub enum Error {
    /// The source could not be opened or read, or an output could not be
    /// written.
    Io(anyhow::Error),
    /// The source holds no stream the engine reads.
    NoStream(anyhow::Error),
    /// Something was asked for that does not apply: an option of the other
    /// family of streams, or of the other object format.
    Refused(anyhow::Error),
    /// The stream could not be reported or delivered as it was asked for: a
    /// stream none of whose frames decodes, a decode that could not go on,
    /// or a programme the output cannot carry.
    Stream(anyhow::Error),
}

impl Error {
    /// What went wrong, with everything that led to it: the chain the words
    /// of the error are written from, for a caller that wants what lies at
    /// the bottom of it, the `io::Error` of a file that is not there for one.
    #[must_use]
    pub const fn cause(&self) -> &anyhow::Error {
        match self {
            Self::Io(e) | Self::NoStream(e) | Self::Refused(e) | Self::Stream(e) => e,
        }
    }
}

/// Marks what went wrong as a refusal of something that does not apply.
#[derive(Debug)]
pub(crate) struct Refusal(String);

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refusal {}

/// Marks what went wrong as a source that holds no stream.
#[derive(Debug)]
pub(crate) struct NoStream(String);

impl fmt::Display for NoStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NoStream {}

/// The refusal of something that does not apply, in `words`.
pub(crate) fn refusal(words: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(Refusal(words.into()))
}

/// A source that holds no stream, in `words`.
pub(crate) fn no_stream(words: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(NoStream(words.into()))
}

/// The kind of what went wrong: by what marks it, and where nothing does, by
/// whether reading or writing is what failed.
fn sort(e: anyhow::Error) -> Error {
    if e.chain().any(|cause| cause.is::<Refusal>()) {
        Error::Refused(e)
    } else if e.chain().any(|cause| cause.is::<NoStream>()) {
        Error::NoStream(e)
    } else if e.chain().any(|cause| cause.is::<std::io::Error>()) {
        Error::Io(e)
    } else {
        Error::Stream(e)
    }
}

/// What a stream declares of itself.
#[derive(Debug)]
pub struct Probe {
    /// The syntax of the stream.
    pub format: Format,
    /// What `info` reports of it.
    pub report: Info,
}

/// The syntax of an AC-3 family stream, which the first of its frames that
/// decodes says. A stream with frames none of which decodes has no syntax to
/// name and no report: the command line says so in a line, and a call says it
/// in the same words.
fn ac3_format(source: &Source, syntax: Option<oadec_eac3::Syntax>) -> Result<Format, Error> {
    match syntax {
        Some(oadec_eac3::Syntax::Ac3) => Ok(Format::Ac3),
        Some(oadec_eac3::Syntax::Eac3) => Ok(Format::Eac3),
        None => Err(Error::Stream(anyhow::Error::msg(eac3::no_frames(
            source.name(),
        )))),
    }
}

/// Reads what a stream declares of itself: what `oadec info` reports, and
/// the syntax it is of.
///
/// # Errors
/// When the source cannot be read or holds no stream, and when it holds
/// frames of which none decodes.
pub fn probe(source: &Source) -> Result<Probe, Error> {
    let report = api::info(source).map_err(sort)?;
    let format = match &report {
        Info::TrueHd(report) if report.is_mlp() => Format::Mlp,
        Info::TrueHd(_) => Format::TrueHd,
        Info::Ac3(report) => ac3_format(source, report.syntax())?,
    };
    Ok(Probe { format, report })
}

/// How many findings a rule has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleCount {
    /// The rule, by the name `oadec rules` lists it under.
    pub rule: String,
    /// Its severity in this report: `info`, `warning`, `error` or `fatal`.
    pub severity: String,
    /// How many findings it has.
    pub count: u64,
}

/// The findings of the checks, counted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tally {
    /// Findings that end the reading of the stream.
    pub fatal: u64,
    /// Findings that fail the stream.
    pub error: u64,
    /// Findings that do not fail it.
    pub warning: u64,
    /// Findings that only say something of it.
    pub info: u64,
    /// The rules that have findings, as the report lists them.
    pub rules: Vec<RuleCount>,
}

/// The report of the checks, with the same fields whichever family the
/// stream is of. Each field is what `verify --json` says of it.
#[derive(Debug)]
pub struct VerifyReport {
    /// The syntax of the stream.
    pub format: Format,
    /// Whether the stream passed: what the exit code of `verify` says.
    pub clean: bool,
    /// The first thing found wrong, where anything was.
    pub first_problem: Option<String>,
    /// The findings, counted by severity and by rule.
    pub tally: Tally,
    /// The parts of the syntax the stream exercises, by name.
    pub coverage: Vec<String>,
    /// The stretches of a TrueHD stream that damage reaches, as the report
    /// lists them; none for an AC-3 family stream, whose findings each name
    /// their frame.
    pub regions: Vec<Value>,
    /// The fields of the TrueHD syntax counted, when the census was asked
    /// for.
    pub census: Option<Value>,
    /// What the protection fields of the containers say, counted without a
    /// key. Of a stream that carries no container it says that there are
    /// none.
    pub protection: Value,
    /// The whole report, as `verify --json` prints it.
    pub info: Value,
    /// The report of the family the stream is of, which the command prints.
    pub report: Verified,
}

fn tally_of(diagnostics: &Value) -> Tally {
    let count = |name: &str| diagnostics["counts"][name].as_u64().unwrap_or(0);
    Tally {
        fatal: count("fatal"),
        error: count("error"),
        warning: count("warning"),
        info: count("info"),
        rules: diagnostics["rules"]
            .as_array()
            .map(|rules| {
                rules
                    .iter()
                    .map(|rule| RuleCount {
                        rule: rule["id"].as_str().unwrap_or_default().to_string(),
                        severity: rule["severity"].as_str().unwrap_or_default().to_string(),
                        count: rule["count"].as_u64().unwrap_or(0),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// A value of a report, where the report has it and it is not null.
fn some(value: &Value) -> Option<Value> {
    (!value.is_null()).then(|| value.clone())
}

/// Checks a stream, as `oadec verify` does, and hands back one report of
/// what was found.
///
/// # Errors
/// When the source cannot be read or holds no stream, when an option is
/// given that does not apply to the stream, and when the stream holds frames
/// of which none decodes: there is nothing to report of it then.
pub fn verify(source: &Source, options: &VerifyOptions) -> Result<VerifyReport, Error> {
    let report = api::verify(source, options).map_err(sort)?;
    let format = match &report {
        Verified::TrueHd(report) if report.is_mlp() => Format::Mlp,
        Verified::TrueHd(_) => Format::TrueHd,
        Verified::Ac3(report) => ac3_format(source, report.syntax())?,
    };
    // a report that names a syntax is one that has something to say
    let info = report.json().map_err(sort)?.unwrap_or_default();
    // the containers ride in the extra data of a TrueHD stream and in the
    // skip fields of an E-AC-3 one, and each report counts them where they are
    let protection = if format.is_ac3_family() {
        info["emdf"]["protection"].clone()
    } else {
        info["extra"]["protection"].clone()
    };
    Ok(VerifyReport {
        format,
        clean: report.clean(),
        first_problem: info["first_error"].as_str().map(str::to_string),
        tally: tally_of(&info["diagnostics"]),
        coverage: info["coverage"]
            .as_array()
            .map(|names| {
                names
                    .iter()
                    .filter_map(|name| name.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        regions: info["diagnostics"]["regions"]
            .as_array()
            .cloned()
            .unwrap_or_default(),
        census: some(&info["census"]),
        protection,
        info,
        report,
    })
}

/// Decodes a presentation of a TrueHD or MLP stream, or the programme of an
/// AC-3 or E-AC-3 one, into `sink`, as `oadec decode --format pcm` and `wav`
/// do. What the decode has to say while it runs goes to `remarks`, a line at
/// a time.
///
/// # Errors
/// When the source cannot be read or holds no stream, when an option is
/// given that does not apply to it, when the sink cannot be written to, and
/// when the decode cannot go on.
pub fn decode(
    source: &Source,
    options: &PcmOptions,
    sink: &mut dyn PcmSink,
    remarks: &mut dyn Write,
) -> Result<Pcm, Error> {
    api::decode_pcm(source, options, sink, remarks).map_err(sort)
}

/// Decodes the object programme of a stream to a DAMF set or an ADM BWF file
/// at `base`, as `oadec decode --format damf` and `adm` do. What the decode
/// has to say while it runs goes to `remarks`, a line at a time.
///
/// # Errors
/// When the source cannot be read or holds no stream, when an option is
/// given that does not apply, when the files cannot be written, and when the
/// stream has no object programme the format can carry.
pub fn decode_objects(
    source: &Source,
    base: &Path,
    options: &ObjectOptions,
    remarks: &mut dyn Write,
) -> Result<Objects, Error> {
    api::decode_objects(source, base, options, remarks).map_err(sort)
}

/// The metadata a stream carries beside its audio.
#[derive(Debug)]
pub struct Metadata {
    /// The object audio metadata: what `oadec oamd` reports.
    pub objects: OamdReport,
    /// The containers that carry it and what else rides in them: what
    /// `oadec emdf` reports.
    pub containers: EmdfReport,
}

impl Metadata {
    /// Whether both reports are clean: what the exit codes of the two
    /// commands say.
    #[must_use]
    pub fn clean(&self) -> bool {
        self.objects.clean() && self.containers.clean()
    }
}

/// Reads the metadata a stream carries beside its audio: its object audio
/// metadata, and the containers that carry it. The stream is walked once for
/// each.
///
/// # Errors
/// When the source cannot be read, or holds no stream.
pub fn metadata(source: &Source) -> Result<Metadata, Error> {
    Ok(Metadata {
        objects: api::oamd(source).map_err(sort)?,
        containers: api::emdf(source).map_err(sort)?,
    })
}
