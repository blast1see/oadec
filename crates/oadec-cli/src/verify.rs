//! `oadec verify`: integrity counters over a whole stream.

use std::path::Path;
use std::time::Instant;

use anyhow::Result;
use oadec_truehd::{MAX_PRESENTATIONS, MajorSync, StreamConfig};
use serde::Serialize;

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
        self.failed == 0 && self.per_presentation.iter().all(|p| p.error.is_none())
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
    let mut sessions: Vec<(usize, Session, Option<String>)> = (0..MAX_PRESENTATIONS)
        .filter(|&p| config.presentations.is_carried(p))
        .map(|p| (p, Session::new(p, false, Order::Stream), None))
        .collect();
    input::for_each_unit(path, |unit| {
        for (_, session, error) in &mut sessions {
            if error.is_none()
                && let Err(e) = session.decode(&unit)
            {
                *error = Some(format!("{e:#}"));
            }
        }
        Ok(())
    })?;
    let mut checks = LosslessChecks::default();
    for (presentation, session, error) in sessions {
        let p = match session.stats() {
            Some(s) => PresentationChecks {
                presentation,
                evaluated: s.lossless_checks,
                failed: s.lossless_mismatches,
                skipped: s.lossless_checks_skipped,
                first_problem: s.first_problem.clone(),
                error,
            },
            None => PresentationChecks {
                presentation,
                evaluated: 0,
                failed: 0,
                skipped: 0,
                first_problem: None,
                error,
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
    Ok(StreamCheck {
        clean: scan.failures.is_clean() && scan.first_major_sync.is_some(),
        first_problem: scan.first_error,
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

/// Runs the command; returns `true` when the stream is clean. With `decode`,
/// every presentation the stream carries is decoded as well and its lossless
/// checks join the verdict.
pub fn run(path: &Path, json: bool, decode: bool) -> Result<bool> {
    let started = Instant::now();
    let scan = scan::scan(path)?;
    if scan.units == 0 {
        // Nothing framed: the file is not a stream to judge but an unsupported
        // input, exit 2, as it is for every command that reports one.
        return Err(crate::info::no_stream(path));
    }
    let checks = match (&scan.first_major_sync, decode) {
        (Some(ms), true) => Some(lossless_checks(path, ms)?),
        (None, true) => Some(LosslessChecks::default()),
        (_, false) => None,
    };
    let elapsed = started.elapsed().as_secs_f64();
    let clean = scan.failures.is_clean()
        && scan.first_major_sync.is_some()
        && checks.as_ref().is_none_or(LosslessChecks::is_clean);
    let speed = scan.duration_seconds().map(|d| d / elapsed.max(1e-9));
    if json {
        let mut value = serde_json::to_value(&scan)?;
        value["lossless_checks"] = serde_json::to_value(&checks)?;
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
