//! `oadec verify`: integrity counters over a whole stream.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use anyhow::Result;
use oadec_truehd::damage::{Region, RegionKind};
use oadec_truehd::disc::{self, Disc, Row};
use oadec_truehd::{
    Location, MAX_PRESENTATIONS, MajorSync, Provenance, RuleId, Severity, StreamConfig, Tally, Unit,
};
use serde::Serialize;
use serde_json::{Value, json};

use crate::decode::{Order, Session};
use crate::source::Source;
use crate::{input, scan};

/// The lossless check words one presentation's decode evaluated.
#[derive(Debug, Clone, Serialize)]
pub struct PresentationChecks {
    pub presentation: usize,
    pub evaluated: u64,
    pub failed: u64,
    /// Checks not evaluated because their section spans a seamless branch.
    pub skipped: u64,
    /// The first problem the decoder described: a failed check, or a fault
    /// the integrity pass counts as well.
    pub first_problem: Option<String>,
    /// Why the decode stopped before the end of the stream, if it did.
    pub error: Option<String>,
    /// Access units whose samples needed more bits than `max_bits` allows,
    /// which `decode` counts against its verdict.
    pub max_bits_violations: u64,
    /// Samples beyond 24 bits, which every writer can only clamp and `decode`
    /// counts against its verdict.
    pub clipped: u64,
}

/// What `verify --decode` adds to the integrity pass.
#[derive(Debug, Clone, Default, Serialize)]
pub struct LosslessChecks {
    pub evaluated: u64,
    pub failed: u64,
    pub skipped: u64,
    pub per_presentation: Vec<PresentationChecks>,
}

impl LosslessChecks {
    /// Whether every check held and every presentation decoded to the end.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.failed == 0
            && self
                .per_presentation
                .iter()
                .all(|p| p.error.is_none() && p.max_bits_violations == 0 && p.clipped == 0)
    }
}

/// Access units handed to the decodes of the presentations at a time: enough
/// of them that handing them over costs nothing beside decoding them.
const UNITS_AT_A_TIME: usize = 64;

/// How many times that many units a decode may have waiting for it before
/// the reader of the stream waits for the decode.
const WAITING: usize = 2;

/// Decodes every presentation the stream carries and collects the lossless
/// check words the decoder evaluated.
///
/// The integrity pass parses every segment and checks its parity and CRC, but
/// it makes no samples, so the check word a restart header carries over the
/// decoded output of the section before it was never evaluated by `verify` --
/// only by a decode, and only for the one presentation that decode asked for.
/// A presentation that is a copy of another is not decoded twice: its samples,
/// and so its checks, are those of the other.
///
/// The stream is read once, on the thread that calls, and every presentation
/// is decoded on a thread of its own from the units all of them are handed:
/// the whole takes as long as the widest presentation and not as long as all
/// of them.
fn lossless_checks(source: &Source, ms: &MajorSync) -> Result<LosslessChecks> {
    let config = StreamConfig::from_major_sync(ms)?;
    let carried: Vec<usize> = (0..MAX_PRESENTATIONS)
        .filter(|&p| config.presentations.is_carried(p))
        .collect();
    let per_presentation = std::thread::scope(|scope| -> Result<Vec<PresentationChecks>> {
        let mut hand_over = Vec::with_capacity(carried.len());
        let mut decodes = Vec::with_capacity(carried.len());
        for &presentation in &carried {
            let (to, units) = std::sync::mpsc::sync_channel::<Arc<Vec<Unit>>>(WAITING);
            hand_over.push(to);
            decodes.push(scope.spawn(move || presentation_checks(presentation, &units)));
        }
        let give = |units: Vec<Unit>| {
            let units = Arc::new(units);
            for to in &hand_over {
                // a decode that has stopped at a fault takes no more of
                // them, and that is no fault of the hand-over
                let _ = to.send(Arc::clone(&units));
            }
        };
        let mut units = Vec::with_capacity(UNITS_AT_A_TIME);
        let read = input::for_each_unit(source, |unit| {
            units.push(unit);
            if units.len() == UNITS_AT_A_TIME {
                give(std::mem::replace(
                    &mut units,
                    Vec::with_capacity(UNITS_AT_A_TIME),
                ));
            }
            Ok(())
        });
        // the units after the last full hand-over
        give(units);
        // the decodes end when there is nothing more to hand them
        drop(hand_over);
        let said = decodes
            .into_iter()
            .map(|decode| {
                decode
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect();
        read.map(|_| said)
    })?;
    let mut checks = LosslessChecks::default();
    for p in per_presentation {
        checks.evaluated += p.evaluated;
        checks.failed += p.failed;
        checks.skipped += p.skipped;
        checks.per_presentation.push(p);
    }
    Ok(checks)
}

/// Decodes one presentation from the units it is handed, and says what its
/// lossless checks came to. A decode that stops at a fault takes no more
/// units: the others go on without it.
fn presentation_checks(
    presentation: usize,
    units: &std::sync::mpsc::Receiver<Arc<Vec<Unit>>>,
) -> PresentationChecks {
    let mut session = Session::new(presentation, false, Order::Stream);
    let mut error = None;
    let mut clipped = 0u64;
    'stream: for handed in units {
        for unit in handed.iter() {
            // no start is asked for here, so a session has nothing to remark
            match session.decode(unit, &mut std::io::sink()) {
                // counted with the rule every writer clamps by, so the count
                // is the one `decode` reports for the same presentation
                Ok(Some(frame)) => {
                    for row in frame.pcm {
                        for &ch in frame.order {
                            clipped += u64::from(oadec_spatial::clamp_i24(row[ch]).1);
                        }
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    error = Some(format!("{e:#}"));
                    break 'stream;
                }
            }
        }
    }
    match session.stats() {
        Some(s) => PresentationChecks {
            presentation,
            evaluated: s.lossless_checks,
            failed: s.lossless_mismatches,
            skipped: s.lossless_checks_skipped,
            first_problem: s.first_problem.clone(),
            error,
            max_bits_violations: s.max_bits_violations,
            clipped,
        },
        None => PresentationChecks {
            presentation,
            evaluated: 0,
            failed: 0,
            skipped: 0,
            first_problem: None,
            error,
            max_bits_violations: 0,
            clipped,
        },
    }
}

/// The scan of `verify --decode` and, beside it, the decodes of the
/// presentations: the scan on a thread of its own, the decodes begun from
/// this one as soon as the scan has the major sync they are configured from.
/// The second of the pair is `None` for a stream in which the scan found no
/// major sync, and the error of the decodes where they could not go on.
///
/// The stream is read twice, as it was when one followed the other, and the
/// two readers are the scan and what hands the units to the decodes. This
/// thread is the one that draws how far the command has got, and what it
/// draws is the slower of the two. Where the scan fails, the error is the
/// one it failed with, as it was when the scan came first: it is an error of
/// reading the stream, which the other reader does not get past either.
fn scan_beside_the_decodes(
    source: &Source,
    opts: &Options,
) -> Result<(Box<scan::Scan>, Option<Result<LosslessChecks>>)> {
    std::thread::scope(|scope| {
        let (tell, told) = std::sync::mpsc::channel::<MajorSync>();
        let scanning = scope.spawn(move || {
            scan::scan_announcing(
                source,
                opts.max_per_rule,
                opts.trace_fifo.is_some(),
                &|| true,
                &|sync| {
                    let _ = tell.send(sync.clone());
                },
            )
            // A scan is large. It goes onto the heap here, on the thread that
            // made it: handed back by value it stood on the stack of the
            // thread that waits for it once for every function it passed
            // through, which a build without optimisation does not have room
            // for.
            .map(Box::new)
        });
        // told once, or not at all when the scan ends without a major sync
        let checks = told.recv().ok().map(|sync| lossless_checks(source, &sync));
        let scan = scanning
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))?;
        Ok((scan, checks))
    })
}

