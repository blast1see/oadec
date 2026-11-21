//! `decode --programme all`, and `decode -p all` with a report: every
//! member of a stream in one pass, each to a file of its own.
//!
//! A member is a programme of an AC-3 family stream or a presentation of a
//! TrueHD stream. The stream is read once ([`decode_reader_every`]) and each
//! member is written to the file its decode alone would write, byte for
//! byte, named by [`programme_file`] or [`presentation_file`]. What the run
//! says of each is what the decode of it alone says, member by member, and
//! the run ends with the exit code of [`combined_exit_code`].
//!
//! `decode -p all` without `--json` or `--report` is not here: it is
//! `oadec_engine::decode::run_every`, as it always was.

use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{Context, Result};
use oadec_engine::api::{PcmFile, PcmOptions, PcmSink, Verdict};
use oadec_engine::decode::{presentation_file, programme_file};
use oadec_engine::source::{STDIN, STDIN_NAME, Watcher, piped_stdin};
use oadec_engine::stream::{
    Every, EveryEnd, EverySink, MemberEnd, PcmFrame, StreamEvent, StreamOptions,
    decode_reader_every,
};
use serde_json::{Value, json};

use crate::progress::Terminal;

/// `--programme` of `decode`: one programme, by the substream id of its
/// independent substream, or every one the stream carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProgrammeArg {
    One(u8),
    All,
}

impl std::str::FromStr for ProgrammeArg {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == "all" {
            return Ok(Self::All);
        }
        match s.parse::<u8>() {
            Ok(n) if n <= 7 => Ok(Self::One(n)),
            _ => Err(format!("a programme is a number, 0 to 7, or all: not {s}")),
        }
    }
}

/// The refusal of what makes no files of samples.
pub(crate) const PROGRAMMES_ARE_SAMPLES: &str = "--programme all writes every programme of an \
     AC-3 or E-AC-3 stream as samples, a file each; an object output is of programme 0: ask for \
     --format pcm or wav";

/// The refusal of what writes one stream, or begins or ends a decode of one.
pub(crate) const PROGRAMMES_ARE_FILES: &str = "--programme all writes every programme to a file \
     of its own (OUT.g0.wav, OUT.g1.wav, ...): name a file, and leave out -o -, --start and \
     --on-layout-change, which are of the decode of one programme";

/// The refusal of the options of the decode of a TrueHD stream.
pub(crate) const PROGRAMMES_TAKE_LITTLE: &str = "--programme all decodes an AC-3 or E-AC-3 \
     stream to samples: it takes --format, --order, --core-only, --no-dither, --no-tpnp, \
     --ecpl-spec, --decorrelate, --json and --report, and none of the options of an object \
     output or of a TrueHD decode";

/// The refusal of every programme and every presentation at once.
pub(crate) const PROGRAMMES_OR_PRESENTATIONS: &str = "--programme all and -p are of two kinds \
     of stream: an AC-3 or E-AC-3 stream has programmes, a TrueHD stream presentations; ask for \
     one";

/// The exit code of a run that decoded every member: the one whose meaning
/// weighs most, and not the largest number. 2, a decode that could not go
/// on, before 7, an output that is not trustworthy, before 4, an output with
/// a declared loss, before 0. 141 is never among them: no member is written
/// to standard output.
pub(crate) fn combined_exit_code(codes: impl IntoIterator<Item = u8>) -> u8 {
    let codes: Vec<u8> = codes.into_iter().collect();
    [2, 7, 4]
        .into_iter()
        .find(|worst| codes.contains(worst))
        .unwrap_or(0)
}

/// What a decode of every member is asked.
#[derive(Debug)]
pub(crate) struct Asked {
    /// The stream: a path, or `-` for standard input.
    pub file: PathBuf,
    /// The output the members' files are named after.
    pub output: PathBuf,
    /// WAVE files, and not headerless samples.
    pub wave: bool,
    /// How the samples of every member are decoded; neither a programme
    /// nor a presentation is named.
    pub pcm: PcmOptions,
    /// Programmes or presentations.
    pub every: Every,
    /// The report as JSON on standard output, instead of the summaries in
    /// text.
    pub json: bool,
    /// Where the JSON report is written as well; `-` is standard output.
    pub report: Option<PathBuf>,
    /// Where the use of the queue, and the reading of the stream, is
    /// written.
    pub pipeline_stats: Option<PathBuf>,
}

