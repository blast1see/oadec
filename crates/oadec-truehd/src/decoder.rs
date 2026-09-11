//! Sample reconstruction for one presentation: recorrelation, rematrixing,
//! remapping and the lossless check.
//!
//! The decoder drives the segment parser block by block. After every block the
//! coded samples of the substream are recorrelated (FIR/IIR prediction); when the
//! substream is the top of the requested presentation the union of the
//! recorrelated channels of every substream in the presentation is rematrixed
//! with that substream's matrices and remapped to output channels.
//!
//! All arithmetic is integer: 64-bit accumulators, 18-bit fixed-point matrix
//! coefficients, and the wrap-around and masking behaviour that the format
//! prescribes (checked bit-exactly against the other public decoders on real
//! streams, see `docs/evidence`).

use crate::au::{AccessUnit, DirectoryEntry, StreamConfig};
use crate::block::{Block, SampleBuffer};
use crate::dither::{fill_table_bc, noise_pair_a};
use crate::error::{Error, Result};
use crate::matrix::Matrixing;
use crate::presentation::PresentationKind;
use crate::segment::Segment;
use crate::state::{
    MAX_CHANNELS, MAX_MATRICES, MAX_SAMPLES_PER_AU, ParserState, SYNC_A, SYNC_B, SYNC_C,
    SubstreamState,
};
use crate::sync::MajorSync;
use crate::timing::{StreamTiming, TimingModel};

/// Taps kept for each prediction filter.
pub const FILTER_ORDER: usize = 8;

/// Columns of the rematrix buffer: the matrix channels plus the two noise
/// channels of sync word A.
const REMATRIX_COLUMNS: usize = MAX_CHANNELS + 2;

/// Fractional bits of the scaled matrix coefficients.
const MATRIX_FRAC: u32 = 18;

/// Counters kept while decoding.
#[derive(Debug, Clone, Default)]
pub struct DecodeStats {
    /// Access units decoded.
    pub units: u64,
    /// Samples (per channel) emitted.
    pub samples: u64,
    /// Lossless checks performed (one per restart header after the first).
    pub lossless_checks: u64,
    /// Lossless checks that failed.
    pub lossless_mismatches: u64,
    /// Access units whose outputs used more bits than `max_bits` allows.
    pub max_bits_violations: u64,
    /// Segments with a parity, CRC, end-pointer or sample-count problem.
    pub segment_problems: u64,
    /// Access units whose input timing jumped.
    pub input_jumps: u64,
    /// Timing jumps judged valid seamless branches.
    pub valid_branches: u64,
    /// Timing jumps that were not valid branches.
    pub invalid_branches: u64,
    /// Lossless checks not performed because the section spans a branch.
    pub lossless_checks_skipped: u64,
    /// Access units dropped as duplicates.
    pub duplicates: u64,
    /// Description of the first problem seen.
    pub first_problem: Option<String>,
}

impl DecodeStats {
    fn note(&mut self, message: impl FnOnce() -> String) {
        if self.first_problem.is_none() {
            self.first_problem = Some(message());
        }
    }
}

/// Decoded samples of one access unit.
#[derive(Debug, Clone, Copy)]
pub struct Decoded<'a> {
    /// Output rows (`[sample][channel]`), already remapped; only the first
    /// `channels` entries of each row are meaningful.
    pub pcm: &'a [[i32; MAX_CHANNELS]],
    /// Output channels of the presentation.
    pub channels: usize,
    /// Whether the unit was flagged as a duplicate to be dropped (see [`Decoder::keep_duplicates`]).
    pub duplicate: bool,
}