/// What `verify` concludes about a stream, for the commands that report its
/// metadata.
///
/// `docs/exit-codes.md` has every command that delivers metadata decide with
/// the faults `verify` counts. `emdf` and `oamd` walk only what they report, and
/// each time they re-derived the verdict from that walk a fault it does not read
/// was missed: a byte skipped, a failed CRC, a JOC payload, the audio of a
/// substream. They take this verdict from the same checks instead.
#[derive(Debug, Clone, Serialize)]
pub struct StreamCheck {
    /// Whether `verify` would call the stream clean.
    pub clean: bool,
    /// The first problem `verify` names.
    pub first_problem: Option<String>,
}

/// Runs the checks of `verify` (without `--decode`) over a stream of either
/// codec. Given up with an error at the first access unit or syncframe at
/// which `go_on` says no.
fn stream_check(source: &Source, go_on: &dyn Fn() -> bool) -> Result<StreamCheck> {
    if crate::eac3::is_eac3(source)? {
        return crate::eac3::stream_check(source, go_on);
    }
    let scan = scan::scan_while(source, Tally::DEFAULT_MAX_PER_RULE, false, go_on)?;
    // the default verdict of `verify`: every counter it has always read, and
    // the rules without a counter that fail a stream by default
    Ok(StreamCheck {
        clean: scan.first_major_sync.is_some() && scan.diag.tally.passes(Severity::Error),
        first_problem: scan.first_error.or_else(|| {
            scan.diag
                .tally
                .iter()
                .find(|(r, _)| r.info().severity >= Severity::Error)
                .map(|(r, t)| format!("{}: {}", r.info().id, t.first.message))
        }),
    })
}

/// The checks of `verify` going over a stream on a thread of their own, for a
/// command that reads the same stream in a pass of its own meanwhile.
///
/// The thread does not outlive this value. A command that ends early on an
/// error of its own drops it, and the check is then told to stop, which it
/// does at its next access unit or syncframe, and is waited for. Until this
/// held the thread it was left to run on behind a command that had returned,
/// which ended with the process and so was harmless only in a process.
#[derive(Debug)]
pub struct StreamChecking {
    thread: Option<std::thread::JoinHandle<Result<StreamCheck>>>,
    given_up: Arc<AtomicBool>,
}

/// Starts the checks of `verify` on a thread of their own.
pub fn spawn_stream_check(source: &Source) -> StreamChecking {
    spawn_under(source, Arc::new(AtomicBool::new(false)))
}

/// Starts the checks on a thread that gives the stream up once `given_up`
/// is set.
fn spawn_under(source: &Source, given_up: Arc<AtomicBool>) -> StreamChecking {
    let source = source.clone();
    let flag = Arc::clone(&given_up);
    StreamChecking {
        thread: Some(std::thread::spawn(move || {
            stream_check(&source, &|| !flag.load(Ordering::Relaxed))
        })),
        given_up,
    }
}

/// The verdict of a check started with [`spawn_stream_check`].
pub fn join_stream_check(mut check: StreamChecking) -> Result<StreamCheck> {
    let thread = check
        .thread
        .take()
        .expect("the thread is there until it is joined");
    thread
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

impl Drop for StreamChecking {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            self.given_up.store(true, Ordering::Relaxed);
            // Nobody is left to hear its verdict, or a panic of its own:
            // the command is already on its way out with an error.
            let _ = thread.join();
        }
    }
}

/// How `verify` judges a TrueHD stream and how much of what it found it lists.
#[derive(Debug, Clone)]
pub struct Options {
    /// Decode every presentation and evaluate the lossless check words.
    pub decode: bool,
    /// The lightest severity that fails the run.
    pub fail_on: Severity,
    /// Findings listed in full per rule (0 lists every one); counts are
    /// never limited.
    pub max_per_rule: usize,
    /// Where to write the decoder-buffer trace, one row per access unit.
    pub trace_fifo: Option<std::path::PathBuf>,
    /// Disc formats the stream must fit for the run to pass.
    pub require_disc: Vec<Disc>,
    /// Also report the census: how often each element of the syntax occurs.
    pub census: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            decode: false,
            fail_on: Severity::Error,
            max_per_rule: Tally::DEFAULT_MAX_PER_RULE,
            trace_fifo: None,
            require_disc: Vec::new(),
            census: false,
        }
    }
}

