//! `oadec verify`: integrity counters over a whole stream.

use std::path::Path;
use std::time::Instant;

use anyhow::Result;
use oadec_truehd::damage::{Region, RegionKind};
use oadec_truehd::{
    Location, MAX_PRESENTATIONS, MajorSync, Provenance, RuleId, Severity, StreamConfig, Tally,
};
use serde::Serialize;
use serde_json::{Value, json};

use crate::decode::{Order, Session};
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

/// Decodes every presentation the stream carries and collects the lossless
/// check words the decoder evaluated.
///
/// The integrity pass parses every segment and checks its parity and CRC, but
/// it makes no samples, so the check word a restart header carries over the
/// decoded output of the section before it was never evaluated by `verify` --
/// only by a decode, and only for the one presentation that decode asked for.
/// A presentation that is a copy of another is not decoded twice: its samples,
/// and so its checks, are the other one's.
fn lossless_checks(path: &Path, ms: &MajorSync) -> Result<LosslessChecks> {
    let config = StreamConfig::from_major_sync(ms)?;
    let mut sessions: Vec<(usize, Session, Option<String>, u64)> = (0..MAX_PRESENTATIONS)
        .filter(|&p| config.presentations.is_carried(p))
        .map(|p| (p, Session::new(p, false, Order::Stream), None, 0))
        .collect();
    input::for_each_unit(path, |unit| {
        for (_, session, error, clipped) in &mut sessions {
            if error.is_some() {
                continue;
            }
            match session.decode(&unit) {
                // counted with the rule every writer clamps by, so the count
                // is the one `decode` reports for the same presentation
                Ok(Some(frame)) => {
                    for row in frame.pcm {
                        for &ch in frame.order {
                            *clipped += u64::from(oadec_spatial::clamp_i24(row[ch]).1);
                        }
                    }
                }
                Ok(None) => {}
                Err(e) => *error = Some(format!("{e:#}")),
            }
        }
        Ok(())
    })?;
    let mut checks = LosslessChecks::default();
    for (presentation, session, error, clipped) in sessions {
        let p = match session.stats() {
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
        };
        checks.evaluated += p.evaluated;
        checks.failed += p.failed;
        checks.skipped += p.skipped;
        checks.per_presentation.push(p);
    }
    Ok(checks)
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
/// codec.
pub fn stream_check(path: &Path) -> Result<StreamCheck> {
    if crate::eac3::is_eac3(path)? {
        return crate::eac3::stream_check(path);
    }
    let scan = scan::scan(path)?;
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

/// Starts [`stream_check`] on a thread of its own, for a command that reads the
/// same stream in a pass of its own meanwhile.
pub fn spawn_stream_check(path: &Path) -> std::thread::JoinHandle<Result<StreamCheck>> {
    let path = path.to_path_buf();
    std::thread::spawn(move || stream_check(&path))
}

/// The verdict of a check started with [`spawn_stream_check`].
pub fn join_stream_check(
    check: std::thread::JoinHandle<Result<StreamCheck>>,
) -> Result<StreamCheck> {
    check
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

/// How `verify` judges a TrueHD stream and how much of what it found it lists.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Machine-readable JSON instead of text.
    pub json: bool,
    /// Decode every presentation and evaluate the lossless check words.
    pub decode: bool,
    /// The lightest severity that fails the run.
    pub fail_on: Severity,
    /// Findings listed in full per rule (0 lists every one); counts are
    /// never limited.
    pub max_per_rule: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            json: false,
            decode: false,
            fail_on: Severity::Error,
            max_per_rule: Tally::DEFAULT_MAX_PER_RULE,
        }
    }
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
fn clock(seconds: f64) -> String {
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

/// Runs the command; returns `true` when the stream is clean. With `decode`,
/// every presentation the stream carries is decoded as well and its lossless
/// checks join the verdict.
pub fn run(path: &Path, opts: &Options) -> Result<bool> {
    let started = Instant::now();
    let mut scan = scan::scan_with(path, opts.max_per_rule)?;
    if scan.units == 0 {
        // Nothing framed: the file is not a stream to judge but an unsupported
        // input, exit 2, as it is for every command that reports one.
        return Err(crate::info::no_stream(path));
    }
    let checks = match (&scan.first_major_sync, opts.decode) {
        (Some(ms), true) => Some(lossless_checks(path, ms)?),
        (None, true) => Some(LosslessChecks::default()),
        (_, false) => None,
    };
    if let Some(c) = &checks {
        record_lossless(&mut scan.diag.tally, c);
    }
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
    let speed = scan.duration_seconds().map(|d| d / elapsed.max(1e-9));
    if opts.json {
        let mut value = serde_json::to_value(&scan)?;
        value["lossless_checks"] = serde_json::to_value(&checks)?;
        value["diagnostics"] = diagnostics_json(&scan, opts.fail_on);
        value["clean"] = serde_json::json!(clean);
        value["seconds"] = serde_json::json!(elapsed);
        value["realtime_factor"] = serde_json::json!(speed);
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
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
        if let Some(c) = &checks {
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
        print_diagnostics(&scan, opts.fail_on);
        if let Some(err) = &scan.first_error {
            println!("First error:       {err}");
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
    Ok(clean)
}