/// Decoder state of one substream.
#[derive(Debug, Clone)]
struct SubstreamDecoder {
    /// A restart header has been applied.
    active: bool,
    sync_word: u16,
    min_chan: usize,
    max_chan: usize,
    max_matrix_chan: usize,
    ch_assign: [u8; MAX_CHANNELS],
    dither_shift: u8,
    dither_seed: u32,
    max_bits: u8,
    output_timing: u16,
    /// Dither table of sync words B and C (regenerated per access unit).
    dither_table: [i32; 256],
    /// FIR history per channel, most recent output first.
    fir_hist: [[i32; FILTER_ORDER]; MAX_CHANNELS],
    /// IIR history per channel, most recent input first.
    iir_hist: [[i32; FILTER_ORDER]; MAX_CHANNELS],
    /// Recorrelated channels `min_chan..=max_chan` of the current access unit.
    recorrelated: Box<[[i32; MAX_CHANNELS]; MAX_SAMPLES_PER_AU]>,
    /// Scaled matrix coefficients (18 fractional bits), `[matrix][column]`.
    m: [[i32; REMATRIX_COLUMNS]; MAX_MATRICES],
    /// Scaled delta coefficients of sync word C.
    d: [[i32; MAX_CHANNELS]; MAX_MATRICES],
    /// Lossless check word of the current access unit.
    lossless_au: u32,
    /// Lossless check word accumulated since the restart header.
    lossless_accum: u32,
    /// Lossless check word of the previous access unit.
    lossless_prev: u32,
    /// Sign-folded OR of every output of the access unit.
    output_bits: u32,
    /// Samples of the access unit decoded so far.
    decoded: usize,
    /// Trailing silent samples declared by the termination word.
    zero_samples: usize,
}

impl Default for SubstreamDecoder {
    fn default() -> Self {
        Self {
            active: false,
            sync_word: 0,
            min_chan: 0,
            max_chan: 0,
            max_matrix_chan: 0,
            ch_assign: core::array::from_fn(|i| i as u8),
            dither_shift: 0,
            dither_seed: 0,
            max_bits: 24,
            output_timing: 0,
            dither_table: [0; 256],
            fir_hist: [[0; FILTER_ORDER]; MAX_CHANNELS],
            iir_hist: [[0; FILTER_ORDER]; MAX_CHANNELS],
            recorrelated: Box::new([[0; MAX_CHANNELS]; MAX_SAMPLES_PER_AU]),
            m: [[0; REMATRIX_COLUMNS]; MAX_MATRICES],
            d: [[0; MAX_CHANNELS]; MAX_MATRICES],
            lossless_au: 0,
            lossless_accum: 0,
            lossless_prev: 0,
            output_bits: 0,
            decoded: 0,
            zero_samples: 0,
        }
    }
}

impl SubstreamDecoder {
    /// Re-initialises the state from a restart header (already applied to `ss`).
    fn restart(&mut self, ss: &SubstreamState, output_timing: u16) {
        self.active = true;
        self.sync_word = ss.sync_word;
        self.min_chan = ss.min_chan;
        self.max_chan = ss.max_chan;
        self.max_matrix_chan = ss.max_matrix_chan;
        self.ch_assign = ss.ch_assign;
        self.dither_shift = ss.dither_shift;
        self.dither_seed = ss.dither_seed;
        self.max_bits = ss.max_bits;
        self.output_timing = output_timing;
        self.dither_table = [0; 256];
        self.fir_hist = [[0; FILTER_ORDER]; MAX_CHANNELS];
        self.iir_hist = [[0; FILTER_ORDER]; MAX_CHANNELS];
        self.m = [[0; REMATRIX_COLUMNS]; MAX_MATRICES];
        self.d = [[0; MAX_CHANNELS]; MAX_MATRICES];
        self.lossless_au = 0;
        self.lossless_accum = 0;
        self.output_bits = 0;
        // `recorrelated`, `decoded`, `zero_samples` and `lossless_prev` describe
        // the access unit in progress and the previous one; they stay.
    }