/// One member as the run writes it.
struct Member {
    file: PathBuf,
    out: PcmFile,
    events: Vec<StreamEvent>,
    /// Its decode ended with an error of the sink: nothing more of it is
    /// written.
    failed: bool,
}

/// The files of every member, made as each is met.
struct Members<'a> {
    asked: &'a Asked,
    err: &'a mut dyn Write,
    by_id: std::collections::BTreeMap<u8, Member>,
}

impl std::fmt::Debug for Members<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Members")
    }
}

impl Members<'_> {
    fn noun(&self) -> &'static str {
        match self.asked.every {
            Every::Programmes => "programme",
            Every::Presentations => "presentation",
        }
    }

    fn member(&mut self, id: u8) -> Result<&mut Member> {
        self.by_id
            .get_mut(&id)
            .with_context(|| format!("member {id} was not opened"))
    }
}

impl EverySink for Members<'_> {
    fn open(&mut self, id: u8) -> Result<()> {
        let file = match self.asked.every {
            Every::Programmes => programme_file(&self.asked.output, id),
            Every::Presentations => presentation_file(&self.asked.output, usize::from(id)),
        };
        let out = PcmFile::create(&file, self.asked.wave)?;
        self.by_id.insert(
            id,
            Member {
                file,
                out,
                events: Vec::new(),
                failed: false,
            },
        );
        Ok(())
    }

    fn event(&mut self, id: u8, event: &StreamEvent) -> Result<()> {
        let noun = self.noun();
        if let StreamEvent::Remark(line) = event {
            // what the decode of one says, said of its member
            let line = match line.strip_prefix("warning: ") {
                Some(rest) => format!("warning: {noun} {id}: {rest}"),
                None => format!("{noun} {id}: {line}"),
            };
            writeln!(self.err, "{line}")?;
        }
        let member = self.member(id)?;
        if let StreamEvent::Opened { layout, rate, .. } = event {
            member.out.begin(layout.spec(*rate))?;
        }
        member.events.push(event.clone());
        Ok(())
    }

    fn pcm(&mut self, id: u8, frame: PcmFrame<'_>) -> Result<()> {
        let member = self.member(id)?;
        let written = member.out.write(frame.bytes);
        member.failed |= written.is_err();
        Ok(written?)
    }
}

/// Runs the decode of every member, says what it says, and returns the
/// combined exit code.
pub(crate) fn decode(asked: &Asked, err: &mut dyn Write) -> Result<ExitCode> {
    let (reader, name, size): (Box<dyn io::Read + Send>, String, Option<u64>) =
        if asked.file.as_os_str() == STDIN {
            (Box::new(piped_stdin()?), STDIN_NAME.to_string(), None)
        } else {
            let name = asked.file.display().to_string();
            let file = File::open(&asked.file).with_context(|| format!("opening {name}"))?;
            let size = file.metadata().ok().map(|m| m.len());
            (Box::new(file), name, size)
        };
    let watcher: Arc<dyn Watcher> = Arc::new(Terminal);
    let options = StreamOptions {
        name: name.clone(),
        pcm: asked.pcm.clone(),
        // each member keeps the layout of its first group, as the decode of
        // one to a file does
        on_layout_change: None,
        wave: asked.wave,
        watcher: Some(watcher),
        size,
    };
    let mut members = Members {
        asked,
        err,
        by_id: std::collections::BTreeMap::new(),
    };
    let end = decode_reader_every(reader, options, asked.every, &mut members)?;
    let Members { by_id, err, .. } = members;
    let mut written = std::collections::BTreeMap::new();
    for (id, mut member) in by_id {
        member.out.finish()?;
        written.insert(id, (member.file, member.events));
    }
    if let Some(path) = &asked.pipeline_stats {
        let stats = json!({
            "queue_bound": end.queue.bound,
            "queue_high_water": end.queue.high_water,
            "bytes_read": end.bytes,
            "opened": 1,
            "units_waiting_bound": end.queue.units_bound,
            "units_waiting_high_water": end.queue.units_high_water,
        });
        std::fs::write(path, format!("{stats:#}\n"))
            .with_context(|| format!("writing {}", path.display()))?;
    }
    let combined = combined_exit_code(end.members.iter().map(|(_, m)| m.exit_code()));
    let report = report(&name, asked, &end, &written, combined);
    if let Some(path) = &asked.report {
        let text = format!("{report:#}\n");
        if path.as_os_str() == crate::pipe::STDOUT {
            io::stdout().write_all(text.as_bytes())?;
        } else {
            std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))?;
        }
    }
    if asked.json {
        io::stdout().write_all(format!("{report}\n").as_bytes())?;
        return Ok(ExitCode::from(combined));
    }
    let noun = match asked.every {
        Every::Programmes => "programme",
        Every::Presentations => "presentation",
    };
    let mut stdout = io::stdout();
    for (id, member) in &end.members {
        let file = written
            .get(id)
            .map_or_else(String::new, |(file, _)| file.display().to_string());
        writeln!(err, "{noun} {id}: {file}")?;
        match member {
            MemberEnd::Ended(end) => {
                end.print_summary(&mut stdout, &mut *err)?;
                end.findings.report(&mut *err)?;
            }
            MemberEnd::Failed { error, .. } => writeln!(err, "error: {error:#}")?,
        }
    }
    if combined == 0 {
        writeln!(err, "done")?;
    }
    Ok(ExitCode::from(combined))
}

