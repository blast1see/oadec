//! `decode --format pcm|wav`: the samples of a stream, read once as it
//! arrives.
//!
//! The stream is read once, from its file or from standard input, by
//! [`oadec_engine::stream::decode_reader`], and the samples go to a file or
//! to standard output unit by unit (`docs/streaming.md`). What the run says
//! is what a decode to a file has always said, on the same streams: the
//! summary, the warnings and the verdict on stderr, and the report of the
//! pass of an AC-3 family decode on stdout, which goes to stderr instead when
//! the samples take stdout.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{Context, Result};
use oadec_engine::api::{PcmFile, PcmOptions, PcmSink, Verdict};
use oadec_engine::source::{STDIN, STDIN_NAME, Watcher, piped_stdin};
use oadec_engine::stream::{
    LayoutChange, PcmFrame, StreamEnd, StreamEvent, StreamOptions, StreamSink, decode_reader,
    stream_wave_header,
};
use serde_json::{Value, json};

use crate::pipe::{self, EXIT_RECEIVER_CLOSED};
use crate::progress::Terminal;

/// What a decode to samples is asked.
#[derive(Debug)]
pub(crate) struct Asked {
    /// The stream: a path, or `-` for standard input.
    pub file: PathBuf,
    /// The output: a path, or `-` for standard output.
    pub output: PathBuf,
    /// A WAVE output, and not headerless samples.
    pub wave: bool,
    pub pcm: PcmOptions,
    pub on_layout_change: Option<LayoutChange>,
    /// The report as JSON instead of the summary in text.
    pub json: bool,
    /// Where the JSON report is written as well; `-` is stdout.
    pub report: Option<PathBuf>,
    /// Where the use of the queue is written.
    pub pipeline_stats: Option<PathBuf>,
}

/// Where the samples go.
enum Target {
    /// A file. It is made before the stream is read, as it always was, and
    /// its WAVE header waits for the first samples, which say what it holds.
    /// A stream that holds none, or is refused, leaves no file.
    File(PcmFile),
    /// Standard output.
    Stdout(BufWriter<io::StdoutLock<'static>>),
}

/// What the samples and the events of a decode are written to.
struct Sink<'a> {
    target: Target,
    wave: bool,
    /// Where a remark of the decode is said.
    err: &'a mut dyn Write,
    /// What the report lists: everything but the remarks, in order.
    events: Vec<StreamEvent>,
    /// Bytes written to standard output.
    written: u64,
}

impl Sink<'_> {
    fn stdout_error(e: io::Error) -> anyhow::Error {
        anyhow::Error::new(e).context("writing the samples to standard output")
    }

    /// Every sample is in.
    fn finish(&mut self) -> Result<()> {
        match &mut self.target {
            Target::File(file) => file.finish()?,
            Target::Stdout(out) => out.flush().map_err(Self::stdout_error)?,
        }
        Ok(())
    }
}

impl std::fmt::Debug for Sink<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Sink")
    }
}

impl StreamSink for Sink<'_> {
    fn event(&mut self, event: &StreamEvent) -> Result<()> {
        match event {
            StreamEvent::Remark(line) => {
                writeln!(self.err, "{line}")?;
                return Ok(());
            }
            StreamEvent::Opened { layout, rate, .. } => {
                let spec = layout.spec(*rate);
                match &mut self.target {
                    Target::File(file) => file.begin(spec)?,
                    Target::Stdout(out) => {
                        if self.wave {
                            let header = stream_wave_header(spec);
                            out.write_all(&header).map_err(Self::stdout_error)?;
                            self.written += header.len() as u64;
                        }
                    }
                }
            }
            StreamEvent::ConfigChange { .. } | StreamEvent::Finding(_) => {}
        }
        self.events.push(event.clone());
        Ok(())
    }

    fn pcm(&mut self, frame: PcmFrame<'_>) -> Result<()> {
        match &mut self.target {
            Target::File(file) => file.write(frame.bytes)?,
            Target::Stdout(out) => {
                out.write_all(frame.bytes).map_err(Self::stdout_error)?;
                self.written += frame.bytes.len() as u64;
            }
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        if let Target::Stdout(out) = &mut self.target {
            out.flush().map_err(Self::stdout_error)?;
        }
        Ok(())
    }
}