    /// Loads the scaled matrix and delta coefficients after a `matrixing()` block.
    fn update_matrices(&mut self, ss: &SubstreamState, m: Matrixing) {
        let mmc = ss.max_matrix_chan;
        match ss.sync_word {
            SYNC_A | SYNC_B => {
                let extra = if ss.sync_word == SYNC_A { 2 } else { 0 };
                for pmi in 0..ss.primitive_matrices {
                    let shift = MATRIX_FRAC - u32::from(ss.frac_bits[pmi]);
                    for ch in 0..=mmc + extra {
                        self.m[pmi][ch] = ss.matrix_coeff[pmi][ch] << shift;
                    }
                }
            }
            _ => {
                if m.new_matrix {
                    for pmi in 0..ss.primitive_matrices {
                        let shift = MATRIX_FRAC as i32 + i32::from(ss.cf_shift_code[pmi])
                            - i32::from(ss.frac_bits[pmi]);
                        for ch in 0..=mmc {
                            self.m[pmi][ch] = shift_left(ss.matrix_coeff[pmi][ch], shift);
                        }
                    }
                }
                if m.interpolation_used {
                    if m.new_delta {
                        for pmi in 0..ss.primitive_matrices {
                            let shift = MATRIX_FRAC as i32 + i32::from(ss.cf_shift_code[pmi])
                                - i32::from(ss.frac_bits[pmi])
                                - i32::from(ss.delta_precision[pmi]);
                            for ch in 0..=mmc {
                                self.d[pmi][ch] = shift_left(ss.delta_cf[pmi][ch], shift);
                            }
                        }
                    }
                } else {
                    self.d = [[0; MAX_CHANNELS]; MAX_MATRICES];
                }
            }
        }
    }

    /// Recorrelates channel `ch` for samples `n0..n1` of `buf`.
    #[allow(
        clippy::needless_range_loop,
        reason = "filter taps and histories share one index"
    )]
    fn recorrelate(
        &mut self,
        ss: &SubstreamState,
        ch: usize,
        n0: usize,
        n1: usize,
        buf: &SampleBuffer,
    ) -> Result<()> {
        let fir = &ss.fir[ch];
        let iir = &ss.iir[ch];
        let fir_order = usize::from(fir.order);
        let iir_order = usize::from(iir.order);
        let shift = if fir_order != 0 {
            fir.coeff_q
        } else {
            iir.coeff_q
        };
        let mask = !((1i64 << ss.quant_step_size[ch]) - 1);
        let limit: i64 = if self.sync_word == SYNC_C {
            1 << 31
        } else {
            1 << 23
        };
        let fir_hist = &mut self.fir_hist[ch];
        let iir_hist = &mut self.iir_hist[ch];
        for n in n0..n1 {
            let residual = i64::from(buf.samples[n][ch]);
            let mut acc = 0i64;
            for k in 0..fir_order {
                acc += i64::from(fir.coeff[k]) * i64::from(fir_hist[k]);
            }
            for k in 0..iir_order {
                acc += i64::from(iir.coeff[k]) * i64::from(iir_hist[k]);
            }
            let pred = acc >> shift;
            let out = residual + (pred & mask);
            let iir_in = out - pred;
            if out < -limit || out >= limit {
                return Err(Error::malformed(format!(
                    "channel {ch} sample {n}: recorrelated value {out} out of range"
                )));
            }
            if iir_in < -limit || iir_in >= limit {
                return Err(Error::malformed(format!(
                    "channel {ch} sample {n}: IIR input {iir_in} out of range"
                )));
            }
            fir_hist.copy_within(0..FILTER_ORDER - 1, 1);
            fir_hist[0] = out as i32;
            iir_hist.copy_within(0..FILTER_ORDER - 1, 1);
            iir_hist[0] = iir_in as i32;
            self.recorrelated[n][ch] = out as i32;
        }
        Ok(())
    }
}

/// Shifts left by a possibly negative amount (a negative amount shifts right).
#[inline]
fn shift_left(value: i32, shift: i32) -> i32 {
    if shift >= 0 {
        value << shift
    } else {
        value >> -shift
    }
}

/// Folds a 32-bit lossless check word to the eight bits carried by the restart header.
#[inline]
#[must_use]
pub fn fold_lossless(word: u32) -> u8 {
    let x = word ^ (word >> 16);
    (x ^ (x >> 8)) as u8
}

/// Everything the block callback needs besides the parser state and buffers.
#[derive(Debug, Clone)]
struct Core {
    sub: [SubstreamDecoder; 4],
    /// Rematrix buffer of the presentation being decoded.
    rematrix: Box<[[i32; REMATRIX_COLUMNS]; MAX_SAMPLES_PER_AU]>,
    /// Output rows of the presentation being decoded.
    output: Box<[[i32; MAX_CHANNELS]; MAX_SAMPLES_PER_AU]>,
    samples_per_au: usize,
    /// Substream whose matrices produce the output.
    top: usize,
    substream_mask: u8,
    stats: DecodeStats,
    unit_index: u64,
    /// Output timing of the previous access unit's restart header of `top`, for
    /// duplicate detection.
    duplicate_timing: bool,
    duplicate_samples: bool,
    /// Input/output timing, branches and duplicates.
    timing: TimingModel,
}

