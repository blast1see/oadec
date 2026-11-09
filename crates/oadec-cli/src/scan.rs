//! One pass over a TrueHD stream collecting what `info` and `verify` report.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use oadec_emdf::container;
use oadec_truehd::damage::{DamageTracker, Region, Scope, scope_of};
use oadec_truehd::sync::SYNC_FBB;
use oadec_truehd::{
    AccessUnit, ExtraKind, Gap, Location, MajorSync, ParserState, RuleId, SampleBuffer, Segment,
    StreamConfig, StreamTiming, Tally, TimingModel, Unit,
};
use serde::Serialize;

use crate::input::{self, PassSummary};

/// Integrity counters (all should be zero for a conformant stream).
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Failures {
    pub header_parity: u64,
    pub major_sync_crc: u64,
    pub major_sync_signature: u64,
    pub framing_errors: u64,
    pub extra_header_parity: u64,
    pub extra_truncated: u64,
    pub extra_evolution_parity: u64,
    pub extra_padding_nonzero: u64,
    pub evolution_container_errors: u64,
    /// Object Audio Metadata payloads that would not parse.
    ///
    /// The scanner used to count payload ids and bytes and never read one, so
    /// `verify` called a stream clean while `decode --format damf` exited 7 on
    /// the same file and told the reader that `verify` reported the same
    /// faults. Thirteen of 198 library titles carry a truncated element in
    /// their first access unit -- `truehdd` warns about the same one -- and the
    /// two commands disagreed about every one of them.
    pub oamd_errors: u64,
    pub resyncs: u64,
    pub skipped_bytes: u64,
    pub trailing_bytes: u64,
    pub config_changes: u64,
    /// Timing jumps that were not valid seamless branches.
    pub invalid_branches: u64,
    /// Segments that failed to parse (syntax errors, restart header CRC, ranges).
    pub substream_errors: u64,
    /// Blocks whose data bit count differed from `block_data_bits`.
    pub block_data_bits: u64,
    /// Segments whose parity byte did not match.
    pub segment_parity: u64,
    /// Segments whose CRC byte did not match.
    pub segment_crc: u64,
    /// Segments that did not end exactly at their end pointer.
    pub segment_end: u64,
    /// Segments whose blocks did not add up to one access unit of samples.
    pub sample_count: u64,
    /// Directory `restart_nonexistent` flags contradicting the segment.
    pub restart_flag: u64,
    /// Termination words whose last 13 bits were neither `0x1234` nor a zero-sample count.
    pub terminator_tail: u64,
    /// Segments with room for a termination word that held something else.
    pub unexpected_tail: u64,
}

impl Failures {
    /// Adds `magnitude` to the counter of `rule`, if the rule had one before
    /// rules had names (`RuleInfo::legacy_counter`).
    pub fn bump(&mut self, rule: RuleId, magnitude: u64) {
        let slot = match rule {
            RuleId::AuHeaderParity => &mut self.header_parity,
            RuleId::SyncCrc => &mut self.major_sync_crc,
            RuleId::SyncSignature => &mut self.major_sync_signature,
            RuleId::AuFraming => &mut self.framing_errors,
            RuleId::ExtraHeaderParity => &mut self.extra_header_parity,
            RuleId::ExtraTruncated => &mut self.extra_truncated,
            RuleId::ExtraParity => &mut self.extra_evolution_parity,
            RuleId::ExtraPadding => &mut self.extra_padding_nonzero,
            RuleId::EvoContainer => &mut self.evolution_container_errors,
            RuleId::EvoOamd => &mut self.oamd_errors,
            RuleId::SyncResync => &mut self.resyncs,
            RuleId::SyncSkippedBytes => &mut self.skipped_bytes,
            RuleId::SyncTrailingBytes => &mut self.trailing_bytes,
            RuleId::ConfigChanged => &mut self.config_changes,
            RuleId::TimingInvalidBranch => &mut self.invalid_branches,
            RuleId::SegmentSyntax => &mut self.substream_errors,
            RuleId::BlockDataBits => &mut self.block_data_bits,
            RuleId::SegmentParity => &mut self.segment_parity,
            RuleId::SegmentCrc => &mut self.segment_crc,
            RuleId::SegmentEndPointer => &mut self.segment_end,
            RuleId::SegmentSampleCount => &mut self.sample_count,
            RuleId::AuRestartFlag => &mut self.restart_flag,
            RuleId::SegmentTerminator => &mut self.terminator_tail,
            RuleId::SegmentUnexpectedTail => &mut self.unexpected_tail,
            _ => return,
        };
        *slot += magnitude;
    }

    /// Whether every counter is zero.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.header_parity == 0
            && self.major_sync_crc == 0
            && self.major_sync_signature == 0
            && self.framing_errors == 0
            && self.extra_header_parity == 0
            && self.extra_truncated == 0
            && self.extra_evolution_parity == 0
            && self.extra_padding_nonzero == 0
            && self.evolution_container_errors == 0
            && self.oamd_errors == 0
            && self.resyncs == 0
            && self.skipped_bytes == 0
            && self.trailing_bytes == 0
            && self.config_changes == 0
            && self.invalid_branches == 0
            && self.substream_errors == 0
            && self.block_data_bits == 0
            && self.segment_parity == 0
            && self.segment_crc == 0
            && self.segment_end == 0
            && self.sample_count == 0
            && self.restart_flag == 0
            && self.terminator_tail == 0
            && self.unexpected_tail == 0
    }
}

/// The line `verify` and `info` print for elements whose declared size is not theirs.
pub(crate) fn print_stale_sizes(count: u64) {
    if count > 0 {
        println!(
            "                   {count} elements declare a size that is not theirs; their content is complete and was read (docs/oamd.md)"
        );
    }
}