/// The substreams the stream declares, which are the ones a census is of.
fn carried_substreams(scan: &scan::Scan) -> usize {
    scan.config
        .as_ref()
        .map_or(0, |c| usize::from(c.substreams).min(4))
}

/// Writes what every unit puts into the decoder buffer as CSV.
fn write_trace(path: &Path, layouts: &[oadec_truehd::buffer::UnitLayout]) -> Result<()> {
    use std::io::Write as _;
    let mut w = std::io::BufWriter::new(std::fs::File::create(path)?);
    writeln!(
        w,
        "unit,offset,bytes,input_timing,major_sync,dir0,dir1,dir2,dir3,ss0,ss1,ss2,ss3,extra,out0,out1,out2,out3"
    )?;
    for l in layouts {
        let ot = |i: usize| l.output_timing[i].map_or_else(String::new, |v| v.to_string());
        writeln!(
            w,
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            l.index,
            l.offset,
            l.len_bytes,
            l.input_timing,
            l.major_sync_bytes,
            l.directory_bytes[0],
            l.directory_bytes[1],
            l.directory_bytes[2],
            l.directory_bytes[3],
            l.substream_bytes[0],
            l.substream_bytes[1],
            l.substream_bytes[2],
            l.substream_bytes[3],
            l.extra_bytes,
            ot(0),
            ot(1),
            ot(2),
            ot(3)
        )?;
    }
    w.flush()?;
    Ok(())
}

/// Records what the decodes of `verify --decode` found as findings.
fn record_lossless(diag: &mut Tally, checks: &LosslessChecks) {
    for p in &checks.per_presentation {
        if p.failed > 0 {
            diag.add(
                RuleId::DecodeLosslessCheck,
                Location::default(),
                p.failed,
                || {
                    format!(
                        "presentation {}: {} of {} check words failed; first: {}",
                        p.presentation,
                        p.failed,
                        p.evaluated,
                        p.first_problem.as_deref().unwrap_or("-")
                    )
                },
            );
        }
        if let Some(e) = &p.error {
            diag.add(RuleId::DecodeStopped, Location::default(), 1, || {
                format!("presentation {}: {e}", p.presentation)
            });
        }
        if p.max_bits_violations > 0 {
            diag.add(
                RuleId::DecodeMaxBits,
                Location::default(),
                p.max_bits_violations,
                || {
                    format!(
                        "presentation {}: {} access units need more bits than max_bits",
                        p.presentation, p.max_bits_violations
                    )
                },
            );
        }
        if p.clipped > 0 {
            diag.add(
                RuleId::DecodeClipped,
                Location::default(),
                p.clipped,
                || {
                    format!(
                        "presentation {}: {} samples beyond 24 bits",
                        p.presentation, p.clipped
                    )
                },
            );
        }
    }
}