impl Core {
    fn on_block(
        &mut self,
        i: usize,
        block: &Block,
        state: &ParserState,
        buf: &SampleBuffer,
    ) -> Result<()> {
        let ss = &state.substream[i];
        if let Some(rh) = &block.restart {
            let rt = self.timing.restart_header(i, rh.output_timing);
            let sd = &mut self.sub[i];
            if sd.active && i == self.top {
                if rt.branch.is_some_and(|b| b.is_valid()) {
                    // The check word spans the splice; the header belongs to the new section.
                    self.stats.lossless_checks_skipped += 1;
                } else {
                    self.stats.lossless_checks += 1;
                    let computed = fold_lossless(sd.lossless_accum);
                    if computed != rh.lossless_check {
                        self.stats.lossless_mismatches += 1;
                        let unit = self.unit_index;
                        self.stats.note(|| {
                            format!(
                                "access unit {unit}: substream {i}: lossless check {computed:#04X} differs from header {:#04X}",
                                rh.lossless_check
                            )
                        });
                    }
                }
                if rt.duplicate_timing {
                    self.duplicate_timing = true;
                }
            }
            sd.restart(ss, rh.output_timing);
        }
        let sd = &mut self.sub[i];
        if !sd.active {
            return Err(Error::malformed(format!(
                "substream {i}: block before a restart header"
            )));
        }
        if let Some(header) = &block.header {
            if let Some(m) = header.matrixing {
                sd.update_matrices(ss, m);
            }
            for ch in ss.min_chan..=ss.max_chan {
                if let Some(cp) = header.channel[ch]
                    && cp.new_iir
                    && ss.iir[ch].new_states
                {
                    sd.iir_hist[ch] = ss.iir[ch].state;
                }
            }
        }
        let n0 = sd.decoded;
        let n1 = n0 + block.block_size;
        if n1 > buf.len || n1 > MAX_SAMPLES_PER_AU {
            return Err(Error::malformed(format!(
                "substream {i}: block samples {n0}..{n1} beyond the buffer"
            )));
        }
        for ch in ss.min_chan..=ss.max_chan {
            sd.recorrelate(ss, ch, n0, n1, buf)?;
        }
        if i == self.top {
            self.rematrix_block(i, state, buf, n0, n1);
        }
        let sd = &mut self.sub[i];
        sd.decoded = n1;
        if n1 == self.samples_per_au {
            if sd.sync_word == SYNC_C {
                for pmi in 0..ss.primitive_matrices {
                    for ch in 0..=sd.max_matrix_chan {
                        sd.m[pmi][ch] = sd.m[pmi][ch].wrapping_add(sd.d[pmi][ch]);
                    }
                }
            }
            if i == self.top {
                if sd.output_bits >> sd.max_bits != 0 {
                    self.stats.max_bits_violations += 1;
                    let unit = self.unit_index;
                    let max_bits = sd.max_bits;
                    self.stats.note(|| {
                        format!(
                            "access unit {unit}: substream {i}: outputs exceed max_bits {max_bits}"
                        )
                    });
                }
                if self.duplicate_timing && sd.lossless_au == sd.lossless_prev {
                    self.duplicate_samples = true;
                }
                sd.lossless_prev = sd.lossless_au;
            }
        }
        Ok(())
    }