/// Timing model statistics.
#[derive(Debug, Clone, Default, Serialize)]
pub struct TimingStats {
    /// Access units whose input timing jumped.
    pub input_jumps: u64,
    /// Restart headers whose output timing was not the expected one.
    pub output_jumps: u64,
    /// Jumps judged to be valid seamless branches.
    pub valid_branches: u64,
    /// Jumps that were not valid branches (the stream restarts there).
    pub invalid_branches: u64,
    /// Restart headers repeating the previous output timing (duplicate candidates).
    pub duplicate_candidates: u64,
    /// Peak data rate changes at major syncs.
    pub peak_rate_changes: u64,
    /// The first jumps: access unit, advance before and after, conditions met.
    pub branches: Vec<BranchReport>,
}

/// One judged jump.
#[derive(Debug, Clone, Serialize)]
pub struct BranchReport {
    pub unit: u64,
    pub input_jump: bool,
    pub output_jump: bool,
    /// The major sync of that access unit declared a new peak data rate, which
    /// a stream may do at a branch and nowhere else.
    pub rate_change: bool,
    pub prev_advance: u32,
    pub advance: u32,
    pub valid: bool,
    pub advance_step: bool,
    pub fifo_duration: bool,
    pub within_75ms: bool,
    pub data_rate: bool,
}

/// Extra-data statistics.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ExtraStats {
    pub units_with_extra: u64,
    pub padding_blocks: u64,
    pub opaque_blocks: u64,
    pub evolution_blocks: u64,
    pub evolution_frames: u64,
    /// Object Audio Metadata payloads that parsed, and the two fields of them
    /// that no real stream has ever been seen to carry: gains other than unity
    /// by decibel value, mutes counted apart because an inactive object's
    /// default *is* a mute, and updates with a non-zero size.
    pub oamd_ok: u64,
    /// Elements whose declared size is not theirs: read all the same, because the
    /// content is complete, and said here because it is not what the size claims.
    pub oamd_stale_sizes: u64,
    pub oamd_gains_db: BTreeMap<String, u64>,
    pub oamd_muted_updates: u64,
    pub oamd_sized_updates: u64,
    /// Payload id -> occurrences.
    pub payload_ids: BTreeMap<u32, u64>,
    /// Payload id -> total bytes.
    pub payload_bytes: BTreeMap<u32, u64>,
    pub protected_frames: u64,
    /// The key identifiers and protection words the Evolution frames declare.
    pub protection: crate::protection::ProtectionStats,
}

/// What the first restart header of a substream declared.
#[derive(Debug, Clone, Serialize)]
pub struct RestartSummary {
    pub sync_word: String,
    pub min_chan: u8,
    pub max_chan: u8,
    pub max_matrix_chan: u8,
    pub error_protect: bool,
}

/// Per-substream statistics.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SubstreamStats {
    pub segments: u64,
    /// Segments skipped because no restart header had been seen yet (or since an error).
    pub skipped: u64,
    pub restart_headers: u64,
    /// Restart sync word -> occurrences.
    pub sync_words: BTreeMap<String, u64>,
    pub first_restart: Option<RestartSummary>,
    pub blocks: u64,
    pub max_blocks_per_segment: u64,
    pub protected_blocks: u64,
    pub matrixing_blocks: u64,
    pub interpolated_blocks: u64,
    pub max_primitive_matrices: u64,
    pub terminated_segments: u64,
    pub zero_sample_segments: u64,
    pub crc_segments: u64,
}

/// Everything learned in one pass.
#[derive(Debug, Clone, Serialize)]
pub struct Scan {
    pub file: String,
    pub file_bytes: u64,
    pub units: u64,
    pub major_syncs: u64,
    pub max_major_sync_interval: u64,
    pub min_unit_bytes: usize,
    pub max_unit_bytes: usize,
    pub total_unit_bytes: u64,
    pub first_input_timing: Option<u16>,
    pub last_input_timing: Option<u16>,
    pub first_unit_offset: Option<u64>,
    #[serde(skip)]
    pub first_major_sync: Option<MajorSync>,
    #[serde(skip)]
    pub config: Option<StreamConfig>,
    #[serde(skip)]
    parser: Option<ParserState>,
    #[serde(skip)]
    buffer: Box<SampleBuffer>,
    #[serde(skip)]
    timing: Option<TimingModel>,
    pub timing_stats: TimingStats,
    pub crc_present_units: [u64; 4],
    pub drc_updates: [u64; 4],
    pub substreams: [SubstreamStats; 4],
    pub extra: ExtraStats,
    pub failures: Failures,
    pub first_error: Option<String>,
    /// DVD-Audio (FBB) sync patterns met inside this TrueHD stream and passed
    /// over. Not a fault by itself: the damage that exposed one is counted
    /// where it was found.
    pub foreign_syncs: u64,
    /// Every finding of the pass, by rule, with where each rule fired, and
    /// the damage regions they add up to.
    #[serde(skip)]
    pub diag: Diagnostics,
    /// Branches of the timing model already turned into findings.
    #[serde(skip)]
    branches_judged: usize,
    /// The object programme the first major sync declares, for the metadata
    /// to be held to.
    #[serde(skip)]
    program_from_sync: Option<oadec_spatial::Program>,
}

/// The findings of a pass and the damage they describe.
#[derive(Debug, Clone, Default)]
pub struct Diagnostics {
    /// Every finding, by rule.
    pub tally: Tally,
    /// The stretches the findings made untrustworthy (after the pass).
    pub regions: Vec<Region>,
    /// Regions beyond the ones kept, counted only.
    pub regions_overflow: u64,
    damage: DamageTracker,
    /// Substreams of the stream, for faults that damage all of them.
    substreams: u8,
}

