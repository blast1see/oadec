//! One pass over a TrueHD stream collecting what `info` and `verify` report.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use oadec_emdf::container;
use oadec_truehd::{
    AccessUnit, ExtraKind, MajorSync, ParserState, SampleBuffer, Segment, StreamConfig,
    StreamTiming, TimingModel, Unit,
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
    /// Payload id -> occurrences.
    pub payload_ids: BTreeMap<u32, u64>,
    /// Payload id -> total bytes.
    pub payload_bytes: BTreeMap<u32, u64>,
    pub protected_frames: u64,
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
}

fn note_first(slot: &mut Option<String>, message: impl FnOnce() -> String) {
    if slot.is_none() {
        *slot = Some(message());
    }
}

impl Scan {
    fn new(path: &Path) -> Self {
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
        }
    }

    fn note_error(&mut self, unit_index: u64, offset: u64, what: &str) {
        self.failures.framing_errors += 1;
        note_first(&mut self.first_error, || {
            format!("access unit {unit_index} at byte {offset}: {what}")
        });
    }

    fn take_unit(&mut self, unit: &Unit, since_major_sync: &mut u64) {
        let index = self.units;
        self.units += 1;
        self.min_unit_bytes = self.min_unit_bytes.min(unit.bytes.len());
        self.max_unit_bytes = self.max_unit_bytes.max(unit.bytes.len());
        self.total_unit_bytes += unit.bytes.len() as u64;
        if self.first_unit_offset.is_none() {
            self.first_unit_offset = Some(unit.offset);
        }
        if unit.has_major_sync {
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
        if self.first_input_timing.is_none() {
            self.first_input_timing = Some(au.header.input_timing);
        }
        self.last_input_timing = Some(au.header.input_timing);
        if !au.header_parity_ok {
            self.failures.header_parity += 1;
        }
        if let Some(ms) = &au.major_sync {
            if !ms.crc_ok {
                self.failures.major_sync_crc += 1;
            }
            if ms.signature != oadec_truehd::sync::SIGNATURE {
                self.failures.major_sync_signature += 1;
            }
            match &self.first_major_sync {
                None => self.first_major_sync = Some(ms.clone()),
                Some(first) => {
                    if first.substream_info != ms.substream_info
                        || first.extended_substream_info != ms.extended_substream_info
                        || first.substreams != ms.substreams
                        || first.format_info != ms.format_info
                    {
                        self.failures.config_changes += 1;
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
            if !extra.header_parity_ok {
                self.failures.extra_header_parity += 1;
            }
            if !extra.padding_zero {
                self.failures.extra_padding_nonzero += 1;
            }
            match &extra.kind {
                ExtraKind::Padding => self.extra.padding_blocks += 1,
                ExtraKind::Opaque(_) => self.extra.opaque_blocks += 1,
                ExtraKind::Truncated => self.failures.extra_truncated += 1,
                ExtraKind::Evolution { frame, .. } => {
                    self.extra.evolution_blocks += 1;
                    if extra.parity_ok == Some(false) {
                        self.failures.extra_evolution_parity += 1;
                    }
                    if !frame.is_empty() {
                        self.extra.evolution_frames += 1;
                        match container::parse_evolution(frame) {
                            Ok(c) => {
                                if !c.protection.primary.is_empty() {
                                    self.extra.protected_frames += 1;
                                }
                                for p in &c.payloads {
                                    *self.extra.payload_ids.entry(p.id).or_default() += 1;
                                    *self.extra.payload_bytes.entry(p.id).or_default() +=
                                        p.data.len() as u64;
                                }
                            }
                            Err(e) => {
                                self.failures.evolution_container_errors += 1;
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
            while stats.branches.len() < model.branches.len() && stats.branches.len() < 64 {
                let b = model.branches[stats.branches.len()];
                stats.branches.push(BranchReport {
                    unit: b.unit,
                    input_jump: b.input_jump,
                    output_jump: b.output_jump,
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
        for i in 0..count {
            let entry = &au.directory[i];
            let Some(bytes) = unit.bytes.get(au.segment_range(i)) else {
                self.failures.framing_errors += 1;
                note_first(&mut self.first_error, || {
                    format!("access unit {index}: substream {i} range outside the unit")
                });
                continue;
            };
            if !parser.substream[i].restart_seen && entry.restart_nonexistent {
                self.substreams[i].skipped += 1;
                continue;
            }
            let seg = match Segment::parse(bytes, parser, i, entry.crc_present, &mut self.buffer) {
                Ok(seg) => seg,
                Err(e) => {
                    self.failures.substream_errors += 1;
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
                f.restart_flag += 1;
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
                    f.block_data_bits += 1;
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
                f.sample_count += 1;
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
                }
                if !t.tail_ok {
                    f.terminator_tail += 1;
                }
            }
            if seg.unexpected_tail {
                f.unexpected_tail += 1;
                note_first(&mut self.first_error, || {
                    format!(
                        "access unit {index}: substream {i}: unexpected data before the end pointer"
                    )
                });
            }
            if !seg.parity_ok {
                f.segment_parity += 1;
                note_first(&mut self.first_error, || {
                    format!("access unit {index}: substream {i}: parity mismatch")
                });
            }
            if !seg.crc_ok {
                f.segment_crc += 1;
                note_first(&mut self.first_error, || {
                    format!("access unit {index}: substream {i}: CRC mismatch")
                });
            }
            if !seg.end_ok {
                f.segment_end += 1;
                note_first(&mut self.first_error, || {
                    format!(
                        "access unit {index}: substream {i}: segment ended at bit {} of {}",
                        seg.len_bits,
                        bytes.len() * 8
                    )
                });
            }
        }
    }

    fn finish(&mut self, summary: PassSummary) {
        self.file_bytes = summary.file_bytes;
        self.failures.resyncs = summary.stats.resyncs;
        self.failures.skipped_bytes = summary.stats.skipped_bytes;
        self.failures.trailing_bytes = summary.trailing_bytes;
        self.failures.major_sync_crc += summary.stats.major_sync_crc_failures;
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

/// Scans a whole file.
pub fn scan(path: &Path) -> Result<Scan> {
    let mut scan = Scan::new(path);
    let mut since_major_sync = 0u64;
    let summary = input::for_each_unit(path, |unit| {
        scan.take_unit(&unit, &mut since_major_sync);
        Ok(())
    })?;
    scan.finish(summary);
    Ok(scan)
}