    /// Rematrixes and remaps samples `n0..n1` of the presentation topped by substream `i`.
    #[allow(
        clippy::needless_range_loop,
        reason = "matrix rows, coefficients and bypassed bits share one index"
    )]
    fn rematrix_block(
        &mut self,
        i: usize,
        state: &ParserState,
        buf: &SampleBuffer,
        n0: usize,
        n1: usize,
    ) {
        // Union of the recorrelated channels of every substream in the presentation.
        for j in 0..=i {
            if (self.substream_mask >> j) & 1 == 0 {
                continue;
            }
            let (lo, hi) = (self.sub[j].min_chan, self.sub[j].max_chan);
            for n in n0..n1 {
                let src = &self.sub[j].recorrelated[n];
                self.rematrix[n][lo..=hi].copy_from_slice(&src[lo..=hi]);
            }
        }
        let ss = &state.substream[i];
        let sd = &mut self.sub[i];
        let mmc = sd.max_matrix_chan;
        let matrices = ss.primitive_matrices;
        let spa = self.samples_per_au;
        let table_len = spa.next_power_of_two();
        let table_mask = table_len - 1;
        if n0 == 0 {
            sd.lossless_au = 0;
            sd.output_bits = 0;
            if sd.sync_word != SYNC_A {
                fill_table_bc(&mut sd.dither_seed, &mut sd.dither_table[..table_len]);
            }
        }
        let recip = (1i64 << 16) / spa as i64;
        for n in n0..n1 {
            let row = &mut self.rematrix[n];
            let lsb = &buf.bypassed_lsb[n];
            match sd.sync_word {
                SYNC_A => {
                    let (a, b) = noise_pair_a(&mut sd.dither_seed, sd.dither_shift);
                    row[mmc + 1] = a;
                    row[mmc + 2] = b;
                    for pmi in 0..matrices {
                        let out_ch = usize::from(ss.matrix_ch[pmi]);
                        let mut acc = 0i64;
                        for ch in 0..=mmc + 2 {
                            acc += i64::from(row[ch]) * i64::from(sd.m[pmi][ch]);
                        }
                        row[out_ch] = matrix_output(acc, ss.quant_step_size[out_ch], lsb[pmi]);
                    }
                }
                SYNC_B => {
                    for pmi in 0..matrices {
                        let out_ch = usize::from(ss.matrix_ch[pmi]);
                        let mut acc = 0i64;
                        for ch in 0..=mmc {
                            acc += i64::from(row[ch]) * i64::from(sd.m[pmi][ch]);
                        }
                        let scale = ss.dither_scale[pmi];
                        if scale != 0 {
                            let index = (matrices - pmi) * (2 * n + 1) + n;
                            acc += i64::from(sd.dither_table[index & table_mask]) << (11 + scale);
                        }
                        row[out_ch] = matrix_output(acc, ss.quant_step_size[out_ch], lsb[pmi]);
                    }
                }
                _ => {
                    for pmi in 0..matrices {
                        let out_ch = usize::from(ss.matrix_ch[pmi]);
                        let mut acc = 0i64;
                        let mut acc_delta = 0i64;
                        for ch in 0..=mmc {
                            let v = i64::from(row[ch]);
                            acc += v * i64::from(sd.m[pmi][ch]);
                            acc_delta += v * i64::from(sd.d[pmi][ch]);
                        }
                        let scale = ss.dither_scale[pmi];
                        if scale != 0 {
                            let index = (matrices - pmi) * (2 * n + 1) + n;
                            acc += i64::from(sd.dither_table[index & table_mask]) << (11 + scale);
                        }
                        acc += (acc_delta >> MATRIX_FRAC) * n as i64 * (recip << 2);
                        row[out_ch] = matrix_output(acc, ss.quant_step_size[out_ch], lsb[pmi]);
                    }
                }
            }
            // remap
            let out = &mut self.output[n];
            let mut check = 0u32;
            let mut bits = 0u32;
            for chi in 0..=mmc {
                let shift = ss.output_shift[chi];
                let v = shift_left(row[chi], i32::from(shift));
                out[usize::from(sd.ch_assign[chi])] = v;
                check ^= (v as u32 & 0xFF_FFFF) << (chi & 7);
                bits |= (v ^ (v >> 31)) as u32;
            }
            sd.lossless_au ^= check;
            sd.lossless_accum ^= check;
            sd.output_bits |= bits;
        }
    }
}

/// Final value of a matrix output: 18 fractional bits removed, quantiser step
/// bits cleared, bypassed LSBs added.
#[inline]
fn matrix_output(acc: i64, qss: u32, bypassed_lsb: i32) -> i32 {
    (((acc >> MATRIX_FRAC) as i32) & !((1i32 << qss) - 1)).wrapping_add(bypassed_lsb)
}