impl Diagnostics {
    fn new(max_per_rule: usize) -> Self {
        Self {
            tally: Tally::new(max_per_rule),
            ..Self::default()
        }
    }

    /// Records a finding, and the damage it does if it does any.
    fn add(
        &mut self,
        rule: RuleId,
        at: Location,
        magnitude: u64,
        message: impl FnOnce() -> String,
    ) {
        if let Some(unit) = at.unit {
            self.damage
                .fault(unit, scope_of(rule, &at, self.substreams), rule);
        }
        self.tally.add(rule, at, magnitude, message);
    }
}

/// Counts a fault under the counter its rule had before rules had names, and
/// records it as a finding: one call, so the two cannot drift apart.
fn fault(
    failures: &mut Failures,
    diag: &mut Diagnostics,
    rule: RuleId,
    at: Location,
    message: impl FnOnce() -> String,
) {
    failures.bump(rule, 1);
    diag.add(rule, at, 1, message);
}

/// Whether a major sync at access unit `index` is one of the two a bitstream
/// may open with (Dolby, section 2.5): the first two units of the stream, or
/// the unit a branch joins a clip at and the one after it.
fn opens_a_bitstream(index: u64, mut branch_units: impl Iterator<Item = u64>) -> bool {
    index <= 1 || branch_units.any(|b| b == index || b + 1 == index)
}

/// What a later major sync changes of the first one, if anything `verify`
/// counts as a configuration change.
fn config_change(first: &MajorSync, ms: &MajorSync) -> Option<String> {
    if first.substreams != ms.substreams {
        Some(format!(
            "substreams {} -> {}",
            first.substreams, ms.substreams
        ))
    } else if first.substream_info != ms.substream_info {
        Some(format!(
            "substream_info {:#04X} -> {:#04X}",
            first.substream_info, ms.substream_info
        ))
    } else if first.extended_substream_info != ms.extended_substream_info {
        Some(format!(
            "extended_substream_info {} -> {}",
            first.extended_substream_info, ms.extended_substream_info
        ))
    } else if first.format_info != ms.format_info {
        Some(format!(
            "format_info changed (rate code {} -> {})",
            first.format_info.sampling_frequency_code, ms.format_info.sampling_frequency_code
        ))
    } else {
        None
    }
}

fn note_first(slot: &mut Option<String>, message: impl FnOnce() -> String) {
    if slot.is_none() {
        *slot = Some(message());
    }
}

impl Scan {
    fn new(path: &Path, max_per_rule: usize) -> Self {
        Self {
            file: path.display().to_string(),
            file_bytes: 0,
            units: 0,
            major_syncs: 0,
            max_major_sync_interval: 0,
            min_unit_bytes: usize::MAX,
            max_unit_bytes: 0,
            total_unit_bytes: 0,
            first_input_timing: None,
            last_input_timing: None,
            first_unit_offset: None,
            first_major_sync: None,
            config: None,
            parser: None,
            buffer: Box::default(),
            timing: None,
            timing_stats: TimingStats::default(),
            crc_present_units: [0; 4],
            drc_updates: [0; 4],
            substreams: core::array::from_fn(|_| SubstreamStats::default()),
            extra: ExtraStats::default(),
            failures: Failures::default(),
            first_error: None,
            foreign_syncs: 0,
            diag: Diagnostics::new(max_per_rule),
            branches_judged: 0,
            program_from_sync: None,
        }
    }

    fn note_error(&mut self, unit_index: u64, offset: u64, what: &str) {
        fault(
            &mut self.failures,
            &mut self.diag,
            RuleId::AuFraming,
            Location::unit(unit_index, offset),
            || what.to_string(),
        );
        note_first(&mut self.first_error, || {
            format!("access unit {unit_index} at byte {offset}: {what}")
        });
    }

    /// What the extractor passed over before a unit, or at the end of the
    /// stream: counted by `finish` from the extractor's totals, so recorded
    /// here without a counter.
    fn take_gap(&mut self, gap: Gap, before_unit: Option<u64>) {
        // the unit sync was found again at, so a report can say when
        let at = Location {
            unit: before_unit,
            ..Location::byte(gap.offset)
        };
        let before = before_unit.map_or_else(
            || "at the end of the stream".to_string(),
            |u| format!("before access unit {u}"),
        );
        if let Some(unit) = before_unit {
            self.diag
                .damage
                .fault(unit, Scope::Lost(gap.bytes), RuleId::SyncSkippedBytes);
        }
        let tally = &mut self.diag.tally;
        if gap.bytes > 0 {
            tally.add(RuleId::SyncSkippedBytes, at, gap.bytes, || {
                format!("{} bytes skipped {before}", gap.bytes)
            });
        }
        if gap.major_sync_crc_failures > 0 {
            tally.add(RuleId::SyncCrc, at, gap.major_sync_crc_failures, || {
                format!(
                    "{} major syncs with a failed CRC-16 among the bytes skipped {before}",
                    gap.major_sync_crc_failures
                )
            });
        }
        if gap.resyncs > 0 {
            tally.add(RuleId::SyncResync, at, gap.resyncs, || {
                format!("sync lost here and found again {before}")
            });
        }
        if gap.foreign_syncs > 0 {
            tally.add(RuleId::SyncForeignPattern, at, gap.foreign_syncs, || {
                format!(
                    "{} DVD-Audio sync patterns among the bytes skipped {before}",
                    gap.foreign_syncs
                )
            });
        }
    }