/// Runs the decode, says what it says, and returns the exit code.
pub(crate) fn decode(asked: Asked, err: &mut dyn Write) -> Result<ExitCode> {
    let to_stdout = asked.output.as_os_str() == pipe::STDOUT;
    let stdin = if asked.file.as_os_str() == STDIN {
        Some(piped_stdin()?)
    } else {
        None
    };
    // the output, then the input: the order a decode to a file has always
    // had, which leaves an empty file where the stream cannot be read
    let target = if to_stdout {
        Target::Stdout(BufWriter::with_capacity(1 << 20, io::stdout().lock()))
    } else {
        Target::File(PcmFile::create(&asked.output, asked.wave)?)
    };
    let (reader, name, size): (Box<dyn io::Read + Send>, String, Option<u64>) = match stdin {
        Some(stdin) => (Box::new(stdin), STDIN_NAME.to_string(), None),
        None => {
            let name = asked.file.display().to_string();
            let file = File::open(&asked.file).with_context(|| format!("opening {name}"))?;
            let size = file.metadata().ok().map(|m| m.len());
            (Box::new(file), name, size)
        }
    };
    let watcher: Arc<dyn Watcher> = Arc::new(Terminal);
    let options = StreamOptions {
        name: name.clone(),
        pcm: asked.pcm.clone(),
        // a receiver of headerless samples is never told their width changed
        on_layout_change: asked
            .on_layout_change
            .or(to_stdout.then_some(LayoutChange::Stop)),
        wave: asked.wave,
        watcher: Some(watcher),
        size,
    };
    let mut sink = Sink {
        target,
        wave: asked.wave,
        err,
        events: Vec::new(),
        written: 0,
    };
    let decoded = decode_reader(reader, options, &mut sink).and_then(|end| {
        sink.finish()?;
        Ok(end)
    });
    let end = match decoded {
        Ok(end) => end,
        Err(e) if to_stdout && pipe::receiver_closed(&e) => {
            writeln!(
                sink.err,
                "stopped: the receiver closed standard output after {} bytes",
                sink.written
            )?;
            return Ok(ExitCode::from(EXIT_RECEIVER_CLOSED));
        }
        Err(e) => {
            // A refusal, and a stream that holds none, leave no file: the
            // decode of a file refused before it made one, and took back
            // the one it made of nothing.
            let (gone, e) = match oadec_engine::Error::from(e) {
                oadec_engine::Error::Refused(e) | oadec_engine::Error::NoStream(e) => (true, e),
                oadec_engine::Error::Io(e) | oadec_engine::Error::Stream(e) => (false, e),
            };
            if gone && let Target::File(mut file) = sink.target {
                file.nothing();
            }
            return Err(e);
        }
    };
    let Sink { events, err, .. } = sink;
    if let Some(path) = &asked.pipeline_stats {
        let stats = json!({
            "queue_bound": end.queue.bound,
            "queue_high_water": end.queue.high_water,
            "bytes_read": end.bytes,
            "opened": 1,
        });
        std::fs::write(path, format!("{stats:#}\n"))
            .with_context(|| format!("writing {}", path.display()))?;
    }
    let verdict = end.verdict();
    let report = (asked.json || asked.report.is_some())
        .then(|| report(&name, asked.wave, &end, &events, verdict));
    if let (Some(report), Some(path)) = (&report, &asked.report) {
        let text = format!("{report:#}\n");
        if path.as_os_str() == pipe::STDOUT {
            io::stdout().write_all(text.as_bytes())?;
        } else {
            std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))?;
        }
    }
    if asked.json {
        let line = format!("{}\n", report.as_ref().expect("asked for above"));
        if to_stdout {
            err.write_all(line.as_bytes())?;
        } else {
            io::stdout().write_all(line.as_bytes())?;
        }
        return Ok(ExitCode::from(verdict.exit_code()));
    }
    // what a decode to a file prints on stdout goes to stderr when the
    // samples take stdout
    let mut stdout = io::stdout();
    if to_stdout {
        let mut out = Vec::new();
        end.print_summary(&mut out, &mut *err)?;
        err.write_all(&out)?;
    } else {
        end.print_summary(&mut stdout, &mut *err)?;
    }
    let verdict = end.findings.report(&mut *err)?;
    if matches!(verdict, Verdict::Clean) {
        writeln!(err, "done")?;
    }
    Ok(ExitCode::from(verdict.exit_code()))
}

/// The report of a decode as `--json` and `--report` write it. It holds no
/// time, so the report of a stream is the same on every run.
fn report(
    name: &str,
    wave: bool,
    end: &StreamEnd,
    events: &[StreamEvent],
    verdict: Verdict,
) -> Value {
    let output = events.iter().find_map(|e| match e {
        StreamEvent::Opened { layout, rate, .. } => Some(json!({
            "format": if wave { "wav" } else { "pcm" },
            "channels": layout.channels,
            "rate": rate,
            "mask": layout.mask,
            "sample": sample_name(layout.sample),
        })),
        _ => None,
    });
    let layout = |l: &oadec_engine::stream::Layout| {
        json!({
            "channels": l.channels,
            "mask": l.mask,
            "sample": sample_name(l.sample),
        })
    };
    let events: Vec<Value> = events
        .iter()
        .map(|e| match e {
            StreamEvent::Opened {
                format,
                layout: l,
                rate,
            } => json!({
                "event": "opened",
                "format": format.name(),
                "layout": layout(l),
                "rate": rate,
            }),
            StreamEvent::ConfigChange {
                unit,
                byte_offset,
                what,
                policy,
                old,
                old_rate,
                new,
                new_rate,
            } => json!({
                "event": "config_change",
                "unit": unit,
                "byte_offset": byte_offset,
                "what": what,
                "policy": policy.name(),
                "old": layout(old),
                "old_rate": old_rate,
                "new": new.as_ref().map(layout),
                "new_rate": new_rate,
            }),
            StreamEvent::Finding(f) => json!({
                "event": "finding",
                "unit": f.unit,
                "byte_offset": f.byte_offset,
                "rule": f.rule,
                "message": f.message,
            }),
            StreamEvent::Remark(line) => json!({ "event": "remark", "text": line }),
        })
        .collect();
    json!({
        "stream": name,
        "format": end.format.name(),
        "output": output,
        "samples": end.samples,
        "units": end.units,
        "bytes": end.bytes,
        "events": events,
        "findings": end.findings.entries(),
        "first_problem": end.findings.first(),
        "verdict": match verdict {
            Verdict::Clean => "clean",
            Verdict::Lossy => "lossy",
            Verdict::Faulty => "faulty",
        },
        "exit_code": verdict.exit_code(),
    })
}

fn sample_name(sample: oadec_engine::api::WavSample) -> &'static str {
    match sample {
        oadec_engine::api::WavSample::Int24 => "s24le",
        oadec_engine::api::WavSample::Float32 => "f32le",
    }
}