/// Decoder for one presentation of a TrueHD stream.
#[derive(Debug, Clone)]
pub struct Decoder {
    config: StreamConfig,
    parser: ParserState,
    presentation: usize,
    buffers: [Box<SampleBuffer>; 4],
    core: Core,
    keep_duplicates: bool,
}

impl Decoder {
    /// Creates a decoder for `presentation` (0..=3) of the stream described by
    /// its first major sync.
    pub fn new(major_sync: &MajorSync, presentation: usize) -> Result<Self> {
        let config = StreamConfig::from_major_sync(major_sync)?;
        let map = &config.presentations;
        let top = match map.kind(presentation) {
            PresentationKind::Independent | PresentationKind::DownmixOf(_) => presentation,
            PresentationKind::CopyOf(i) => i,
            PresentationKind::Invalid => {
                return Err(Error::malformed(format!(
                    "presentation {presentation} is not available"
                )));
            }
        };
        if top >= usize::from(config.substreams) {
            return Err(Error::malformed(format!(
                "presentation {presentation} needs substream {top} but the stream has {}",
                config.substreams
            )));
        }
        let parser = ParserState::new(&config);
        let core = Core {
            sub: core::array::from_fn(|_| SubstreamDecoder::default()),
            rematrix: Box::new([[0; REMATRIX_COLUMNS]; MAX_SAMPLES_PER_AU]),
            output: Box::new([[0; MAX_CHANNELS]; MAX_SAMPLES_PER_AU]),
            samples_per_au: usize::from(config.samples_per_au),
            top,
            substream_mask: map.mask(top),
            stats: DecodeStats::default(),
            unit_index: 0,
            duplicate_timing: false,
            duplicate_samples: false,
            timing: TimingModel::new(StreamTiming::new(major_sync, &config)),
        };
        Ok(Self {
            config,
            parser,
            presentation,
            buffers: core::array::from_fn(|_| Box::default()),
            core,
            keep_duplicates: false,
        })
    }

    /// Keeps access units flagged as duplicates instead of marking them.
    pub fn keep_duplicates(&mut self, keep: bool) {
        self.keep_duplicates = keep;
    }

    /// The requested presentation.
    #[must_use]
    pub fn presentation(&self) -> usize {
        self.presentation
    }

    /// The stream configuration in force.
    #[must_use]
    pub fn config(&self) -> &StreamConfig {
        &self.config
    }

    /// The presentation whose substream produces the output (differs from the
    /// requested one only when that is a copy).
    #[must_use]
    pub fn source_presentation(&self) -> usize {
        self.core.top
    }

    /// Output channels of the presentation (known after the first access unit).
    #[must_use]
    pub fn channels(&self) -> usize {
        self.core.sub[self.core.top].max_matrix_chan + 1
    }

    /// Counters so far.
    #[must_use]
    pub fn stats(&self) -> &DecodeStats {
        &self.core.stats
    }

    /// The timing model (jumps, branches) so far.
    #[must_use]
    pub fn timing(&self) -> &TimingModel {
        &self.core.timing
    }