/// The report of a decode of every member, as `--json` and `--report` write
/// it: the stream, every member by its id in ascending order, and the
/// combined exit code. Each member's entry holds what the report of its
/// decode alone holds, and its `id` and `file`; a member whose decode could
/// not go on holds what it wrote before, its `error`, and exit code 2. It
/// holds no time, so the report of a stream is the same on every run.
fn report(
    name: &str,
    asked: &Asked,
    end: &EveryEnd,
    written: &std::collections::BTreeMap<u8, (PathBuf, Vec<StreamEvent>)>,
    combined: u8,
) -> Value {
    let members: Vec<Value> = end
        .members
        .iter()
        .map(|(id, member)| {
            let (file, events) = written
                .get(id)
                .map_or((None, &[][..]), |(f, e)| (Some(f.as_path()), &e[..]));
            member_report(name, asked.wave, *id, file, events, member)
        })
        .collect();
    let key = match asked.every {
        Every::Programmes => "programmes",
        Every::Presentations => "presentations",
    };
    let mut report = json!({
        "stream": name,
        "format": end.format.name(),
        "bytes": end.bytes,
        "combined_exit_code": combined,
    });
    report[key] = Value::Array(members);
    report
}

/// The entry of one member.
///
/// Stop 1: the fields of the report of the decode of one are made by
/// `samples::report`, which this package may not change; until the lead
/// says how the two are to share it (the report of stop 1), the entry holds
/// what is known here, and the tests hold it to the report of the decode of
/// one, field by field.
fn member_report(
    name: &str,
    wave: bool,
    id: u8,
    file: Option<&Path>,
    events: &[StreamEvent],
    member: &MemberEnd,
) -> Value {
    let (verdict, samples, error) = match member {
        MemberEnd::Ended(end) => (
            match end.verdict() {
                Verdict::Clean => "clean",
                Verdict::Lossy => "lossy",
                Verdict::Faulty => "faulty",
            },
            end.samples,
            None,
        ),
        MemberEnd::Failed { error, samples } => ("failed", *samples, Some(format!("{error:#}"))),
    };
    json!({
        "id": id,
        "file": file.map(|f| f.display().to_string()),
        "stream": name,
        "format": if wave { "wav" } else { "pcm" },
        "samples": samples,
        "events": events.len(),
        "verdict": verdict,
        "exit_code": member.exit_code(),
        "error": error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The meaning of a code orders it, not its number: 2 before 7 before 4
    /// before 0, whatever order the members come in.
    #[test]
    fn the_combined_exit_code_is_the_weightiest_meaning_and_not_the_largest_number() {
        for (codes, want) in [
            (vec![], 0),
            (vec![0], 0),
            (vec![0, 0, 0], 0),
            (vec![4, 0], 4),
            (vec![0, 4, 7], 7),
            (vec![7, 4], 7),
            (vec![7, 2], 2),
            (vec![2, 7, 4, 0], 2),
            (vec![0, 2], 2),
            (vec![4, 2], 2),
        ] {
            assert_eq!(combined_exit_code(codes.clone()), want, "{codes:?}");
        }
    }

    #[test]
    fn a_programme_is_a_number_to_seven_or_all() {
        use std::str::FromStr;
        assert_eq!(ProgrammeArg::from_str("all"), Ok(ProgrammeArg::All));
        assert_eq!(ProgrammeArg::from_str("0"), Ok(ProgrammeArg::One(0)));
        assert_eq!(ProgrammeArg::from_str("7"), Ok(ProgrammeArg::One(7)));
        for bad in ["8", "-1", "every", "", "0x1"] {
            assert!(ProgrammeArg::from_str(bad).is_err(), "{bad}");
        }
    }
}