/// `h:mm:ss.mmm` of a sample position.
pub(crate) fn clock(seconds: f64) -> String {
    let ms = (seconds * 1000.0).round() as u64;
    format!(
        "{}:{:02}:{:02}.{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

/// The time at the start of access unit `unit`, by the nominal count of
/// samples per unit (a gap or a branch is not accounted for).
fn seconds_at(scan: &scan::Scan, unit: u64) -> Option<f64> {
    let cfg = scan.config.as_ref()?;
    Some(unit as f64 * f64::from(cfg.samples_per_au) / f64::from(cfg.sampling_frequency))
}

/// A location as text: `au 101 @0x1A2B4+3 ss1 0:00:02.104`.
fn location_text(scan: &scan::Scan, at: &Location) -> String {
    let mut parts = Vec::new();
    if let Some(u) = at.unit {
        parts.push(format!("au {u}"));
    }
    if let Some(o) = at.offset {
        parts.push(match at.bit {
            Some(b) => format!("@0x{o:X}+{b}"),
            None => format!("@0x{o:X}"),
        });
    }
    if let Some(ss) = at.substream {
        parts.push(format!("ss{ss}"));
    }
    if let Some(t) = at.unit.and_then(|u| seconds_at(scan, u)) {
        parts.push(clock(t));
    }
    if parts.is_empty() {
        "--".to_string()
    } else {
        parts.join(" ")
    }
}

fn location_json(scan: &scan::Scan, at: &Location) -> Value {
    json!({
        "unit": at.unit,
        "offset": at.offset,
        "bit": at.bit,
        "substream": at.substream,
        "seconds": at.unit.and_then(|u| seconds_at(scan, u)),
    })
}

/// The `buffer` object of `verify --json`: the decoder-buffer model's peaks.
fn buffer_json(scan: &scan::Scan) -> Value {
    let (Some(m), Some(cfg)) = (&scan.fifo, &scan.config) else {
        return Value::Null;
    };
    let fs = f64::from(cfg.sampling_frequency);
    let substreams = usize::from(cfg.substreams).min(4);
    json!({
        "model": "docs/truehd.md, Decoder buffer",
        "max_latency": {
            "samples": m.max_latency.value,
            "ms": m.max_latency.value as f64 * 1000.0 / fs,
            "unit": m.max_latency.unit,
        },
        "min_latency": m.min_latency.map(|(v, u)| json!({"samples": v, "unit": u})),
        "max_data_rate": {"bps": m.max_rate.value, "unit": m.max_rate.unit},
        "substreams": (0..substreams).map(|s| json!({
            "depth": m.depth[s].value,
            "unit": m.depth[s].unit,
            "allowed": m.allowed(s),
        })).collect::<Vec<_>>(),
        "decoders": (0..4).filter(|&p| m.decoder_mask(p) != 0).map(|p| json!({
            "presentation": p,
            "substream_mask": m.decoder_mask(p),
            "depth": m.depth[4 + p].value,
            "unit": m.depth[4 + p].unit,
        })).collect::<Vec<_>>(),
        "whole_stream": {
            "depth": m.depth[8].value,
            "unit": m.depth[8].unit,
            "buffer_every_decoder_has": oadec_truehd::buffer::MIN_FIFO_BYTES,
        },
        "restarts": m.restarts,
    })
}

/// The `Buffer:` lines of the text report.
#[allow(clippy::print_stdout, reason = "printing is what this one is for")]
fn print_buffer(scan: &scan::Scan) {
    let (Some(m), Some(cfg)) = (&scan.fifo, &scan.config) else {
        return;
    };
    let fs = f64::from(cfg.sampling_frequency);
    println!(
        "Buffer:            delay up to {} samples ({:.1} ms, au {}); delivery up to {:.3} Mbit/s (au {})",
        m.max_latency.value,
        m.max_latency.value as f64 * 1000.0 / fs,
        m.max_latency.unit,
        m.max_rate.value as f64 / 1e6,
        m.max_rate.unit
    );
    let substreams = usize::from(cfg.substreams).min(4);
    let depths = (0..substreams)
        .map(|s| format!("{}/{}", m.depth[s].value, m.allowed(s)))
        .collect::<Vec<_>>()
        .join(", ");
    let decoders = (0..4)
        .filter(|&p| m.decoder_mask(p) != 0)
        .map(|p| format!("p{p} {}", m.depth[4 + p].value))
        .collect::<Vec<_>>()
        .join(", ");
    println!(
        "                   substream depth {depths} bytes; decoders {decoders}; stream {}/{} bytes",
        m.depth[8].value,
        oadec_truehd::buffer::MIN_FIFO_BYTES
    );
}

/// The rules whose findings say a stream is delivered outside the bounds of
/// any TrueHD stream, which keep it off a film disc.
const DELIVERY_RULES: [RuleId; 6] = [
    RuleId::BufferSubstreamDepth,
    RuleId::BufferStreamDepth,
    RuleId::BufferLatencyOver75ms,
    RuleId::BufferLatencyUnderFrame,
    RuleId::RateOverDeclaredPeak,
    RuleId::RateOverCeiling,
];

/// How the stream fits each disc format, from its first major sync and the
/// delivery findings of the scan.
fn disc_rows(scan: &scan::Scan) -> Option<[Row; 3]> {
    let ms = scan.first_major_sync.as_ref()?;
    let mut facts = disc::Facts::from_major_sync(ms);
    facts.peak_delivery_bps = scan.fifo.as_ref().map_or(0, |m| m.max_rate.value);
    facts.delivery = DELIVERY_RULES
        .iter()
        .filter_map(|r| scan.diag.tally.get(*r).map(|t| (r.info().id, t.count)))
        .collect();
    Some(disc::judge(&facts))
}

/// The `disc` object of `verify --json`.
fn disc_json(rows: Option<&[Row; 3]>, required: &[Disc]) -> Value {
    let Some(rows) = rows else {
        return Value::Null;
    };
    json!({
        "model": "docs/truehd.md, Disc formats",
        "required": required.iter().map(|d| d.id()).collect::<Vec<_>>(),
        "rows": rows.iter().map(|r| json!({
            "disc": r.disc.id(),
            "name": r.disc.name(),
            "verdict": r.verdict.word(),
            "reasons": r.verdict.reasons().iter().map(|x| json!({
                "text": x.text,
                "source": x.source,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

/// The `Disc:` lines of the text report: the verdicts, then each reason once.
#[allow(clippy::print_stdout, reason = "printing is what this one is for")]
fn print_disc(rows: Option<&[Row; 3]>, required: &[Disc]) {
    let Some(rows) = rows else {
        return;
    };
    let verdicts = rows
        .iter()
        .map(|r| format!("{} {}", r.disc.name(), r.verdict.word()))
        .collect::<Vec<_>>()
        .join(", ");
    let required = if required.is_empty() {
        String::new()
    } else {
        format!(
            " (required: {})",
            required
                .iter()
                .map(|d| d.name())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    println!("Disc:              {verdicts}{required}");
    let mut said = Vec::new();
    for reason in rows.iter().flat_map(|r| r.verdict.reasons()) {
        if !said.contains(&reason.text) {
            println!("                   {}", reason.text);
            said.push(reason.text.clone());
        }
    }
}

/// Records a finding for each disc format the run requires and the stream
/// does not fit.
fn record_required_discs(diag: &mut Tally, rows: Option<&[Row; 3]>, required: &[Disc]) {
    for disc in required {
        let row = rows.and_then(|rows| rows.iter().find(|r| r.disc == *disc));
        if row.is_some_and(|r| r.verdict.is_yes()) {
            continue;
        }
        diag.add(RuleId::DiscRequired, Location::default(), 1, || {
            let reasons = row.map_or_else(
                || "the stream declares nothing to judge".to_string(),
                |r| {
                    r.verdict
                        .reasons()
                        .iter()
                        .map(|x| x.text.as_str())
                        .collect::<Vec<_>>()
                        .join("; ")
                },
            );
            format!(
                "{} {}: {reasons}",
                disc.name(),
                row.map_or("no", |r| r.verdict.word())
            )
        });
    }
}

/// The `diagnostics` object of `verify --json`.
fn diagnostics_json(scan: &scan::Scan, fail_on: Severity) -> Value {
    let d = &scan.diag.tally;
    let rules: Vec<Value> = d
        .iter()
        .map(|(rule, t)| {
            let info = rule.info();
            json!({
                "id": info.id,
                "severity": info.severity.name(),
                "count": t.count,
                "magnitude": t.magnitude,
                "provenance": info.provenance.iter().map(Provenance::kind).collect::<Vec<_>>(),
                "legacy_counter": info.legacy_counter,
                "first": location_json(scan, &t.first.at),
                "first_message": t.first.message,
                "last": location_json(scan, &t.last),
                "listed": t.listed.len(),
                "suppressed": t.suppressed(),
            })
        })
        .collect();
    let findings: Vec<Value> = d
        .iter()
        .flat_map(|(_, t)| t.listed.iter())
        .map(|f| {
            let mut v = location_json(scan, &f.at);
            v["rule"] = json!(f.rule.info().id);
            v["severity"] = json!(f.rule.info().severity.name());
            v["magnitude"] = json!(f.magnitude);
            v["message"] = json!(f.message);
            v
        })
        .collect();
    let counts: serde_json::Map<String, Value> = Severity::ALL
        .iter()
        .map(|s| (s.name().to_string(), json!(d.count_at(*s))))
        .collect();
    json!({
        "schema": 1,
        "fail_on": fail_on.name(),
        "worst": d.worst().map(Severity::name),
        "max_per_rule": d.max_per_rule(),
        "counts": counts,
        "rules": rules,
        "findings": findings,
        "regions": scan.diag.regions.iter().map(|r| region_json(scan, r)).collect::<Vec<_>>(),
        "regions_overflow": scan.diag.regions_overflow,
    })
}

/// The presentations that read a damaged substream (audio), carry the object
/// metadata (metadata), or all of them (lost bytes).
fn presentations_hit(scan: &scan::Scan, region: &Region) -> Vec<usize> {
    let Some(cfg) = scan.config.as_ref() else {
        return Vec::new();
    };
    let map = cfg.presentations;
    (0..MAX_PRESENTATIONS)
        .filter(|&p| map.is_carried(p))
        .filter(|&p| match region.kind {
            RegionKind::Audio => map.mask(p) & region.substreams != 0,
            RegionKind::Metadata => p == 3,
            RegionKind::Lost => true,
        })
        .collect()
}

fn region_json(scan: &scan::Scan, r: &Region) -> Value {
    json!({
        "kind": r.kind.name(),
        "first_unit": r.first_unit,
        "last_unit": r.last_unit,
        "start_seconds": seconds_at(scan, r.first_unit),
        "end_seconds": seconds_at(scan, r.last_unit + 1),
        "substreams": (0..8).filter(|s| r.substreams >> s & 1 == 1).collect::<Vec<_>>(),
        "presentations": presentations_hit(scan, r),
        "rules": r.rules.iter().map(|x| x.info().id).collect::<Vec<_>>(),
        "entry_unit": r.entry_unit,
        "entry_seconds": r.entry_unit.and_then(|u| seconds_at(scan, u)),
        "lost_bytes": r.lost_bytes,
    })
}

/// One damage region as text.
fn region_text(scan: &scan::Scan, r: &Region) -> String {
    let when = |u: u64| seconds_at(scan, u).map_or_else(|| format!("au {u}"), clock);
    let what = match r.kind {
        RegionKind::Lost => format!("{} bytes lost before au {}", r.lost_bytes, r.first_unit),
        RegionKind::Audio | RegionKind::Metadata => {
            let units = if r.first_unit == r.last_unit {
                format!("au {}", r.first_unit)
            } else {
                format!("au {}-{}", r.first_unit, r.last_unit)
            };
            format!(
                "{} {}..{} ({units})",
                r.kind.name(),
                when(r.first_unit),
                when(r.last_unit + 1)
            )
        }
    };
    let hit = presentations_hit(scan, r)
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" ");
    let entry = r.entry_unit.map_or_else(
        || "not clean again before the end".to_string(),
        |u| format!("clean again at au {u} ({})", when(u)),
    );
    let at = if r.kind == RegionKind::Lost {
        format!("{what} ({})", when(r.first_unit))
    } else {
        what
    };
    format!("{at}; presentations {hit}; {entry}")
}

/// The `Findings:` and `Rules:` blocks of the text report: every listed
/// finding, then one line per rule that fired, at most six of them, the
/// heaviest first, so the last lines of the report keep the summary and the
/// verdict.
#[allow(clippy::print_stdout, reason = "printing is what this one is for")]
fn print_diagnostics(scan: &scan::Scan, fail_on: Severity) {
    let d = &scan.diag.tally;
    if d.is_empty() {
        return;
    }
    let mut first = true;
    // findings of an info rule are counted below and listed only when the
    // threshold asks for them: a clean stream's report stays short
    for (rule, t) in d
        .iter()
        .filter(|(r, _)| fail_on == Severity::Info || r.info().severity > Severity::Info)
    {
        for f in &t.listed {
            println!(
                "{}{:<8} {:<26} {}  {}",
                if first {
                    "Findings:          "
                } else {
                    "                   "
                },
                rule.info().severity.name(),
                rule.info().id,
                location_text(scan, &f.at),
                f.message
            );
            first = false;
        }
        if t.suppressed() > 0 {
            println!(
                "                   {}: {} listed, {} more not listed (--max-per-rule), last at {}",
                rule.info().id,
                t.listed.len(),
                t.suppressed(),
                location_text(scan, &t.last)
            );
        }
    }
    let counts = Severity::ALL
        .iter()
        .rev()
        .map(|s| format!("{} {}", d.count_at(*s), s.name()))
        .collect::<Vec<_>>()
        .join(", ");
    println!("Rules:             {counts} (fail on {})", fail_on.name());
    let mut fired: Vec<_> = d.iter().collect();
    fired.sort_by_key(|(r, _)| core::cmp::Reverse(r.info().severity));
    for (rule, t) in fired.iter().take(6) {
        let span = if t.count > 1 {
            format!(
                "{} .. {}",
                location_text(scan, &t.first.at),
                location_text(scan, &t.last)
            )
        } else {
            location_text(scan, &t.first.at)
        };
        println!(
            "                   {:<8} {:<26} {:>6}x  {}",
            rule.info().severity.name(),
            rule.info().id,
            t.count,
            span
        );
    }
    if fired.len() > 6 {
        println!(
            "                   and {} more rules (verify --json)",
            fired.len() - 6
        );
    }
    let regions = &scan.diag.regions;
    let total = regions.len() as u64 + scan.diag.regions_overflow;
    for (k, r) in regions.iter().take(4).enumerate() {
        println!(
            "{}{}",
            if k == 0 {
                format!(
                    "Damage:            {total} region{}; ",
                    if total == 1 { "" } else { "s" }
                )
            } else {
                "                   ".to_string()
            },
            region_text(scan, r)
        );
    }
    if total > 4 {
        println!(
            "                   and {} more regions (verify --json)",
            total - 4
        );
    }
}

/// What `verify` found in a TrueHD stream: everything its report says, and
/// the verdict.
#[derive(Debug)]
pub struct Report {
    // on the heap: a scan is large, and a report is handed from frame to frame
    scan: Box<scan::Scan>,
    checks: Option<LosslessChecks>,
    discs: Option<[Row; 3]>,
    elapsed: f64,
    clean: bool,
    fail_on: Severity,
    require_disc: Vec<Disc>,
    census: bool,
}

/// Reads the stream and judges it, and prints nothing. With `decode`, every
/// presentation the stream carries is decoded as well and its lossless checks
/// join the verdict.
pub fn report(source: &Source, opts: &Options) -> Result<Report> {
    let started = Instant::now();
    let (mut scan, decoded) = if opts.decode {
        scan_beside_the_decodes(source, opts)?
    } else {
        let scan = scan::scan_keeping(source, opts.max_per_rule, opts.trace_fifo.is_some())?;
        (Box::new(scan), None)
    };
    if let (Some(out), Some(layouts)) = (&opts.trace_fifo, &scan.layouts) {
        write_trace(out, layouts)?;
    }
    if scan.units == 0 {
        // Nothing framed: the file is not a stream to judge but an unsupported
        // input, exit 2, as it is for every command that reports one.
        return Err(crate::info::no_stream(source));
    }
    let checks = match (decoded, opts.decode) {
        (Some(decoded), _) => Some(decoded?),
        // asked for, of a stream without a major sync to decode from
        (None, true) => Some(LosslessChecks::default()),
        (None, false) => None,
    };
    if let Some(c) = &checks {
        record_lossless(&mut scan.diag.tally, c);
    }
    let discs = disc_rows(&scan);
    record_required_discs(&mut scan.diag.tally, discs.as_ref(), &opts.require_disc);
    let elapsed = started.elapsed().as_secs_f64();
    // The verdict is the worst finding against the threshold. Every counter
    // the verdict used to read is a rule at `error` or above, so at the
    // default threshold the two readings are the same one.
    let clean = scan.first_major_sync.is_some() && scan.diag.tally.passes(opts.fail_on);
    debug_assert!(
        opts.fail_on != Severity::Error
            || clean
                == (scan.failures.is_clean()
                    && scan.first_major_sync.is_some()
                    && checks.as_ref().is_none_or(LosslessChecks::is_clean)
                    && !scan.diag.tally.iter().any(|(r, _)| {
                        r.info().legacy_counter.is_none()
                            && r.info().severity >= Severity::Error
                            && !matches!(
                                r,
                                RuleId::DecodeLosslessCheck
                                    | RuleId::DecodeStopped
                                    | RuleId::DecodeMaxBits
                                    | RuleId::DecodeClipped
                            )
                    })),
        "the rule verdict and the counter verdict disagree"
    );
    Ok(Report {
        scan,
        checks,
        discs,
        elapsed,
        clean,
        fail_on: opts.fail_on,
        require_disc: opts.require_disc.clone(),
        census: opts.census,
    })
}

impl Report {
    /// Whether the stream is clean.
    #[must_use]
    pub fn clean(&self) -> bool {
        self.clean
    }

    /// How fast the stream was read against its own duration.
    fn speed(&self) -> Option<f64> {
        self.scan
            .duration_seconds()
            .map(|d| d / self.elapsed.max(1e-9))
    }

    /// The report as `verify --json` prints it.
    ///
    /// # Errors
    /// When a part of the report does not serialise, which none does.
    pub fn json(&self) -> Result<Value> {
        let scan: &scan::Scan = &self.scan;
        let mut value = serde_json::to_value(scan)?;
        value["lossless_checks"] = serde_json::to_value(&self.checks)?;
        value["diagnostics"] = diagnostics_json(scan, self.fail_on);
        value["buffer"] = buffer_json(scan);
        value["disc"] = disc_json(self.discs.as_ref(), &self.require_disc);
        if self.census {
            value["census"] = crate::census::json(&scan.census, carried_substreams(scan));
        }
        value["clean"] = serde_json::json!(self.clean);
        value["seconds"] = serde_json::json!(self.elapsed);
        value["realtime_factor"] = serde_json::json!(self.speed());
        Ok(value)
    }

    /// Prints the report on stdout, as JSON or as text.
    ///
    /// # Errors
    /// As [`Report::json`].
    #[allow(clippy::print_stdout, reason = "printing is what this one is for")]
    pub fn print(&self, json: bool) -> Result<()> {
        if json {
            println!("{}", serde_json::to_string_pretty(&self.json()?)?);
            return Ok(());
        }
        self.print_text();
        Ok(())
    }

    #[allow(clippy::print_stdout, reason = "printing is what this one is for")]
    fn print_text(&self) {
        let scan: &scan::Scan = &self.scan;
        let (checks, discs, clean) = (&self.checks, &self.discs, self.clean);
        let (elapsed, speed) = (self.elapsed, self.speed());
        println!("File:              {}", scan.file);
        println!(
            "Access units:      {} ({} major syncs, {} bytes)",
            scan.units, scan.major_syncs, scan.total_unit_bytes
        );
        let f = &scan.failures;
        println!("Header parity:     {} failures", f.header_parity);
        println!(
            "Major sync CRC:    {} failures, {} bad signatures",
            f.major_sync_crc, f.major_sync_signature
        );
        println!(
            "Framing:           {} errors, {} configuration changes",
            f.framing_errors, f.config_changes
        );
        println!(
            "Extra data:        {} header parity failures, {} truncated, {} Evolution parity failures, {} non-zero padding, {} container errors",
            f.extra_header_parity,
            f.extra_truncated,
            f.extra_evolution_parity,
            f.extra_padding_nonzero,
            f.evolution_container_errors
        );
        if scan.extra.oamd_ok > 0 || f.oamd_errors > 0 {
            println!(
                "Object metadata:   {} payloads parsed, {} errors; {} non-unity gains {:?}, {} mutes, {} non-zero sizes",
                scan.extra.oamd_ok,
                f.oamd_errors,
                scan.extra.oamd_gains_db.values().sum::<u64>(),
                scan.extra.oamd_gains_db,
                scan.extra.oamd_muted_updates,
                scan.extra.oamd_sized_updates
            );
            crate::scan::print_stale_sizes(scan.extra.oamd_stale_sizes);
        }
        println!(
            "Sync:              {} resyncs, {} skipped bytes, {} trailing bytes",
            f.resyncs, f.skipped_bytes, f.trailing_bytes
        );
        let substreams = scan
            .config
            .as_ref()
            .map_or(0, |c| usize::from(c.substreams));
        for (i, st) in scan.substreams.iter().enumerate().take(substreams) {
            let words = st
                .sync_words
                .iter()
                .map(|(w, n)| format!("{w} x{n}"))
                .collect::<Vec<_>>()
                .join(", ");
            let first = st.first_restart.as_ref().map_or_else(String::new, |r| {
                format!(
                    ", channels {}..={} (matrix {}), error_protect {}",
                    r.min_chan, r.max_chan, r.max_matrix_chan, r.error_protect
                )
            });
            println!(
                "Substream {i}:       {} segments ({} skipped), {} restart headers [{words}]{first}",
                st.segments, st.skipped, st.restart_headers
            );
            println!(
                "                   {} blocks (max {} per segment, {} protected), {} matrixing ({} interpolated), up to {} matrices, {} CRC segments, {} terminated ({} with zero samples)",
                st.blocks,
                st.max_blocks_per_segment,
                st.protected_blocks,
                st.matrixing_blocks,
                st.interpolated_blocks,
                st.max_primitive_matrices,
                st.crc_segments,
                st.terminated_segments,
                st.zero_sample_segments
            );
        }
        println!(
            "Substream checks:  {} parse errors, {} block bit-count mismatches, {} parity failures, {} CRC failures, {} end-pointer mismatches, {} sample-count mismatches, {} restart flag mismatches, {} bad terminators, {} unexpected tails",
            f.substream_errors,
            f.block_data_bits,
            f.segment_parity,
            f.segment_crc,
            f.segment_end,
            f.sample_count,
            f.restart_flag,
            f.terminator_tail,
            f.unexpected_tail
        );
        println!(
            "Coding tools:      {}",
            if scan.coverage.is_empty() {
                "none beyond plain coding".to_string()
            } else {
                scan.coverage.join(", ")
            }
        );
        if let Some(c) = checks {
            println!(
                "Lossless checks:   {} evaluated, {} failed, {} skipped",
                c.evaluated, c.failed, c.skipped
            );
            for p in &c.per_presentation {
                if let Some(e) = &p.error {
                    println!(
                        "                   presentation {}: the decode stopped: {e}",
                        p.presentation
                    );
                } else if p.failed > 0
                    && let Some(first) = &p.first_problem
                {
                    println!(
                        "                   presentation {}: {} failed, first problem: {first}",
                        p.presentation, p.failed
                    );
                }
            }
        }
        let t = &scan.timing_stats;
        println!(
            "Timing:            {} input timing jumps, {} output timing jumps, {} valid seamless branches, {} invalid branches, {} duplicate candidates, {} peak rate changes",
            t.input_jumps,
            t.output_jumps,
            t.valid_branches,
            t.invalid_branches,
            t.duplicate_candidates,
            t.peak_rate_changes
        );
        for b in t.branches.iter().take(8) {
            println!(
                "                   access unit {}: {} advance {} -> {}{} (step {}, fifo {}, 75ms {}, rate {})",
                b.unit,
                if b.valid {
                    "seamless branch"
                } else {
                    "restart"
                },
                b.prev_advance,
                b.advance,
                if b.rate_change {
                    ", peak data rate changed"
                } else {
                    ""
                },
                b.advance_step,
                b.fifo_duration,
                b.within_75ms,
                b.data_rate
            );
        }
        print_buffer(scan);
        print_disc(discs.as_ref(), &self.require_disc);
        print_diagnostics(scan, self.fail_on);
        if let Some(err) = &scan.first_error {
            println!("First error:       {err}");
        }
        if self.census {
            for (i, line) in crate::census::lines(&scan.census, carried_substreams(scan))
                .iter()
                .enumerate()
            {
                let label = if i == 0 { "Census:" } else { "" };
                println!("{label:<19}{line}");
            }
        }
        match speed {
            Some(x) => println!("Speed:             {elapsed:.2} s ({x:.0}x realtime)"),
            None => println!("Speed:             {elapsed:.2} s"),
        }
        println!(
            "Result:            {}",
            if clean { "CLEAN" } else { "NON-CONFORMANT" }
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Source {
        Source::file(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("fixtures")
                .join(name),
        )
    }

    /// The same bytes, held in memory under the name of the file.
    fn in_memory(file: &Source) -> Source {
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut file.open().expect("opened"), &mut bytes).expect("read");
        Source::memory(file.name(), bytes)
    }

    /// One stream of each syntax the commands read.
    const STREAMS: [&str; 4] = [
        "truehd-20-44k.thd",
        "mlp-51-96k.mlp",
        "authored-scene.mlp",
        "authored-scene.ec3",
    ];

    fn said(check: &StreamCheck) -> Value {
        serde_json::to_value(check).expect("a value")
    }

    /// What the decode of one presentation of a stream says of its lossless
    /// checks, decoded by itself from the first unit to the last.
    fn alone(source: &Source, presentation: usize) -> Value {
        let mut session = Session::new(presentation, false, Order::Stream);
        let mut error = None;
        let mut clipped = 0u64;
        input::for_each_unit(source, |unit| {
            if error.is_some() {
                return Ok(());
            }
            match session.decode(&unit, &mut std::io::sink()) {
                Ok(Some(frame)) => {
                    for row in frame.pcm {
                        for &ch in frame.order {
                            clipped += u64::from(oadec_spatial::clamp_i24(row[ch]).1);
                        }
                    }
                }
                Ok(None) => {}
                Err(e) => error = Some(format!("{e:#}")),
            }
            Ok(())
        })
        .expect("read");
        let stats = session.stats();
        json!({
            "presentation": presentation,
            "evaluated": stats.map_or(0, |s| s.lossless_checks),
            "failed": stats.map_or(0, |s| s.lossless_mismatches),
            "skipped": stats.map_or(0, |s| s.lossless_checks_skipped),
            "first_problem": stats.and_then(|s| s.first_problem.clone()),
            "error": error,
            "max_bits_violations": stats.map_or(0, |s| s.max_bits_violations),
            "clipped": clipped,
        })
    }

    /// What `verify --decode` says of a presentation is what the decode of
    /// that presentation says by itself. The presentations of a stream are
    /// decoded side by side, each from the units all of them are given, and
    /// one that stops at a fault stops alone: the others go on to the end.
    #[test]
    fn the_presentations_side_by_side_say_what_each_says_alone() {
        let mut one_stopped_and_another_went_on = 0;
        let mut presentations = 0;
        // A stream as it is (place 0), and with a bit turned so many eighths
        // of the way through it, or (place 8) in one of its last units, which
        // are handed to the decodes after the last full hand-over. Of the
        // stream of four presentations, the bit at seven eighths stops two
        // and leaves two; the streams of one presentation stop at theirs.
        let streams: [(&str, &[usize]); 4] = [
            ("authored-scene.mlp", &[0, 4, 7, 8]),
            ("truehd-51-88k.thd", &[0, 2]),
            ("mlp-51-96k.mlp", &[0, 2]),
            ("truehd-20-44k.thd", &[3]),
        ];
        for (name, places) in streams {
            let mut whole = Vec::new();
            std::io::Read::read_to_end(&mut fixture(name).open().expect("opened"), &mut whole)
                .expect("read");
            for &n in places {
                let mut bytes = whole.clone();
                if n > 0 {
                    let at = match n {
                        8 => bytes.len() - 700,
                        _ => bytes.len() * n / 8 + 3,
                    };
                    bytes[at] ^= 0x10;
                }
                let source = Source::memory(name, bytes);
                let options = Options {
                    decode: true,
                    ..Options::default()
                };
                let report = report(&source, &options).expect("a report");
                let checks = report.checks.as_ref().expect("asked for");
                let carried: Vec<usize> = checks
                    .per_presentation
                    .iter()
                    .map(|p| p.presentation)
                    .collect();
                assert!(carried.is_sorted_by(|a, b| a < b), "{name}: {carried:?}");
                assert!(!carried.is_empty(), "{name}");
                for said in &checks.per_presentation {
                    assert_eq!(
                        serde_json::to_value(said).expect("a value"),
                        alone(&source, said.presentation),
                        "{name}, stream {n}, presentation {}",
                        said.presentation
                    );
                }
                let sum = |of: fn(&PresentationChecks) -> u64| -> u64 {
                    checks.per_presentation.iter().map(of).sum()
                };
                assert_eq!(checks.evaluated, sum(|p| p.evaluated), "{name}, stream {n}");
                assert_eq!(checks.failed, sum(|p| p.failed), "{name}, stream {n}");
                assert_eq!(checks.skipped, sum(|p| p.skipped), "{name}, stream {n}");
                let stopped = checks
                    .per_presentation
                    .iter()
                    .filter(|p| p.error.is_some())
                    .count();
                if stopped > 0 && stopped < carried.len() {
                    one_stopped_and_another_went_on += 1;
                }
                presentations += carried.len();
            }
        }
        assert_eq!(presentations, 23);
        assert_eq!(one_stopped_and_another_went_on, 1);
    }

    /// A stream read from memory is the stream read from its file: the checks
    /// say the same of both, and the report of a scan is the same text.
    #[test]
    fn a_stream_in_memory_is_checked_as_its_file_is() {
        for name in STREAMS {
            let file = fixture(name);
            let memory = in_memory(&file);
            let of_file = stream_check(&file, &|| true).expect("checked");
            let of_memory = stream_check(&memory, &|| true).expect("checked");
            assert_eq!(said(&of_file), said(&of_memory), "{name}");
            if name.ends_with(".ec3") {
                continue;
            }
            let report = |source: &Source| {
                serde_json::to_string(&scan::scan(source).expect("scanned")).expect("a report")
            };
            assert_eq!(report(&file), report(&memory), "{name}");
        }
    }

    /// A check that is told to stop gives the stream up at the first access
    /// unit or syncframe it comes to.
    #[test]
    fn a_check_told_to_stop_gives_the_stream_up_at_once() {
        for name in STREAMS {
            let asked = std::cell::Cell::new(0u32);
            let stopped = stream_check(&fixture(name), &|| {
                asked.set(asked.get() + 1);
                false
            });
            let error = stopped.expect_err("given up");
            assert_eq!(asked.get(), 1, "{name}");
            assert!(
                format!("{error:#}").contains("given up"),
                "{name}: {error:#}"
            );
        }
    }

    /// On a thread of its own the check gives the verdict it gives here.
    #[test]
    fn a_check_that_is_joined_gives_the_verdict_of_the_check() {
        for name in STREAMS {
            let source = fixture(name);
            let here = stream_check(&source, &|| true).expect("checked");
            let there = join_stream_check(spawn_stream_check(&source)).expect("checked");
            assert_eq!(said(&here), said(&there), "{name}");
        }
    }

    /// The thread reads the flag its handle holds: one started under a flag
    /// that is already set gives the stream up without a verdict.
    #[test]
    fn a_check_on_its_thread_stops_when_its_flag_is_set() {
        for name in STREAMS {
            let check = spawn_under(&fixture(name), Arc::new(AtomicBool::new(true)));
            let error = join_stream_check(check).expect_err("given up");
            assert!(
                format!("{error:#}").contains("given up"),
                "{name}: {error:#}"
            );
        }
    }

    /// The thread of a check does not outlive its handle: one that is dropped
    /// is told to stop and waited for.
    #[test]
    fn a_check_that_is_dropped_is_stopped_and_waited_for() {
        for name in STREAMS {
            let check = spawn_stream_check(&in_memory(&fixture(name)));
            let given_up = Arc::clone(&check.given_up);
            drop(check);
            assert!(given_up.load(Ordering::Relaxed), "{name}: told to stop");
            // the thread held the other handle on the flag, and it has ended
            assert_eq!(Arc::strong_count(&given_up), 1, "{name}");
        }
    }
}