    /// Decodes one access unit.
    pub fn decode(&mut self, unit: &[u8]) -> Result<Decoded<'_>> {
        let (au, config) = AccessUnit::parse(unit, Some(&self.config))?;
        if let Some(ms) = &au.major_sync {
            if let Some(what) = self.config.incompatible_with(&config) {
                return Err(Error::malformed(format!(
                    "{what} changed at a major sync (not supported yet)"
                )));
            }
            self.parser.update(&config);
            self.core
                .timing
                .update_config(StreamTiming::new(ms, &config));
            self.config = config;
        }
        self.core.duplicate_timing = false;
        self.core.duplicate_samples = false;
        if self
            .core
            .timing
            .begin_unit(au.header.input_timing, u32::from(au.header.length_words))
        {
            self.core.stats.input_jumps += 1;
        }
        let top = self.core.top;
        for i in 0..=top {
            if (self.core.substream_mask >> i) & 1 == 0 {
                continue;
            }
            let entry: &DirectoryEntry = au.directory.get(i).ok_or_else(|| {
                Error::malformed(format!(
                    "access unit without a directory entry for substream {i}"
                ))
            })?;
            let bytes = unit.get(au.segment_range(i)).ok_or_else(|| {
                Error::malformed(format!("substream {i} range outside the access unit"))
            })?;
            if !self.parser.substream[i].restart_seen && entry.restart_nonexistent {
                return Err(Error::malformed(format!(
                    "substream {i}: no restart header seen yet"
                )));
            }
            self.core.sub[i].decoded = 0;
            self.core.sub[i].zero_samples = 0;
            let core = &mut self.core;
            let seg = Segment::parse_with(
                bytes,
                &mut self.parser,
                i,
                entry.crc_present,
                &mut self.buffers[i],
                |block, state, buf| core.on_block(i, block, state, buf),
            )?;
            if let Some(t) = seg.terminator
                && t.zero_samples_indicated
            {
                self.core.sub[i].zero_samples = usize::from(t.zero_samples);
            }
            if !(seg.parity_ok && seg.crc_ok && seg.end_ok && seg.sample_count_ok) {
                self.core.stats.segment_problems += 1;
                let unit_index = self.core.unit_index;
                self.core.stats.note(|| {
                    format!("access unit {unit_index}: substream {i}: segment integrity problem")
                });
            }
        }
        self.core.timing.end_unit();
        self.core.stats.valid_branches = self.core.timing.valid_branches() as u64;
        self.core.stats.invalid_branches = self.core.timing.invalid_branches() as u64;
        let spa = self.core.samples_per_au;
        let len = spa.saturating_sub(self.core.sub[top].zero_samples);
        let duplicate =
            self.core.duplicate_timing && self.core.duplicate_samples && !self.keep_duplicates;
        self.core.stats.units += 1;
        if duplicate {
            self.core.stats.duplicates += 1;
        } else {
            self.core.stats.samples += len as u64;
        }
        self.core.unit_index += 1;
        Ok(Decoded {
            pcm: &self.core.output[..len],
            channels: self.core.sub[top].max_matrix_chan + 1,
            duplicate,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::FilterCoeffs;

    #[test]
    fn lossless_fold_matches_the_reference_definition() {
        assert_eq!(fold_lossless(0), 0);
        assert_eq!(fold_lossless(0x0000_00AB), 0xAB);
        assert_eq!(fold_lossless(0x1234_5678), 0x12 ^ 0x34 ^ 0x56 ^ 0x78);
    }

    #[test]
    fn fir_predictor_reconstructs_a_ramp() {
        // An order-1 FIR with coefficient 1.0 predicts "same as last sample":
        // residuals of 1 decode to a ramp.
        let mut ss = SubstreamState {
            sync_word: SYNC_A,
            restart_seen: true,
            ..SubstreamState::default()
        };
        ss.fir[0] = FilterCoeffs {
            order: 1,
            coeff_q: 12,
            coeff_bits: 14,
            coeff_shift: 0,
            coeff: [1 << 12, 0, 0, 0, 0, 0, 0, 0],
            new_states: false,
            state: [0; 8],
        };
        let mut sd = SubstreamDecoder {
            active: true,
            sync_word: SYNC_A,
            ..SubstreamDecoder::default()
        };
        let mut buf = SampleBuffer::default();
        for n in 0..8 {
            buf.samples[n][0] = 1;
        }
        buf.len = 8;
        sd.recorrelate(&ss, 0, 0, 8, &buf).unwrap();
        let outputs: Vec<i32> = (0..8).map(|n| sd.recorrelated[n][0]).collect();
        assert_eq!(outputs, vec![1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(sd.fir_hist[0][0], 8);
        assert_eq!(sd.fir_hist[0][1], 7);
    }

    #[test]
    fn iir_prediction_uses_the_prediction_error_history() {
        // IIR of order 1 with coefficient -1.0: pred = -(previous iir input);
        // iir input = out - pred. Residuals 0 give out = pred.
        let mut ss = SubstreamState {
            sync_word: SYNC_A,
            restart_seen: true,
            ..SubstreamState::default()
        };
        ss.iir[0] = FilterCoeffs {
            order: 1,
            coeff_q: 8,
            coeff_bits: 10,
            coeff_shift: 0,
            coeff: [-(1 << 8), 0, 0, 0, 0, 0, 0, 0],
            new_states: true,
            state: [5, 0, 0, 0, 0, 0, 0, 0],
        };
        let mut sd = SubstreamDecoder {
            active: true,
            sync_word: SYNC_A,
            ..SubstreamDecoder::default()
        };
        sd.iir_hist[0] = ss.iir[0].state;
        let buf = SampleBuffer {
            len: 3,
            ..SampleBuffer::default()
        };
        sd.recorrelate(&ss, 0, 0, 3, &buf).unwrap();
        // n0: pred = -5, out = -5, iir_in = 0; n1: pred = 0, out = 0, iir_in = 0; n2: 0
        assert_eq!(sd.recorrelated[0][0], -5);
        assert_eq!(sd.recorrelated[1][0], 0);
        assert_eq!(sd.iir_hist[0][0], 0);
    }

    #[test]
    fn quantiser_step_masks_the_prediction() {
        let mut ss = SubstreamState {
            sync_word: SYNC_A,
            restart_seen: true,
            ..SubstreamState::default()
        };
        ss.quant_step_size[0] = 4;
        ss.fir[0] = FilterCoeffs {
            order: 1,
            coeff_q: 8,
            coeff_bits: 10,
            coeff_shift: 0,
            coeff: [1 << 8, 0, 0, 0, 0, 0, 0, 0],
            new_states: false,
            state: [0; 8],
        };
        let mut sd = SubstreamDecoder {
            active: true,
            sync_word: SYNC_A,
            ..SubstreamDecoder::default()
        };
        sd.fir_hist[0][0] = 0x123; // previous output with low bits set
        let mut buf = SampleBuffer::default();
        buf.samples[0][0] = 16;
        buf.len = 1;
        sd.recorrelate(&ss, 0, 0, 1, &buf).unwrap();
        assert_eq!(sd.recorrelated[0][0], 16 + (0x123 & !0xF));
    }

    #[test]
    fn matrix_output_rounds_masks_and_adds_bypassed_bits() {
        assert_eq!(matrix_output(5 << 18, 0, 0), 5);
        assert_eq!(matrix_output((5 << 18) + 1, 0, 0), 5);
        assert_eq!(matrix_output(-1, 0, 0), -1);
        assert_eq!(matrix_output(0b10111 << 18, 2, 0b01), 0b10101);
    }

    #[test]
    fn matrix_scaling_follows_the_sync_word() {
        let mut ss = SubstreamState {
            sync_word: SYNC_B,
            restart_seen: true,
            max_matrix_chan: 1,
            primitive_matrices: 1,
            ..SubstreamState::default()
        };
        ss.frac_bits[0] = 2;
        ss.matrix_coeff[0][0] = 3;
        ss.matrix_coeff[0][1] = -1;
        let mut sd = SubstreamDecoder::default();
        sd.update_matrices(
            &ss,
            Matrixing {
                new_matrix: true,
                new_matrix_config: true,
                ..Matrixing::default()
            },
        );
        assert_eq!(sd.m[0][0], 3 << 16);
        assert_eq!(sd.m[0][1], -1 << 16);

        ss.sync_word = SYNC_C;
        ss.cf_shift_code[0] = 2;
        ss.delta_bits[0] = 3;
        ss.delta_precision[0] = 1;
        ss.delta_cf[0][1] = 5;
        sd.update_matrices(
            &ss,
            Matrixing {
                new_matrix: true,
                new_matrix_config: true,
                interpolation_used: true,
                new_delta: true,
                new_delta_config: true,
            },
        );
        assert_eq!(sd.m[0][0], 3 << 18);
        assert_eq!(sd.d[0][1], 5 << 17);
        // interpolation switched off clears the deltas but keeps the matrix
        sd.update_matrices(
            &ss,
            Matrixing {
                new_matrix: false,
                ..Matrixing::default()
            },
        );
        assert_eq!(sd.d[0][1], 0);
        assert_eq!(sd.m[0][0], 3 << 18);
    }
}