    fn take_unit(&mut self, unit: &Unit, since_major_sync: &mut u64) {
        let index = self.units;
        self.units += 1;
        let at_unit = Location::unit(index, unit.offset);
        if let Some(gap) = unit.gap_before {
            self.take_gap(gap, Some(index));
        }
        if !unit.has_major_sync && unit.bytes.get(4..8) == Some(&SYNC_FBB.to_be_bytes()[..]) {
            self.diag
                .tally
                .add(RuleId::SyncForeignPattern, at_unit, 1, || {
                    "the directory of this minor-sync unit spells the DVD-Audio sync pattern"
                        .to_string()
                });
        }
        self.min_unit_bytes = self.min_unit_bytes.min(unit.bytes.len());
        self.max_unit_bytes = self.max_unit_bytes.max(unit.bytes.len());
        self.total_unit_bytes += unit.bytes.len() as u64;
        if self.first_unit_offset.is_none() {
            self.first_unit_offset = Some(unit.offset);
        }
        if !unit.has_major_sync && unit.bytes.get(4..6) == Some(&[0xF8, 0x72][..]) {
            self.diag
                .tally
                .add(RuleId::AuSyncEmulation, at_unit, 1, || {
                    "the directory of this minor-sync unit begins with 0xF872".to_string()
                });
        }
        // A major sync interval is judged where both ends were delivered: after
        // a gap the units between are not all here to count.
        let mut spacing = None;
        if unit.has_major_sync {
            let interval = *since_major_sync;
            if self.major_syncs > 0 && unit.gap_before.is_none() {
                if interval > 128 {
                    self.diag
                        .tally
                        .add(RuleId::SyncMajorSyncInterval, at_unit, 1, || {
                            format!(
                                "{interval} access unit{} since the last major sync",
                                if interval == 1 { "" } else { "s" }
                            )
                        });
                }
                if interval < 8 {
                    spacing = Some(interval);
                }
            }
            self.major_syncs += 1;
            self.max_major_sync_interval = self.max_major_sync_interval.max(*since_major_sync);
            *since_major_sync = 0;
        }
        *since_major_sync += 1;

        let (au, config) = match AccessUnit::parse(&unit.bytes, self.config.as_ref()) {
            Ok(x) => x,
            Err(e) => {
                self.note_error(index, unit.offset, &e.to_string());
                return;
            }
        };
        self.diag.substreams = config.substreams;
        if self.first_input_timing.is_none() {
            self.first_input_timing = Some(au.header.input_timing);
        }
        self.last_input_timing = Some(au.header.input_timing);
        if !au.header_parity_ok {
            fault(
                &mut self.failures,
                &mut self.diag,
                RuleId::AuHeaderParity,
                at_unit,
                || "the check nibble does not fold the header and directory to 0xF".to_string(),
            );
        }
        if let Some(ms) = &au.major_sync {
            if !ms.crc_ok {
                fault(
                    &mut self.failures,
                    &mut self.diag,
                    RuleId::SyncCrc,
                    at_unit,
                    || "major sync CRC-16 mismatch".to_string(),
                );
            }
            if ms.signature != oadec_truehd::sync::SIGNATURE {
                let signature = ms.signature;
                fault(
                    &mut self.failures,
                    &mut self.diag,
                    RuleId::SyncSignature,
                    at_unit,
                    || format!("signature {signature:#06X}, not 0xB752"),
                );
            }
            if ms.has_16ch_presentation()
                && self.program_from_sync.is_none()
                && let Ok(program) = crate::damf::program_from_major_sync(ms)
            {
                self.program_from_sync = Some(program);
            }
            match &self.first_major_sync {
                None => self.first_major_sync = Some(ms.clone()),
                Some(first) => {
                    if let Some(what) = config_change(first, ms) {
                        fault(
                            &mut self.failures,
                            &mut self.diag,
                            RuleId::ConfigChanged,
                            at_unit,
                            || what,
                        );
                    }
                }
            }
        }
        for (i, entry) in au.directory.iter().enumerate().take(4) {
            if entry.crc_present {
                self.crc_present_units[i] += 1;
            }
            if entry.extra_word {
                self.drc_updates[i] += 1;
            }
        }
        if let Some(extra) = &au.extra {
            self.extra.units_with_extra += 1;
            let at_extra = Location::unit(index, unit.offset + au.segments_end() as u64);
            if !extra.header_parity_ok {
                fault(
                    &mut self.failures,
                    &mut self.diag,
                    RuleId::ExtraHeaderParity,
                    at_extra,
                    || "the length check nibble does not fold to 0xF".to_string(),
                );
            }
            if !extra.padding_zero {
                fault(
                    &mut self.failures,
                    &mut self.diag,
                    RuleId::ExtraPadding,
                    at_extra,
                    || "padding that is not zero".to_string(),
                );
            }
            if extra.trailing_bytes > 0 {
                let n = extra.trailing_bytes;
                self.diag.add(RuleId::ExtraTrailingBytes, at_extra, 1, || {
                    format!("{n} bytes after the block, before the end of the unit")
                });
            }
            match &extra.kind {
                ExtraKind::Padding => self.extra.padding_blocks += 1,
                ExtraKind::Opaque(_) => self.extra.opaque_blocks += 1,
                ExtraKind::Truncated => {
                    let words = extra.length_words;
                    fault(
                        &mut self.failures,
                        &mut self.diag,
                        RuleId::ExtraTruncated,
                        at_extra,
                        || format!("{words} words declared; the access unit ends first"),
                    );
                }
                ExtraKind::Evolution { frame, .. } => {
                    self.extra.evolution_blocks += 1;
                    if extra.parity_ok == Some(false) {
                        fault(
                            &mut self.failures,
                            &mut self.diag,
                            RuleId::ExtraParity,
                            at_extra,
                            || "parity byte mismatch".to_string(),
                        );
                    }
                    if !frame.is_empty() {
                        self.extra.evolution_frames += 1;
                        match container::parse_evolution(frame) {
                            Ok(c) => {
                                if c.len_bits.div_ceil(8) != frame.len() {
                                    let (syntax, declared) = (c.len_bits.div_ceil(8), frame.len());
                                    self.diag.add(RuleId::EvoLengthMismatch, at_extra, 1, || {
                                        format!(
                                            "the frame declares {declared} bytes, its container takes {syntax}"
                                        )
                                    });
                                }
                                if !c.protection.primary.is_empty() {
                                    self.extra.protected_frames += 1;
                                }
                                self.extra.protection.take(&c);
                                if c.protection.primary_code == 0 {
                                    self.diag.add(
                                        RuleId::EvoProtectionReserved,
                                        at_extra,
                                        1,
                                        || "primary protection length coded 00".to_string(),
                                    );
                                }
                                for p in &c.payloads {
                                    *self.extra.payload_ids.entry(p.id).or_default() += 1;
                                    *self.extra.payload_bytes.entry(p.id).or_default() +=
                                        p.data.len() as u64;
                                    if p.id == oadec_emdf::container::PAYLOAD_ID_OAMD {
                                        match oadec_emdf::oamd::Oamd::parse(&p.data) {
                                            Ok(oamd) => {
                                                self.extra.oamd_ok += 1;
                                                if let Some(ps) = &self.program_from_sync {
                                                    let from_oamd =
                                                        oadec_spatial::Program::from_oamd(&oamd);
                                                    if from_oamd != *ps {
                                                        let said = format!(
                                                            "{} bed channels and {} objects in the metadata, {} and {} in the major sync",
                                                            from_oamd.bed_channels().len(),
                                                            oamd.program.dynamic_objects,
                                                            ps.bed_channels().len(),
                                                            ps.dynamic_objects
                                                        );
                                                        self.diag.add(
                                                            RuleId::EvoProgramMismatch,
                                                            at_extra,
                                                            1,
                                                            || said,
                                                        );
                                                    }
                                                }
                                                self.extra.oamd_stale_sizes += oamd
                                                    .elements
                                                    .iter()
                                                    .filter(|e| !e.size_ok)
                                                    .count()
                                                    as u64;
                                                let c = oamd.gain_and_size_counts();
                                                for (db, n) in &c.gains_db {
                                                    *self
                                                        .extra
                                                        .oamd_gains_db
                                                        .entry(db.to_string())
                                                        .or_default() += n;
                                                }
                                                self.extra.oamd_muted_updates += c.muted;
                                                self.extra.oamd_sized_updates += c.sized;
                                            }
                                            Err(e) => {
                                                fault(
                                                    &mut self.failures,
                                                    &mut self.diag,
                                                    RuleId::EvoOamd,
                                                    at_extra,
                                                    || e.to_string(),
                                                );
                                                note_first(&mut self.first_error, || {
                                                    format!("access unit {index}: OAMD: {e}")
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                fault(
                                    &mut self.failures,
                                    &mut self.diag,
                                    RuleId::EvoContainer,
                                    at_extra,
                                    || e.to_string(),
                                );
                                note_first(&mut self.first_error, || {
                                    format!("access unit {index}: evolution frame: {e}")
                                });
                            }
                        }
                    }
                }
            }
        }

        if au.major_sync.is_some() || self.parser.is_none() {
            match &mut self.parser {
                Some(parser) => parser.update(&config),
                None => self.parser = Some(ParserState::new(&config)),
            }
        }
        if let Some(ms) = &au.major_sync {
            let timing = StreamTiming::new(ms, &config);
            match &mut self.timing {
                Some(model) => model.update_config(timing),
                None => self.timing = Some(TimingModel::new(timing)),
            }
        }
        if let Some(model) = &mut self.timing {
            model.begin_unit(au.header.input_timing, u32::from(au.header.length_words));
        }
        self.take_segments(&au, unit, index);
        if let Some(model) = &mut self.timing {
            model.end_unit();
            let stats = &mut self.timing_stats;
            stats.input_jumps = model.input_jumps;
            stats.output_jumps = model.output_jumps;
            stats.peak_rate_changes = model.peak_rate_changes;
            stats.valid_branches = model.valid_branches() as u64;
            stats.invalid_branches = model.invalid_branches() as u64;
            self.failures.invalid_branches = stats.invalid_branches;
            for b in &model.branches[self.branches_judged..] {
                if !b.is_valid() {
                    let c = b.conditions;
                    let failed = [
                        (!c.advance_step, "advance step"),
                        (!c.fifo_duration, "FIFO duration"),
                        (!c.within_75ms, "75 ms"),
                        (!c.data_rate, "data rate"),
                    ]
                    .iter()
                    .filter(|(f, _)| *f)
                    .map(|(_, n)| *n)
                    .collect::<Vec<_>>()
                    .join(", ");
                    let at = if b.unit == index {
                        at_unit
                    } else {
                        Location {
                            unit: Some(b.unit),
                            ..Location::default()
                        }
                    };
                    // counted above from the model, as before rules had names
                    self.diag.tally.add(RuleId::TimingInvalidBranch, at, 1, || {
                        format!(
                            "advance {} -> {}; failed: {}",
                            b.prev_advance,
                            b.advance,
                            if failed.is_empty() { "none" } else { &failed }
                        )
                    });
                }
            }
            self.branches_judged = model.branches.len();
            if let Some(interval) = spacing {
                let branch_units = model.branches.iter().rev().take(2).map(|b| b.unit);
                if !opens_a_bitstream(index, branch_units) {
                    self.diag
                        .tally
                        .add(RuleId::SyncMajorSyncSpacing, at_unit, 1, || {
                            format!(
                                "{interval} access unit{} since the last major sync",
                                if interval == 1 { "" } else { "s" }
                            )
                        });
                }
            }
            while stats.branches.len() < model.branches.len() && stats.branches.len() < 64 {
                let b = model.branches[stats.branches.len()];
                stats.branches.push(BranchReport {
                    unit: b.unit,
                    input_jump: b.input_jump,
                    output_jump: b.output_jump,
                    rate_change: b.rate_change,
                    prev_advance: b.prev_advance,
                    advance: b.advance,
                    valid: b.is_valid(),
                    advance_step: b.conditions.advance_step,
                    fifo_duration: b.conditions.fifo_duration,
                    within_75ms: b.conditions.within_75ms,
                    data_rate: b.conditions.data_rate,
                });
                if !b.is_valid() {
                    note_first(&mut self.first_error, || {
                        format!(
                            "access unit {}: timing jump that is not a valid seamless branch (advance {} -> {})",
                            b.unit, b.prev_advance, b.advance
                        )
                    });
                }
            }
        }
        self.config = Some(config);
    }

    /// Parses every substream segment of one access unit and tallies the results.
    fn take_segments(&mut self, au: &AccessUnit, unit: &Unit, index: u64) {
        let Some(parser) = self.parser.as_mut() else {
            return;
        };
        let count = parser.substreams.min(au.directory.len()).min(4);
        let mut unit_output_timing: Option<u16> = None;
        let mut timing_disagrees = false;
        for i in 0..count {
            let entry = &au.directory[i];
            let range = au.segment_range(i);
            let at_seg = Location::unit(index, unit.offset + range.start as u64).in_substream(i);
            let Some(bytes) = unit.bytes.get(range.clone()) else {
                fault(
                    &mut self.failures,
                    &mut self.diag,
                    RuleId::AuFraming,
                    at_seg,
                    || "the substream's range lies outside the unit".to_string(),
                );
                note_first(&mut self.first_error, || {
                    format!("access unit {index}: substream {i} range outside the unit")
                });
                continue;
            };
            if !parser.substream[i].restart_seen && entry.restart_nonexistent {
                self.substreams[i].skipped += 1;
                continue;
            }
            let seg =
                match Segment::parse_located(bytes, parser, i, entry.crc_present, &mut self.buffer)
                {
                    Ok(seg) => seg,
                    Err((e, bit)) => {
                        fault(
                            &mut self.failures,
                            &mut self.diag,
                            RuleId::SegmentSyntax,
                            at_seg.at_bit(unit.offset + range.start as u64, bit),
                            || e.to_string(),
                        );
                        note_first(&mut self.first_error, || {
                            format!(
                                "access unit {index} at byte {}: substream {i}: {e}",
                                unit.offset
                            )
                        });
                        parser.substream[i].restart_seen = false;
                        continue;
                    }
                };
            if let (Some(model), Some(rh)) = (
                self.timing.as_mut(),
                seg.blocks.first().and_then(|b| b.restart.as_ref()),
            ) {
                let r = model.restart_header(i, rh.output_timing);
                if r.duplicate_timing {
                    self.timing_stats.duplicate_candidates += 1;
                }
            }
            let ss = &parser.substream[i];
            let st = &mut self.substreams[i];
            let f = &mut self.failures;
            let d = &mut self.diag;
            if let Some(rh) = seg.blocks.first().and_then(|b| b.restart.as_ref()) {
                match unit_output_timing {
                    Some(first) if first != rh.output_timing && !timing_disagrees => {
                        timing_disagrees = true;
                        let other = rh.output_timing;
                        d.add(RuleId::TimingOutputTimingDisagree, at_seg, 1, || {
                            format!("output_timing {other} here, {first} in an earlier substream")
                        });
                    }
                    None => unit_output_timing = Some(rh.output_timing),
                    _ => {}
                }
            }
            let block_limit = if ss.sync_word == 0x31EA { 4 } else { 2 };
            if seg.blocks.len() > block_limit {
                let (n, word) = (seg.blocks.len(), ss.sync_word);
                d.add(RuleId::SegmentBlockCount, at_seg, 1, || {
                    format!("{n} blocks; a {word:#06X} segment carries at most {block_limit}")
                });
            }
            st.segments += 1;
            st.blocks += seg.blocks.len() as u64;
            st.max_blocks_per_segment = st.max_blocks_per_segment.max(seg.blocks.len() as u64);
            st.max_primitive_matrices = st.max_primitive_matrices.max(ss.primitive_matrices as u64);
            if entry.crc_present {
                st.crc_segments += 1;
            }
            if seg.has_restart {
                st.restart_headers += 1;
                *st.sync_words
                    .entry(format!("{:#06X}", ss.sync_word))
                    .or_default() += 1;
                if st.first_restart.is_none() {
                    st.first_restart = Some(RestartSummary {
                        sync_word: format!("{:#06X}", ss.sync_word),
                        min_chan: ss.min_chan as u8,
                        max_chan: ss.max_chan as u8,
                        max_matrix_chan: ss.max_matrix_chan as u8,
                        error_protect: ss.error_protect,
                    });
                }
            }
            if entry.restart_nonexistent == seg.has_restart {
                let has = seg.has_restart;
                fault(f, d, RuleId::AuRestartFlag, at_seg, || {
                    format!(
                        "restart_nonexistent says {}, the segment {} a restart header",
                        if has { "none" } else { "one" },
                        if has { "has" } else { "has no" }
                    )
                });
                note_first(&mut self.first_error, || {
                    format!(
                        "access unit {index}: substream {i}: restart_nonexistent flag contradicts the segment"
                    )
                });
            }
            for b in &seg.blocks {
                if b.block_data_bits.is_some() {
                    st.protected_blocks += 1;
                }
                if !b.block_data_bits_ok {
                    fault(f, d, RuleId::BlockDataBits, at_seg, || {
                        "block data bit count mismatch".to_string()
                    });
                    note_first(&mut self.first_error, || {
                        format!("access unit {index}: substream {i}: block data bit count mismatch")
                    });
                }
                if let Some(m) = b.header.and_then(|h| h.matrixing) {
                    st.matrixing_blocks += 1;
                    if m.interpolation_used {
                        st.interpolated_blocks += 1;
                    }
                }
            }
            if !seg.sample_count_ok {
                let samples = seg.samples;
                fault(f, d, RuleId::SegmentSampleCount, at_seg, || {
                    format!("{samples} samples in the segment")
                });
                note_first(&mut self.first_error, || {
                    format!(
                        "access unit {index}: substream {i}: {} samples in the segment",
                        seg.samples
                    )
                });
            }
            if let Some(t) = seg.terminator {
                st.terminated_segments += 1;
                if t.zero_samples_indicated {
                    st.zero_sample_segments += 1;
                    let spa = parser.samples_per_au;
                    if usize::from(t.zero_samples) >= spa {
                        let n = t.zero_samples;
                        d.add(RuleId::SegmentZeroSamples, at_seg, 1, || {
                            format!("{n} added samples in a unit of {spa}")
                        });
                    }
                }
                if !t.tail_ok {
                    fault(f, d, RuleId::SegmentTerminator, at_seg, || {
                        "the termination word ends in neither 0x1234 nor a zero-sample count"
                            .to_string()
                    });
                }
            }
            if seg.unexpected_tail {
                fault(f, d, RuleId::SegmentUnexpectedTail, at_seg, || {
                    "data where a termination word would fit".to_string()
                });
                note_first(&mut self.first_error, || {
                    format!(
                        "access unit {index}: substream {i}: unexpected data before the end pointer"
                    )
                });
            }
            if !seg.parity_ok {
                fault(f, d, RuleId::SegmentParity, at_seg, || {
                    "parity mismatch".to_string()
                });
                note_first(&mut self.first_error, || {
                    format!("access unit {index}: substream {i}: parity mismatch")
                });
            }
            if !seg.crc_ok {
                fault(f, d, RuleId::SegmentCrc, at_seg, || {
                    "CRC mismatch".to_string()
                });
                note_first(&mut self.first_error, || {
                    format!("access unit {index}: substream {i}: CRC mismatch")
                });
            }
            if !seg.end_ok {
                let (len, total) = (seg.len_bits, bytes.len() * 8);
                fault(f, d, RuleId::SegmentEndPointer, at_seg, || {
                    format!("segment ended at bit {len} of {total}")
                });
                note_first(&mut self.first_error, || {
                    format!(
                        "access unit {index}: substream {i}: segment ended at bit {} of {}",
                        seg.len_bits,
                        bytes.len() * 8
                    )
                });
            }
            let clean_segment = seg.parity_ok
                && seg.crc_ok
                && seg.end_ok
                && seg.sample_count_ok
                && !seg.unexpected_tail
                && seg.terminator.is_none_or(|t| t.tail_ok)
                && seg.blocks.iter().all(|b| b.block_data_bits_ok)
                && entry.restart_nonexistent != seg.has_restart;
            if seg.has_restart && clean_segment {
                self.diag.damage.restart(index, i);
            }
        }
    }

    fn finish(&mut self, summary: PassSummary) {
        if let Some(gap) = summary.final_gap {
            self.take_gap(gap, None);
        }
        if summary.trailing_bytes > 0 {
            let n = summary.trailing_bytes;
            self.diag.tally.add(
                RuleId::SyncTrailingBytes,
                Location::byte(summary.file_bytes.saturating_sub(n)),
                n,
                || format!("{n} bytes after the last access unit do not make one"),
            );
        }
        let damage = std::mem::take(&mut self.diag.damage);
        let (regions, overflow) = damage.finish(self.units.saturating_sub(1));
        self.diag.regions = regions;
        self.diag.regions_overflow = overflow;
        self.file_bytes = summary.file_bytes;
        self.failures.resyncs = summary.stats.resyncs;
        self.failures.skipped_bytes = summary.stats.skipped_bytes;
        self.failures.trailing_bytes = summary.trailing_bytes;
        self.failures.major_sync_crc += summary.stats.major_sync_crc_failures;
        self.foreign_syncs = summary.stats.foreign_syncs;
        if self.min_unit_bytes == usize::MAX {
            self.min_unit_bytes = 0;
        }
    }

    /// Duration in seconds implied by the access-unit count.
    #[must_use]
    pub fn duration_seconds(&self) -> Option<f64> {
        let cfg = self.config.as_ref()?;
        Some(self.units as f64 * f64::from(cfg.samples_per_au) / f64::from(cfg.sampling_frequency))
    }

    /// Average bit rate in bits per second.
    #[must_use]
    pub fn average_bit_rate(&self) -> Option<f64> {
        let secs = self.duration_seconds()?;
        (secs > 0.0).then(|| self.total_unit_bytes as f64 * 8.0 / secs)
    }
}

/// Scans a whole file, listing up to [`Tally::DEFAULT_MAX_PER_RULE`] findings
/// of each rule in full.
pub fn scan(path: &Path) -> Result<Scan> {
    scan_with(path, Tally::DEFAULT_MAX_PER_RULE)
}

/// Scans a whole file, listing up to `max_per_rule` findings of each rule in
/// full (0 lists all). The limit never changes a count or a verdict.
pub fn scan_with(path: &Path, max_per_rule: usize) -> Result<Scan> {
    let mut scan = Scan::new(path, max_per_rule);
    let mut since_major_sync = 0u64;
    let summary = input::for_each_unit(path, |unit| {
        scan.take_unit(&unit, &mut since_major_sync);
        Ok(())
    })?;
    scan.finish(summary);
    Ok(scan)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bitstream may open with two major syncs in a row: at the start of the
    /// stream and at every clip a branch joins, as every branch of a full
    /// title on disc does; nowhere else.
    #[test]
    fn two_major_syncs_in_a_row_open_a_bitstream_and_nothing_else() {
        let none = || std::iter::empty();
        assert!(opens_a_bitstream(0, none()));
        assert!(opens_a_bitstream(1, none()));
        assert!(!opens_a_bitstream(2, none()));
        assert!(opens_a_bitstream(54_205, [54_205].into_iter()));
        assert!(opens_a_bitstream(54_206, [54_205].into_iter()));
        assert!(!opens_a_bitstream(54_207, [54_205].into_iter()));
        assert!(!opens_a_bitstream(54_204, [54_205].into_iter()));
    }

    /// A rule bumps the counter its catalog entry names, by the magnitude, and
    /// no other; a rule without a counter bumps none.
    #[test]
    fn every_rule_bumps_the_counter_the_catalog_names() {
        for rule in RuleId::ALL {
            let mut f = Failures::default();
            f.bump(*rule, 3);
            let value = serde_json::to_value(f).unwrap();
            let fields = value.as_object().unwrap();
            let total: u64 = fields.values().map(|v| v.as_u64().unwrap()).sum();
            match rule.info().legacy_counter {
                Some(name) => {
                    assert_eq!(fields[name], 3, "{} -> {name}", rule.info().id);
                    assert_eq!(total, 3, "{} bumped more than its counter", rule.info().id);
                }
                None => assert_eq!(total, 0, "{} has no counter", rule.info().id),
            }
        }
    }

    /// Every counter the TrueHD verdict reads, one at a time.
    ///
    /// This is the twin of `is_clean` on the E-AC-3 side, which a mutation pass
    /// short-circuited to `true` with nothing failing because the file had no
    /// test module. Neither did this one, and it decides the exit code of
    /// `verify` for every TrueHD stream. The list is the whole content, so the
    /// list is what is pinned -- and the count is asserted too, so a counter
    /// added to the struct and forgotten in the verdict fails here rather than
    /// passing quietly.
    #[test]
    fn every_counter_in_the_truehd_verdict_can_make_a_stream_unclean() {
        assert!(Failures::default().is_clean(), "an empty pass is clean");

        type Set = fn(&mut Failures);
        let counters: [(&str, Set); 24] = [
            ("header_parity", |s| s.header_parity = 1),
            ("major_sync_crc", |s| s.major_sync_crc = 1),
            ("major_sync_signature", |s| s.major_sync_signature = 1),
            ("framing_errors", |s| s.framing_errors = 1),
            ("extra_header_parity", |s| s.extra_header_parity = 1),
            ("extra_truncated", |s| s.extra_truncated = 1),
            ("extra_evolution_parity", |s| s.extra_evolution_parity = 1),
            ("extra_padding_nonzero", |s| s.extra_padding_nonzero = 1),
            ("evolution_container_errors", |s| {
                s.evolution_container_errors = 1
            }),
            ("oamd_errors", |s| s.oamd_errors = 1),
            ("resyncs", |s| s.resyncs = 1),
            ("skipped_bytes", |s| s.skipped_bytes = 1),
            ("trailing_bytes", |s| s.trailing_bytes = 1),
            ("config_changes", |s| s.config_changes = 1),
            ("invalid_branches", |s| s.invalid_branches = 1),
            ("substream_errors", |s| s.substream_errors = 1),
            ("block_data_bits", |s| s.block_data_bits = 1),
            ("segment_parity", |s| s.segment_parity = 1),
            ("segment_crc", |s| s.segment_crc = 1),
            ("segment_end", |s| s.segment_end = 1),
            ("sample_count", |s| s.sample_count = 1),
            ("restart_flag", |s| s.restart_flag = 1),
            ("terminator_tail", |s| s.terminator_tail = 1),
            ("unexpected_tail", |s| s.unexpected_tail = 1),
        ];
        for (name, set) in counters {
            let mut f = Failures::default();
            set(&mut f);
            assert!(!f.is_clean(), "{name} left the stream looking clean");
        }

        // and every `u64` of the struct is in that list, so a counter added
        // later cannot sit outside the verdict unnoticed: the serialized
        // struct has exactly the listed keys, each of them set by its entry
        let mut all = Failures::default();
        for (_, set) in counters {
            set(&mut all);
        }
        assert!(!all.is_clean());
        let value = serde_json::to_value(all).unwrap();
        let keys = value.as_object().unwrap();
        assert_eq!(
            keys.len(),
            counters.len(),
            "a counter is missing from the list"
        );
        for (name, _) in counters {
            assert_eq!(keys[name], 1, "{name} is not the field its entry sets");
        }
    }
}
