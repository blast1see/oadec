//! Sample reconstruction: recorrelation, rematrixing, remapping and the
//! lossless check, for one presentation ([`Decoder`]) or several at once
//! ([`MultiDecoder`](crate::MultiDecoder)), on the same code.
//!
//! The decoder drives the segment parser block by block. After every block the
//! coded samples of the substream are recorrelated (FIR/IIR prediction); when the
//! substream is the top of a requested presentation the union of the
//! recorrelated channels of every substream in the presentation is rematrixed
//! with that substream's matrices and remapped to output channels.
//!
//! Each substream is parsed and recorrelated once an access unit, whatever
//! number of presentations read it. What a presentation makes of the
//! substreams -- the rematrixing, the output, the lossless check, the
//! dither its matrices draw, its damage, its counters and its timing -- is
//! its own (`docs/truehd.md`, *Several presentations in one pass*).
//!
//! All arithmetic is integer: 64-bit accumulators, 18-bit fixed-point matrix
//! coefficients, and the wrap-around and masking behaviour that the format
//! prescribes (checked bit-exactly against the other public decoders on real
//! streams, see `docs/evidence`).

use crate::au::{AccessUnit, StreamConfig};
use crate::block::{Block, SampleBuffer};
use crate::dither::{fill_table_bc, noise_pair_a};
use crate::error::{Error, Result};
use crate::extra::{ExtraData, ExtraKind};
use crate::matrix::Matrixing;
use crate::presentation::{MAX_PRESENTATIONS, PresentationKind};
use crate::restart::RestartHeader;
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
    /// Extra-data blocks whose header nibble parity failed.
    pub extra_header_parity: u64,
    /// Extra-data blocks whose declared length runs past the access unit.
    pub extra_truncated: u64,
    /// Evolution extra-data blocks whose parity byte did not match.
    pub extra_evolution_parity: u64,
    /// Extra-data blocks whose padding was not zero.
    pub extra_padding_nonzero: u64,
    /// Evolution frames whose EMDF container would not parse.
    pub evolution_container_errors: u64,
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
    /// Access units whose output is silence standing in for damage
    /// ([`Decoder::decode_concealing`]).
    pub concealed_units: u64,
    /// Samples of that silence.
    pub concealed_samples: u64,
    /// Description of the first problem seen.
    pub first_problem: Option<String>,
}

impl DecodeStats {
    fn note(&mut self, message: impl FnOnce() -> String) {
        if self.first_problem.is_none() {
            self.first_problem = Some(message());
        }
    }

    /// Counts what is wrong with the extra-data block of access unit `unit`,
    /// with the checks `verify` makes.
    ///
    /// The block carries no audio, so nothing here stops a decode. It does
    /// carry the object metadata, and a delivery that read none of these
    /// checks exited clean on a stream `verify` rejected: a corrupted
    /// Evolution parity byte left `decode` at 0 and `verify` at 7.
    fn take_extra(&mut self, unit: u64, extra: &ExtraData) {
        if !extra.header_parity_ok {
            self.extra_header_parity += 1;
            self.note(|| format!("access unit {unit}: extra data header parity mismatch"));
        }
        if !extra.padding_zero {
            self.extra_padding_nonzero += 1;
            self.note(|| format!("access unit {unit}: non-zero extra data padding"));
        }
        match &extra.kind {
            ExtraKind::Truncated => {
                self.extra_truncated += 1;
                self.note(|| format!("access unit {unit}: extra data runs past the access unit"));
            }
            ExtraKind::Evolution { frame, .. } => {
                if extra.parity_ok == Some(false) {
                    self.extra_evolution_parity += 1;
                    self.note(|| format!("access unit {unit}: Evolution parity mismatch"));
                }
                if !frame.is_empty()
                    && let Err(e) = oadec_emdf::container::parse_evolution(frame)
                {
                    self.evolution_container_errors += 1;
                    self.note(|| format!("access unit {unit}: evolution frame: {e}"));
                }
            }
            ExtraKind::Padding | ExtraKind::Opaque(_) => {}
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
    /// The output is silence standing in for damage
    /// ([`Decoder::decode_concealing`]).
    pub concealed: bool,
    /// See [`Decoder::output_delta`].
    pub output_delta: Option<u16>,
}

/// What a decoder does at a major sync that changes the configuration it was
/// set up for: the substream count, `substream_info`, the samples per access
/// unit or the sampling frequency ([`StreamConfig::incompatible_with`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OnConfigChange {
    /// The access unit is not decoded and [`Error::ConfigChanged`] comes back
    /// for it; everything before it was decoded and is sound. What a decoder
    /// has always done.
    #[default]
    Stop,
    /// The decoder sets itself up again for the new configuration and goes
    /// on. The access unit that carries the change is decoded once, with the
    /// new configuration, and the change is reported beside it
    /// ([`Decoder::decode_following`], [`Decoder::config_change`]).
    Continue,
}

/// A change of configuration a decoder followed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigChange {
    /// Index of the access unit that carries the change, counted from the
    /// first the decoder was given.
    pub unit: u64,
    /// What changed, as [`StreamConfig::incompatible_with`] names it.
    pub what: &'static str,
    /// The configuration before the change.
    pub old: StreamConfig,
    /// The configuration the access unit and those after it are decoded with.
    pub new: StreamConfig,
}

impl ConfigChange {
    /// The presentations the change takes away: decodable before it and not
    /// after ([`StreamConfig::has_presentation`]).
    #[must_use]
    pub fn gone(&self) -> [bool; MAX_PRESENTATIONS] {
        core::array::from_fn(|p| self.old.has_presentation(p) && !self.new.has_presentation(p))
    }

    /// The presentations the change brings: decodable after it and not
    /// before.
    #[must_use]
    pub fn back(&self) -> [bool; MAX_PRESENTATIONS] {
        core::array::from_fn(|p| !self.old.has_presentation(p) && self.new.has_presentation(p))
    }
}

/// One access unit decoded by [`Decoder::decode_following`]: the samples,
/// the fault that was concealed if any, and the change of configuration the
/// unit carries if it carries one, all lent by one borrow of the decoder.
#[derive(Debug)]
pub struct Following<'a> {
    /// The samples.
    pub decoded: Decoded<'a>,
    /// The fault that was concealed, as [`Decoder::decode_concealing`] gives it.
    pub fault: Option<Error>,
    /// The change of configuration this access unit carries.
    pub change: Option<&'a ConfigChange>,
}

/// What [`Decoder::decode`] made of one access unit, before it is lent out.
#[derive(Debug, Clone, Copy)]
struct UnitOut {
    len: usize,
    channels: usize,
    duplicate: bool,
    concealed: bool,
}

/// Decoder state of one substream: what its segments carry from block to
/// block and from unit to unit, whatever presentation reads it.
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
    max_bits: u8,
    output_timing: u16,
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
            max_bits: 24,
            output_timing: 0,
            fir_hist: [[0; FILTER_ORDER]; MAX_CHANNELS],
            iir_hist: [[0; FILTER_ORDER]; MAX_CHANNELS],
            recorrelated: Box::new([[0; MAX_CHANNELS]; MAX_SAMPLES_PER_AU]),
            m: [[0; REMATRIX_COLUMNS]; MAX_MATRICES],
            d: [[0; MAX_CHANNELS]; MAX_MATRICES],
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
        self.max_bits = ss.max_bits;
        self.output_timing = output_timing;
        self.fir_hist = [[0; FILTER_ORDER]; MAX_CHANNELS];
        self.iir_hist = [[0; FILTER_ORDER]; MAX_CHANNELS];
        self.m = [[0; REMATRIX_COLUMNS]; MAX_MATRICES];
        self.d = [[0; MAX_CHANNELS]; MAX_MATRICES];
        // `recorrelated`, `decoded` and `zero_samples` describe the access
        // unit in progress; they stay.
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

/// Which of the first `matrices` matrices move: which have, among the deltas
/// the matrix stage reads, those of channels `0..=max_matrix_chan`, one that
/// is not zero. A delta beyond those channels, or of a matrix beyond the
/// ones in use, is read by nothing and moves nothing.
#[inline(always)]
fn moving_matrices(
    d: &[[i32; MAX_CHANNELS]; MAX_MATRICES],
    matrices: usize,
    max_matrix_chan: usize,
) -> [bool; MAX_MATRICES] {
    let mut moving = [false; MAX_MATRICES];
    for (moves, deltas) in moving.iter_mut().zip(d).take(matrices) {
        *moves = deltas[..=max_matrix_chan].iter().any(|&delta| delta != 0);
    }
    moving
}

/// Final value of a matrix output: 18 fractional bits removed, the bits under
/// the quantiser step cleared, the bypassed LSBs added.
///
/// `uncut` leaves the cut out. A matrix of a `0x31EA` or a `0x31EB` substream
/// that sends a bypassed LSB is not cut to the step of its channel: Dolby's
/// decoder gives the sum as it is with the bit added, on every sample of
/// streams written to ask it of each kind (`docs/truehd.md`, *A bypassed bit
/// over a quantiser step*). No real stream has a matrix with both a bypassed
/// bit and a step, so nothing but a written stream can tell the two readings
/// apart. A `0x31EC` substream, whose matrices send a count of bypassed bits,
/// has not been put to Dolby's decoder and is cut as before.
#[inline]
fn matrix_output(acc: i64, qss: u32, bypassed_lsb: i32, uncut: bool) -> i32 {
    let sum = (acc >> MATRIX_FRAC) as i32;
    let sum = if uncut {
        sum
    } else {
        sum & !((1i32 << qss) - 1)
    };
    sum.wrapping_add(bypassed_lsb)
}

/// What one presentation came to in the access unit being decoded.
#[derive(Debug, Clone, Copy)]
enum Outcome {
    /// Nothing yet.
    Pending,
    /// Samples, or silence standing in for damage with the fault that caused
    /// it (an index into the unit's [`Faults`]).
    Out(UnitOut, Option<usize>),
    /// The unit could not be read for this presentation: the fault, not yet
    /// concealed.
    Failed(usize),
    /// The configuration in force does not carry the presentation.
    Gone,
}

/// The faults of the access unit being decoded, each made once and lent to
/// every presentation it reaches.
#[derive(Debug, Default)]
struct Faults(Vec<Option<Error>>);

impl Faults {
    fn push(&mut self, e: Error) -> usize {
        self.0.push(Some(e));
        self.0.len() - 1
    }

    fn get(&self, k: usize) -> Option<&Error> {
        self.0.get(k).and_then(Option::as_ref)
    }

    fn take(&mut self, k: usize) -> Option<Error> {
        self.0.get_mut(k).and_then(Option::take)
    }
}

impl Clone for Faults {
    /// The faults of a unit are lent and taken within its decode; a clone
    /// starts without them.
    fn clone(&self) -> Self {
        Self::default()
    }
}

/// Everything one presentation decodes on its own: the matrices of its top
/// substream over the recorrelated channels of every substream it reads,
/// and what reads the result -- the output, the lossless check, the dither
/// drawn for its matrices, its damage, its counters and its timing.
#[derive(Debug, Clone)]
struct Lane {
    presentation: usize,
    /// The configuration in force does not carry the presentation.
    gone: bool,
    /// Substream whose matrices produce the output.
    top: usize,
    substream_mask: u8,
    keep_duplicates: bool,
    /// A restart header of `top` has been applied for this presentation.
    top_active: bool,
    /// The register the dither and noise of `top`'s matrices are drawn from.
    dither_seed: u32,
    /// Dither table of sync words B and C (regenerated per access unit).
    dither_table: [i32; 256],
    /// Lossless check word of the current access unit.
    lossless_au: u32,
    /// Lossless check word accumulated since the restart header.
    lossless_accum: u32,
    /// Lossless check word of the previous access unit.
    lossless_prev: u32,
    /// Sign-folded OR of every output of the access unit.
    output_bits: u32,
    /// Part of the section since the restart header of `top` was concealed,
    /// so its lossless check word cannot hold.
    spoiled: bool,
    /// Rematrix buffer.
    rematrix: Box<[[i32; REMATRIX_COLUMNS]; MAX_SAMPLES_PER_AU]>,
    /// Output rows.
    output: Box<[[i32; MAX_CHANNELS]; MAX_SAMPLES_PER_AU]>,
    stats: DecodeStats,
    unit_index: u64,
    /// The restart header of `top` repeats the output timing of the one
    /// before, and the unit's samples repeat the unit before: duplicate
    /// detection.
    duplicate_timing: bool,
    duplicate_samples: bool,
    /// Input/output timing, branches and duplicates.
    timing: TimingModel,
    /// The current access unit has begun in the timing model and not ended.
    unit_open: bool,
    /// The output timing jump of the restart header of `top` in the current
    /// access unit, if it carries one (see [`Decoder::output_delta`]).
    output_delta: Option<u16>,
    /// Substreams a fault left untrustworthy for this presentation: each is
    /// read again from its next restart header, and the presentation is
    /// silent until every one of its substreams has been.
    damaged: [bool; 4],
    /// A substream of the presentation is damaged in this unit.
    silent: bool,
    /// The first fault of this unit.
    fault: Option<usize>,
    outcome: Outcome,
}

/// The substream whose matrices make presentation `presentation`, and the
/// substreams it reads, under `config`.
fn top_of(config: &StreamConfig, presentation: usize) -> Result<(usize, u8)> {
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
    Ok((top, map.mask(top)))
}

impl Lane {
    fn new(presentation: usize, config: &StreamConfig, timing: StreamTiming) -> Result<Self> {
        let (top, substream_mask) = top_of(config, presentation)?;
        Ok(Self {
            presentation,
            gone: false,
            top,
            substream_mask,
            keep_duplicates: false,
            top_active: false,
            dither_seed: 0,
            dither_table: [0; 256],
            lossless_au: 0,
            lossless_accum: 0,
            lossless_prev: 0,
            output_bits: 0,
            spoiled: false,
            rematrix: Box::new([[0; REMATRIX_COLUMNS]; MAX_SAMPLES_PER_AU]),
            output: Box::new([[0; MAX_CHANNELS]; MAX_SAMPLES_PER_AU]),
            stats: DecodeStats::default(),
            unit_index: 0,
            duplicate_timing: false,
            duplicate_samples: false,
            timing: TimingModel::new(timing),
            unit_open: false,
            output_delta: None,
            damaged: [false; 4],
            silent: false,
            fault: None,
            outcome: Outcome::Pending,
        })
    }

    /// Sets the presentation up again under the configuration a change
    /// brought, or marks it gone when that configuration does not carry it.
    /// Its counters and its record of the timing stay.
    fn follow(&mut self, config: &StreamConfig, timing: StreamTiming) {
        let Ok((top, substream_mask)) = top_of(config, self.presentation) else {
            self.gone = true;
            return;
        };
        self.gone = false;
        self.top = top;
        self.substream_mask = substream_mask;
        self.top_active = false;
        self.dither_seed = 0;
        self.dither_table = [0; 256];
        self.lossless_au = 0;
        self.lossless_accum = 0;
        self.lossless_prev = 0;
        self.output_bits = 0;
        self.spoiled = false;
        self.duplicate_timing = false;
        self.duplicate_samples = false;
        self.timing.begin_again(timing);
        self.unit_open = false;
        self.output_delta = None;
        self.damaged = [false; 4];
    }

    /// Reads substream `i` in this unit: not stopped, and `i` one of its.
    fn reads(&self, i: usize) -> bool {
        !self.gone
            && !matches!(self.outcome, Outcome::Failed(_))
            && i <= self.top
            && (self.substream_mask >> i) & 1 == 1
    }

    /// A restart header of substream `i` that this presentation reads: the
    /// timing of every substream, and for `top` the lossless check of the
    /// section it closes and the state of the matrices' dither.
    fn restart_header(&mut self, i: usize, rh: &RestartHeader, ss: &SubstreamState) {
        let rt = self.timing.restart_header(i, rh.output_timing);
        if i != self.top {
            return;
        }
        self.output_delta = rt.output_delta;
        if self.top_active {
            if rt.branch.is_some_and(|b| b.is_valid()) || self.spoiled {
                // The check word spans the splice, or silence that stood in
                // for damage; the header belongs to the new section.
                self.stats.lossless_checks_skipped += 1;
            } else {
                self.stats.lossless_checks += 1;
                let computed = fold_lossless(self.lossless_accum);
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
        self.top_active = true;
        self.dither_seed = ss.dither_seed;
        self.dither_table = [0; 256];
        self.lossless_au = 0;
        self.lossless_accum = 0;
        self.output_bits = 0;
        self.spoiled = false;
    }

    /// The end of the access unit in `top`: the `max_bits` check and
    /// duplicate detection.
    fn end_of_top(&mut self, i: usize, max_bits: u8) {
        if self.output_bits >> max_bits != 0 {
            self.stats.max_bits_violations += 1;
            let unit = self.unit_index;
            self.stats.note(|| {
                format!("access unit {unit}: substream {i}: outputs exceed max_bits {max_bits}")
            });
        }
        if self.duplicate_timing && self.lossless_au == self.lossless_prev {
            self.duplicate_samples = true;
        }
        self.lossless_prev = self.lossless_au;
    }

    /// Rematrixes and remaps samples `n0..n1` of the presentation, `top`
    /// being substream `i`.
    #[allow(
        clippy::needless_range_loop,
        reason = "matrix rows, coefficients and bypassed bits share one index"
    )]
    #[allow(clippy::too_many_arguments, reason = "the block and where it stands")]
    fn rematrix_block(
        &mut self,
        sub: &[SubstreamDecoder; 4],
        i: usize,
        state: &ParserState,
        buf: &SampleBuffer,
        n0: usize,
        n1: usize,
        spa: usize,
    ) {
        // Union of the recorrelated channels of every substream in the presentation.
        for j in 0..=i {
            if (self.substream_mask >> j) & 1 == 0 {
                continue;
            }
            let (lo, hi) = (sub[j].min_chan, sub[j].max_chan);
            for n in n0..n1 {
                let src = &sub[j].recorrelated[n];
                self.rematrix[n][lo..=hi].copy_from_slice(&src[lo..=hi]);
            }
        }
        let ss = &state.substream[i];
        let sd = &sub[i];
        let mmc = sd.max_matrix_chan;
        let matrices = ss.primitive_matrices;
        let table_len = spa.next_power_of_two();
        let table_mask = table_len - 1;
        if n0 == 0 {
            self.lossless_au = 0;
            self.output_bits = 0;
            if sd.sync_word != SYNC_A {
                fill_table_bc(&mut self.dither_seed, &mut self.dither_table[..table_len]);
            }
        }
        let recip = (1i64 << 16) / spa as i64;
        // A `0x31EC` matrix is interpolated over the access unit by a delta
        // for every coefficient, and most of the time it stands still: its
        // deltas are zero then, the sum of the samples by them is zero
        // whatever the samples are, and it is not multiplied out. Over the
        // widest presentations of six object streams 83 to 100 per cent of
        // the matrices applied stand still, and the deltas were half of what
        // the stage multiplied.
        let moving = moving_matrices(&sd.d, matrices, mmc);
        // The presentation's state apart, so that nothing the loop writes is
        // read back through `self`.
        let rematrix = &mut *self.rematrix;
        let output = &mut *self.output;
        let dither_table = &self.dither_table;
        let mut dither_seed = self.dither_seed;
        let mut lossless_au = self.lossless_au;
        let mut lossless_accum = self.lossless_accum;
        let mut output_bits = self.output_bits;
        for n in n0..n1 {
            let row = &mut rematrix[n];
            let lsb = &buf.bypassed_lsb[n];
            match sd.sync_word {
                SYNC_A => {
                    let (a, b) = noise_pair_a(&mut dither_seed, sd.dither_shift);
                    row[mmc + 1] = a;
                    row[mmc + 2] = b;
                    for pmi in 0..matrices {
                        let out_ch = usize::from(ss.matrix_ch[pmi]);
                        let mut acc = 0i64;
                        for ch in 0..=mmc + 2 {
                            acc += i64::from(row[ch]) * i64::from(sd.m[pmi][ch]);
                        }
                        row[out_ch] = matrix_output(
                            acc,
                            ss.quant_step_size[out_ch],
                            lsb[pmi],
                            ss.lsb_bypass_used[pmi],
                        );
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
                            acc += i64::from(dither_table[index & table_mask]) << (11 + scale);
                        }
                        row[out_ch] = matrix_output(
                            acc,
                            ss.quant_step_size[out_ch],
                            lsb[pmi],
                            ss.lsb_bypass_used[pmi],
                        );
                    }
                }
                _ => {
                    for pmi in 0..matrices {
                        let out_ch = usize::from(ss.matrix_ch[pmi]);
                        let mut acc = 0i64;
                        let mut acc_delta = 0i64;
                        if moving[pmi] {
                            for ch in 0..=mmc {
                                let v = i64::from(row[ch]);
                                acc += v * i64::from(sd.m[pmi][ch]);
                                acc_delta += v * i64::from(sd.d[pmi][ch]);
                            }
                        } else {
                            for ch in 0..=mmc {
                                acc += i64::from(row[ch]) * i64::from(sd.m[pmi][ch]);
                            }
                        }
                        let scale = ss.dither_scale[pmi];
                        if scale != 0 {
                            let index = (matrices - pmi) * (2 * n + 1) + n;
                            acc += i64::from(dither_table[index & table_mask]) << (11 + scale);
                        }
                        acc += (acc_delta >> MATRIX_FRAC) * n as i64 * (recip << 2);
                        row[out_ch] =
                            matrix_output(acc, ss.quant_step_size[out_ch], lsb[pmi], false);
                    }
                }
            }
            // remap
            let out = &mut output[n];
            let mut check = 0u32;
            let mut bits = 0u32;
            for chi in 0..=mmc {
                let shift = ss.output_shift[chi];
                let v = shift_left(row[chi], i32::from(shift));
                out[usize::from(sd.ch_assign[chi])] = v;
                check ^= (v as u32 & 0xFF_FFFF) << (chi & 7);
                bits |= (v ^ (v >> 31)) as u32;
            }
            lossless_au ^= check;
            lossless_accum ^= check;
            output_bits |= bits;
        }
        self.dither_seed = dither_seed;
        self.lossless_au = lossless_au;
        self.lossless_accum = lossless_accum;
        self.output_bits = output_bits;
    }

    /// Marks substream `from` and every substream of the presentation after
    /// it as damaged: they did not read this access unit.
    fn damage_from(&mut self, from: usize) {
        for i in from..4 {
            if (self.substream_mask >> i) & 1 == 1 {
                self.damaged[i] = true;
            }
        }
    }

    /// Writes `len` rows of silence for the current unit and counts them.
    fn silence(&mut self, len: usize, channels: usize) -> UnitOut {
        for row in &mut self.output[..len] {
            row.fill(0);
        }
        self.spoiled = true;
        self.output_delta = None;
        self.stats.units += 1;
        self.stats.samples += len as u64;
        self.stats.concealed_units += 1;
        self.stats.concealed_samples += len as u64;
        self.unit_index += 1;
        UnitOut {
            len,
            channels,
            duplicate: false,
            concealed: true,
        }
    }

    /// One access unit of silence for a unit that could not be read for this
    /// presentation; the clocks move on as if it had been.
    fn conceal_unit(&mut self, unit: &[u8], spa: usize, channels: usize) -> UnitOut {
        if !self.damaged.iter().any(|d| *d) {
            self.damage_from(0);
        }
        if !self.unit_open && unit.len() >= 4 {
            // The extractor framed the unit, so its header is sound: the
            // length and the input timing keep the input clock in step.
            let length_words = u32::from(u16::from_be_bytes([unit[0], unit[1]]) & 0x0FFF);
            let input_timing = u16::from_be_bytes([unit[2], unit[3]]);
            self.timing.begin_unit(input_timing, length_words);
            self.unit_open = true;
        }
        if self.unit_open {
            self.timing.end_unit();
            self.unit_open = false;
        }
        self.silence(spa, channels)
    }
}

/// One block of substream `i`: what the substream carries on, once, and for
/// each presentation in `reading` (a bit per lane) what it makes of it.
#[allow(clippy::too_many_arguments, reason = "the block and where it stands")]
fn on_block(
    sub: &mut [SubstreamDecoder; 4],
    lanes: &mut [Lane],
    reading: u8,
    spa: usize,
    i: usize,
    block: &Block,
    state: &ParserState,
    buf: &SampleBuffer,
) -> Result<()> {
    let ss = &state.substream[i];
    if let Some(rh) = &block.restart {
        for (k, lane) in lanes.iter_mut().enumerate() {
            if (reading >> k) & 1 == 1 {
                lane.restart_header(i, rh, ss);
            }
        }
        sub[i].restart(ss, rh.output_timing);
    }
    let sd = &mut sub[i];
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
    for (k, lane) in lanes.iter_mut().enumerate() {
        if (reading >> k) & 1 == 1 && lane.top == i {
            lane.rematrix_block(sub, i, state, buf, n0, n1, spa);
        }
    }
    let sd = &mut sub[i];
    sd.decoded = n1;
    if n1 == spa {
        if sd.sync_word == SYNC_C {
            for pmi in 0..ss.primitive_matrices {
                for ch in 0..=sd.max_matrix_chan {
                    sd.m[pmi][ch] = sd.m[pmi][ch].wrapping_add(sd.d[pmi][ch]);
                }
            }
        }
        let max_bits = sd.max_bits;
        for (k, lane) in lanes.iter_mut().enumerate() {
            if (reading >> k) & 1 == 1 && lane.top == i {
                lane.end_of_top(i, max_bits);
            }
        }
    }
    Ok(())
}

/// The substreams of a stream, each decoded once an access unit, and the
/// presentations made of them: what [`Decoder`] and
/// [`MultiDecoder`](crate::MultiDecoder) both stand on.
#[derive(Debug, Clone)]
pub(crate) struct Engine {
    config: StreamConfig,
    parser: ParserState,
    buffers: [Box<SampleBuffer>; 4],
    sub: [SubstreamDecoder; 4],
    samples_per_au: usize,
    /// One for each presentation asked for, in increasing order.
    lanes: Vec<Lane>,
    on_config_change: OnConfigChange,
    /// The change of configuration the last access unit carried.
    change: Option<ConfigChange>,
    /// Access units given.
    given: u64,
    faults: Faults,
    substream_decodes: [u64; 4],
    substream_faults: [u64; 4],
}

impl Engine {
    /// An engine for `presentations` (distinct, increasing, at most four).
    pub(crate) fn new(major_sync: &MajorSync, presentations: &[usize]) -> Result<Self> {
        let config = StreamConfig::from_major_sync(major_sync)?;
        let lanes = presentations
            .iter()
            .map(|&p| Lane::new(p, &config, StreamTiming::new(major_sync, &config)))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            parser: ParserState::new(&config),
            buffers: core::array::from_fn(|_| Box::default()),
            sub: core::array::from_fn(|_| SubstreamDecoder::default()),
            samples_per_au: usize::from(config.samples_per_au),
            config,
            lanes,
            on_config_change: OnConfigChange::default(),
            change: None,
            given: 0,
            faults: Faults::default(),
            substream_decodes: [0; 4],
            substream_faults: [0; 4],
        })
    }

    pub(crate) fn on_config_change(&mut self, policy: OnConfigChange) {
        self.on_config_change = policy;
    }

    pub(crate) fn keep_duplicates(&mut self, keep: bool) {
        for lane in &mut self.lanes {
            lane.keep_duplicates = keep;
        }
    }

    pub(crate) fn config_change(&self) -> Option<&ConfigChange> {
        self.change.as_ref()
    }

    pub(crate) fn units(&self) -> u64 {
        self.given
    }

    pub(crate) fn substream_decodes(&self) -> [u64; 4] {
        self.substream_decodes
    }

    pub(crate) fn substream_faults(&self) -> [u64; 4] {
        self.substream_faults
    }

    pub(crate) fn lanes(&self) -> usize {
        self.lanes.len()
    }

    pub(crate) fn presentation_of(&self, lane: usize) -> usize {
        self.lanes[lane].presentation
    }

    pub(crate) fn stats(&self, lane: usize) -> &DecodeStats {
        &self.lanes[lane].stats
    }

    fn channels(&self, lane: usize) -> usize {
        self.sub[self.lanes[lane].top].max_matrix_chan + 1
    }

    /// Substreams the presentations read in a unit under the configuration
    /// in force, as a mask.
    fn needed(&self) -> u8 {
        let mut needed = 0;
        for lane in &self.lanes {
            if !lane.gone {
                needed |= lane.substream_mask & ((2u8 << lane.top) - 1);
            }
        }
        needed & ((1u8 << self.config.substreams.min(4)) - 1)
    }

    /// The configuration changed at a major sync and the decoder follows:
    /// the substreams start again under the new one, and each presentation
    /// is set up again or is gone.
    fn follow(&mut self, ms: &MajorSync, config: StreamConfig, what: &'static str, unit: u64) {
        let old = core::mem::replace(&mut self.config, config);
        self.parser = ParserState::new(&self.config);
        // Nothing of a substream is read under the new configuration before
        // its first restart header -- the new parser state refuses a block
        // before one -- and a restart header sets everything a substream
        // carries but the samples of the unit in progress
        // (`SubstreamDecoder::restart`, and the test
        // `a_restart_header_sets_everything_a_substream_carries`). Those
        // samples are what this line is for: a substream under the top one
        // that delivers fewer samples than the unit, or none, leaves the
        // matrices above reading rows it did not write, and they are to be
        // the new configuration's silence, never the old one's samples.
        self.sub = core::array::from_fn(|_| SubstreamDecoder::default());
        self.samples_per_au = usize::from(self.config.samples_per_au);
        for lane in &mut self.lanes {
            lane.follow(&self.config, StreamTiming::new(ms, &self.config));
        }
        self.change = Some(ConfigChange {
            unit,
            what,
            old,
            new: self.config.clone(),
        });
    }

    /// Decodes one access unit for every presentation, leaving what each
    /// came to in its lane. A fault comes back in the lanes it reaches, as
    /// [`Outcome::Failed`] when the unit could not be read for the
    /// presentation and beside silence otherwise. The error is a change of
    /// configuration under [`OnConfigChange::Stop`].
    ///
    /// A segment that parses but fails its parity, CRC, end-pointer or
    /// sample-count check is a fault with `conceal`: its samples are not to
    /// be trusted, nor the substream's state after it. Without, it is
    /// counted and decoded as it stands, as it always was.
    #[allow(
        clippy::too_many_lines,
        reason = "one access unit, in the order the single decoder always read it"
    )]
    pub(crate) fn decode_unit(&mut self, unit: &[u8], conceal: bool) -> Result<()> {
        self.change = None;
        self.faults.0.clear();
        let index = self.given;
        self.given += 1;
        for lane in &mut self.lanes {
            lane.output_delta = None;
            lane.silent = false;
            lane.fault = None;
            lane.outcome = Outcome::Pending;
        }
        let (au, config) = match AccessUnit::parse(unit, Some(&self.config)) {
            Ok(parsed) => parsed,
            Err(e) => {
                let needed = self.needed();
                for (s, faults) in self.substream_faults.iter_mut().enumerate() {
                    *faults += u64::from((needed >> s) & 1);
                }
                let k = self.faults.push(e);
                for lane in &mut self.lanes {
                    lane.outcome = if lane.gone {
                        Outcome::Gone
                    } else {
                        lane.damage_from(0);
                        Outcome::Failed(k)
                    };
                }
                return Ok(());
            }
        };
        if let Some(ms) = &au.major_sync {
            if let Some(what) = self.config.incompatible_with(&config) {
                match self.on_config_change {
                    OnConfigChange::Stop => return Err(Error::ConfigChanged { what }),
                    OnConfigChange::Continue => self.follow(ms, config, what, index),
                }
            } else {
                self.parser.update(&config);
                for lane in &mut self.lanes {
                    if !lane.gone {
                        lane.timing.update_config(StreamTiming::new(ms, &config));
                    }
                }
                self.config = config;
            }
        }
        for lane in &mut self.lanes {
            if lane.gone {
                lane.outcome = Outcome::Gone;
                continue;
            }
            if let Some(extra) = &au.extra {
                lane.stats.take_extra(lane.unit_index, extra);
            }
            lane.duplicate_timing = false;
            lane.duplicate_samples = false;
            if lane
                .timing
                .begin_unit(au.header.input_timing, u32::from(au.header.length_words))
            {
                lane.stats.input_jumps += 1;
            }
            lane.unit_open = true;
        }
        let needed = self.needed();
        let spa = self.samples_per_au;
        for i in 0..4 {
            let mut reading = 0u8;
            for (k, lane) in self.lanes.iter_mut().enumerate() {
                if !lane.reads(i) {
                    continue;
                }
                self.sub[i].decoded = 0;
                self.sub[i].zero_samples = 0;
                let Some(entry) = au.directory.get(i) else {
                    lane.damage_from(i);
                    lane.outcome = Outcome::Failed(self.faults.push(Error::malformed(format!(
                        "access unit without a directory entry for substream {i}"
                    ))));
                    continue;
                };
                if unit.get(au.segment_range(i)).is_none() {
                    lane.damage_from(i);
                    lane.outcome = Outcome::Failed(self.faults.push(Error::malformed(format!(
                        "substream {i} range outside the access unit"
                    ))));
                    continue;
                }
                if lane.damaged[i] {
                    if entry.restart_nonexistent {
                        // still waiting for the restart header that makes it sound
                        lane.silent = true;
                        continue;
                    }
                    lane.damaged[i] = false;
                }
                if !self.parser.substream[i].restart_seen && entry.restart_nonexistent {
                    lane.damaged[i] = true;
                    lane.silent = true;
                    if lane.fault.is_none() {
                        lane.fault = Some(self.faults.push(Error::malformed(format!(
                            "substream {i}: no restart header seen yet"
                        ))));
                    }
                    continue;
                }
                reading |= 1 << k;
            }
            let mut decoded = false;
            if reading != 0 {
                let entry = &au.directory[i];
                let bytes = &unit[au.segment_range(i)];
                let sub = &mut self.sub;
                let lanes = &mut self.lanes;
                let parsed = Segment::parse_with(
                    bytes,
                    &mut self.parser,
                    i,
                    entry.crc_present,
                    &mut self.buffers[i],
                    |block, state, buf| on_block(sub, lanes, reading, spa, i, block, state, buf),
                );
                match parsed {
                    Err(e) => {
                        // this substream is unsound until its next restart
                        // header; the others read on, each from its own segment
                        let k = self.faults.push(e);
                        for (j, lane) in self.lanes.iter_mut().enumerate() {
                            if (reading >> j) & 1 == 1 {
                                lane.damaged[i] = true;
                                lane.silent = true;
                                lane.fault.get_or_insert(k);
                            }
                        }
                    }
                    Ok(seg) => {
                        if let Some(t) = seg.terminator
                            && t.zero_samples_indicated
                        {
                            self.sub[i].zero_samples = usize::from(t.zero_samples);
                        }
                        let sound =
                            seg.parity_ok && seg.crc_ok && seg.end_ok && seg.sample_count_ok;
                        decoded = sound || !conceal;
                        if !sound {
                            let mut failed = None;
                            for (j, lane) in self.lanes.iter_mut().enumerate() {
                                if (reading >> j) & 1 == 0 {
                                    continue;
                                }
                                lane.stats.segment_problems += 1;
                                let unit_index = lane.unit_index;
                                lane.stats.note(|| {
                                    format!(
                                        "access unit {unit_index}: substream {i}: segment integrity problem"
                                    )
                                });
                                if conceal {
                                    lane.damaged[i] = true;
                                    lane.silent = true;
                                    if lane.fault.is_none() {
                                        let k = *failed.get_or_insert_with(|| {
                                            let names = [
                                                (!seg.parity_ok, "parity"),
                                                (!seg.crc_ok, "CRC"),
                                                (!seg.end_ok, "end pointer"),
                                                (!seg.sample_count_ok, "sample count"),
                                            ]
                                            .iter()
                                            .filter(|(f, _)| *f)
                                            .map(|(_, n)| *n)
                                            .collect::<Vec<_>>()
                                            .join(", ");
                                            self.faults.push(Error::malformed(format!(
                                                "substream {i}: the segment fails its {names} check"
                                            )))
                                        });
                                        lane.fault = Some(k);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if (needed >> i) & 1 == 1 {
                if decoded {
                    self.substream_decodes[i] += 1;
                } else {
                    self.substream_faults[i] += 1;
                }
            }
        }
        for k in 0..self.lanes.len() {
            let channels = self.channels(k);
            let lane = &mut self.lanes[k];
            if !matches!(lane.outcome, Outcome::Pending) {
                continue;
            }
            lane.timing.end_unit();
            lane.unit_open = false;
            lane.stats.valid_branches = lane.timing.valid_branches() as u64;
            lane.stats.invalid_branches = lane.timing.invalid_branches() as u64;
            if lane.silent {
                // A substream of the presentation is damaged: what the others
                // decoded cannot be rematrixed without it.
                let delta = lane.output_delta;
                let out = lane.silence(spa, channels);
                lane.output_delta = delta;
                lane.outcome = Outcome::Out(out, lane.fault);
                continue;
            }
            let len = spa.saturating_sub(self.sub[lane.top].zero_samples);
            let duplicate =
                lane.duplicate_timing && lane.duplicate_samples && !lane.keep_duplicates;
            lane.stats.units += 1;
            if duplicate {
                lane.stats.duplicates += 1;
            } else {
                lane.stats.samples += len as u64;
            }
            lane.unit_index += 1;
            lane.outcome = Outcome::Out(
                UnitOut {
                    len,
                    channels,
                    duplicate,
                    concealed: false,
                },
                None,
            );
        }
        Ok(())
    }

    /// Stands silence in for every presentation the unit could not be read
    /// for, with the fault beside it.
    pub(crate) fn conceal_failed(&mut self, unit: &[u8]) {
        let spa = self.samples_per_au;
        for k in 0..self.lanes.len() {
            if let Outcome::Failed(fault) = self.lanes[k].outcome {
                let channels = self.channels(k);
                let out = self.lanes[k].conceal_unit(unit, spa, channels);
                self.lanes[k].outcome = Outcome::Out(out, Some(fault));
            }
        }
    }

    /// A change of configuration stopped the decode: no unit is open.
    pub(crate) fn close_units(&mut self) {
        for lane in &mut self.lanes {
            lane.unit_open = false;
        }
    }

    /// Decodes one access unit and conceals what could not be read, for
    /// every presentation.
    pub(crate) fn decode_concealing(&mut self, unit: &[u8]) -> Result<()> {
        match self.decode_unit(unit, true) {
            Ok(()) => {
                self.conceal_failed(unit);
                Ok(())
            }
            Err(e) => {
                self.close_units();
                Err(e)
            }
        }
    }

    fn lend(&self, lane: usize, out: UnitOut) -> Decoded<'_> {
        let lane = &self.lanes[lane];
        Decoded {
            pcm: &lane.output[..out.len],
            channels: out.channels,
            duplicate: out.duplicate,
            concealed: out.concealed,
            output_delta: lane.output_delta,
        }
    }

    /// What presentation `lane` came to in the last unit. `None` when the
    /// unit left it without an outcome to lend: not decoded, or a fault not
    /// concealed. After [`Engine::decode_concealing`] has returned `Ok` every
    /// presentation has one; the callers make an error of `None` all the same.
    pub(crate) fn lent(&self, lane: usize) -> Option<Lent<'_>> {
        match self.lanes[lane].outcome {
            Outcome::Out(out, fault) => Some(Lent::Samples(
                self.lend(lane, out),
                fault.and_then(|k| self.faults.get(k)),
            )),
            Outcome::Gone => Some(Lent::Gone),
            Outcome::Pending | Outcome::Failed(_) => None,
        }
    }
}

/// What one presentation came to in an access unit, lent out.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Lent<'a> {
    /// Its samples, and the fault that was concealed in them.
    Samples(Decoded<'a>, Option<&'a Error>),
    /// The configuration in force does not carry it.
    Gone,
}

/// The error for a unit that left a presentation without an outcome, which
/// the decoders never do: said, not panicked on.
pub(crate) fn no_outcome(presentation: usize) -> Error {
    Error::malformed(format!(
        "presentation {presentation}: the access unit left it without an outcome"
    ))
}

/// Decoder for one presentation of a TrueHD stream.
#[derive(Debug, Clone)]
pub struct Decoder {
    engine: Engine,
}

impl Decoder {
    /// Creates a decoder for `presentation` (0..=3) of the stream described by
    /// its first major sync.
    pub fn new(major_sync: &MajorSync, presentation: usize) -> Result<Self> {
        Ok(Self {
            engine: Engine::new(major_sync, &[presentation])?,
        })
    }

    fn lane(&self) -> &Lane {
        &self.engine.lanes[0]
    }

    /// Keeps access units flagged as duplicates instead of marking them.
    pub fn keep_duplicates(&mut self, keep: bool) {
        self.engine.keep_duplicates(keep);
    }

    /// What to do at a major sync that changes the configuration;
    /// [`OnConfigChange::Stop`] unless set.
    pub fn on_config_change(&mut self, policy: OnConfigChange) {
        self.engine.on_config_change(policy);
    }

    /// The change of configuration the last access unit carried, if it
    /// carried one and the decoder followed it. Valid until the next access
    /// unit is decoded; readable after a unit that came back
    /// [`Error::PresentationGone`] too.
    #[must_use]
    pub fn config_change(&self) -> Option<&ConfigChange> {
        self.engine.config_change()
    }

    /// The requested presentation.
    #[must_use]
    pub fn presentation(&self) -> usize {
        self.lane().presentation
    }

    /// The stream configuration in force.
    #[must_use]
    pub fn config(&self) -> &StreamConfig {
        &self.engine.config
    }

    /// The presentation whose substream produces the output (differs from the
    /// requested one only when that is a copy).
    #[must_use]
    pub fn source_presentation(&self) -> usize {
        self.lane().top
    }

    /// Output channels of the presentation (known after the first access unit).
    #[must_use]
    pub fn channels(&self) -> usize {
        self.engine.channels(0)
    }

    /// Counters so far. Units in which a change of configuration had taken
    /// the presentation away are not counted.
    #[must_use]
    pub fn stats(&self) -> &DecodeStats {
        &self.lane().stats
    }

    /// The timing model (jumps, branches) so far.
    #[must_use]
    pub fn timing(&self) -> &TimingModel {
        &self.lane().timing
    }

    /// Decodes one access unit.
    pub fn decode(&mut self, unit: &[u8]) -> Result<Decoded<'_>> {
        self.engine.decode_unit(unit, false)?;
        match self.engine.lanes[0].outcome {
            Outcome::Out(out, None) => Ok(self.engine.lend(0, out)),
            Outcome::Out(_, Some(k)) | Outcome::Failed(k) => Err(self.take(k)),
            Outcome::Gone => Err(self.gone()),
            Outcome::Pending => Err(no_outcome(self.lane().presentation)),
        }
    }

    /// Decodes one access unit, or stands silence in for it.
    ///
    /// An access unit [`Decoder::decode`] cannot read becomes one unit of
    /// silence, and the substream that failed is read again from its next
    /// restart header -- with every substream after it, which missed this
    /// unit -- the presentation staying silent until all of them have
    /// restarted. Nothing before the fault and nothing after the restart
    /// changes: a restart header re-initialises everything its substream's
    /// decoding carries. The lossless check word of a section that took
    /// silence is skipped, not failed. Returns the fault that was concealed,
    /// if any; a change of configuration is not concealed but returned
    /// (under [`OnConfigChange::Stop`]), and so is a presentation that a
    /// change followed took away ([`Error::PresentationGone`]).
    pub fn decode_concealing(&mut self, unit: &[u8]) -> Result<(Decoded<'_>, Option<Error>)> {
        let (out, fault) = self.concealing_unit(unit)?;
        Ok((self.engine.lend(0, out), fault))
    }

    /// Decodes one access unit as [`Decoder::decode_concealing`] does, and
    /// lends the change of configuration it carries together with its
    /// samples. With [`OnConfigChange::Continue`] the unit that carries a
    /// change is decoded with the new configuration and `change` names it;
    /// with [`OnConfigChange::Stop`] the change is the error it always was.
    pub fn decode_following(&mut self, unit: &[u8]) -> Result<Following<'_>> {
        let (out, fault) = self.concealing_unit(unit)?;
        Ok(Following {
            decoded: self.engine.lend(0, out),
            fault,
            change: self.engine.config_change(),
        })
    }

    /// The body of [`Decoder::decode_concealing`], before the samples are lent.
    fn concealing_unit(&mut self, unit: &[u8]) -> Result<(UnitOut, Option<Error>)> {
        self.engine.decode_concealing(unit)?;
        match self.engine.lanes[0].outcome {
            Outcome::Out(out, fault) => Ok((out, fault.map(|k| self.take(k)))),
            Outcome::Gone => Err(self.gone()),
            Outcome::Failed(k) => Err(self.take(k)),
            Outcome::Pending => Err(no_outcome(self.lane().presentation)),
        }
    }

    /// Fault `k` of the unit, given away.
    fn take(&mut self, k: usize) -> Error {
        self.engine
            .faults
            .take(k)
            .unwrap_or_else(|| Error::malformed("a fault given away twice"))
    }

    fn gone(&self) -> Error {
        Error::PresentationGone {
            presentation: self.lane().presentation,
        }
    }

    /// The output timing jump at the restart header of the presentation's top
    /// substream in the last access unit, modulo 65 536 samples: zero when the
    /// header says what the units before it implied, the samples lost when a
    /// gap took whole units. `None` when that unit did not restart the
    /// substream, or it is the first restart.
    #[must_use]
    pub fn output_delta(&self) -> Option<u16> {
        self.lane().output_delta
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::FilterCoeffs;

    /// What a restart header sets of a substream, field by field. The
    /// pattern names every field, so a field added to the struct does not
    /// compile here until it is put on one side or the other.
    #[allow(clippy::type_complexity, reason = "every field, as it is")]
    fn set_by_restart(
        sd: &SubstreamDecoder,
    ) -> (
        (
            bool,
            u16,
            usize,
            usize,
            usize,
            [u8; MAX_CHANNELS],
            u8,
            u8,
            u16,
        ),
        (
            [[i32; FILTER_ORDER]; MAX_CHANNELS],
            [[i32; FILTER_ORDER]; MAX_CHANNELS],
            [[i32; REMATRIX_COLUMNS]; MAX_MATRICES],
            [[i32; MAX_CHANNELS]; MAX_MATRICES],
        ),
    ) {
        let SubstreamDecoder {
            active,
            sync_word,
            min_chan,
            max_chan,
            max_matrix_chan,
            ch_assign,
            dither_shift,
            max_bits,
            output_timing,
            fir_hist,
            iir_hist,
            m,
            d,
            // The unit in progress, which a restart header does not touch:
            // `decoded` and `zero_samples` are set to nought at the start of
            // every unit before the substream is read, and `recorrelated`
            // is written before it is read except where a substream
            // delivers fewer samples than the unit (`Engine::follow`).
            recorrelated: _,
            decoded: _,
            zero_samples: _,
        } = sd;
        (
            (
                *active,
                *sync_word,
                *min_chan,
                *max_chan,
                *max_matrix_chan,
                *ch_assign,
                *dither_shift,
                *max_bits,
                *output_timing,
            ),
            (*fir_hist, *iir_hist, *m, *d),
        )
    }

    /// A substream decoder with every field away from what the restart
    /// header of the test sets it to.
    fn worn() -> SubstreamDecoder {
        SubstreamDecoder {
            active: false,
            sync_word: SYNC_C,
            min_chan: 3,
            max_chan: 9,
            max_matrix_chan: 11,
            ch_assign: core::array::from_fn(|i| (MAX_CHANNELS - 1 - i) as u8),
            dither_shift: 7,
            max_bits: 5,
            output_timing: 999,
            fir_hist: [[7; FILTER_ORDER]; MAX_CHANNELS],
            iir_hist: [[-3; FILTER_ORDER]; MAX_CHANNELS],
            recorrelated: Box::new([[5; MAX_CHANNELS]; MAX_SAMPLES_PER_AU]),
            m: [[9; REMATRIX_COLUMNS]; MAX_MATRICES],
            d: [[-9; MAX_CHANNELS]; MAX_MATRICES],
            decoded: 17,
            zero_samples: 4,
        }
    }

    /// A restart header leaves nothing of what a substream carried before
    /// it: a decoder that has run and a fresh one come out of the same
    /// header the same, every field but the samples of the unit in progress.
    #[test]
    fn a_restart_header_sets_everything_a_substream_carries() {
        let mut ss = SubstreamState {
            sync_word: SYNC_B,
            restart_seen: true,
            min_chan: 2,
            max_chan: 5,
            max_matrix_chan: 5,
            dither_shift: 3,
            max_bits: 20,
            ..SubstreamState::default()
        };
        ss.ch_assign = core::array::from_fn(|i| i as u8);
        ss.ch_assign[0] = 1;
        ss.ch_assign[1] = 0;
        let mut used = worn();
        used.restart(&ss, 77);
        let mut fresh = SubstreamDecoder::default();
        fresh.restart(&ss, 77);
        assert_eq!(set_by_restart(&used), set_by_restart(&fresh));
        // and the worn one was worn in every field the header sets
        let worn = worn();
        let (a, b) = (set_by_restart(&worn), set_by_restart(&fresh));
        assert_ne!(a.0.0, b.0.0);
        assert_ne!(a.0.1, b.0.1);
        assert_ne!(a.0.2, b.0.2);
        assert_ne!(a.0.3, b.0.3);
        assert_ne!(a.0.4, b.0.4);
        assert_ne!(a.0.5, b.0.5);
        assert_ne!(a.0.6, b.0.6);
        assert_ne!(a.0.7, b.0.7);
        assert_ne!(a.0.8, b.0.8);
        assert_ne!(a.1.0, b.1.0);
        assert_ne!(a.1.1, b.1.1);
        assert_ne!(a.1.2, b.1.2);
        assert_ne!(a.1.3, b.1.3);
    }

    fn fixture_units(name: &str) -> Vec<Vec<u8>> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../oadec-cli/tests/fixtures")
            .join(name);
        let bytes = std::fs::read(&path).unwrap();
        let mut extractor = crate::extract::Extractor::new();
        extractor.push(&bytes);
        let mut units = Vec::new();
        while let Some(unit) = extractor.next_unit().unwrap() {
            units.push(unit.bytes.to_vec());
        }
        units.extend(
            extractor
                .finish()
                .unwrap()
                .0
                .into_iter()
                .map(|u| u.bytes.to_vec()),
        );
        units
    }

    /// Following a change leaves no substream state of the configuration
    /// before it: every substream decoder is a fresh one, the samples of the
    /// unit in progress included.
    #[test]
    fn a_change_followed_starts_every_substream_afresh() {
        let before = fixture_units("truehd-51-88k.thd");
        let after = fixture_units("truehd-20-44k.thd");
        let (first, _) = AccessUnit::parse(&before[0], None).unwrap();
        let mut decoder = Decoder::new(first.major_sync.as_ref().unwrap(), 0).unwrap();
        for unit in &before[..20] {
            decoder.decode(unit).unwrap();
        }
        let engine = &mut decoder.engine;
        assert!(engine.sub.iter().any(|sd| sd.active && sd.decoded > 0));
        let (au, config) = AccessUnit::parse(&after[0], Some(&engine.config)).unwrap();
        let what = engine.config.incompatible_with(&config).unwrap();
        engine.follow(au.major_sync.as_ref().unwrap(), config, what, 20);
        let fresh = SubstreamDecoder::default();
        for (i, sd) in engine.sub.iter().enumerate() {
            assert_eq!(set_by_restart(sd), set_by_restart(&fresh), "substream {i}");
            assert!(sd.recorrelated == fresh.recorrelated, "substream {i}");
            assert_eq!((sd.decoded, sd.zero_samples), (0, 0), "substream {i}");
        }
    }

    #[test]
    fn lossless_fold_matches_the_reference_definition() {
        assert_eq!(fold_lossless(0), 0);
        assert_eq!(fold_lossless(0x0000_00AB), 0xAB);
        assert_eq!(fold_lossless(0x1234_5678), 0x12 ^ 0x34 ^ 0x56 ^ 0x78);
    }

    /// The five extra-data faults `verify` counts, one at a time, counted by
    /// the decoder with the same checks, and the first one named.
    #[test]
    fn extra_data_faults_are_counted_as_verify_counts_them() {
        fn counts(s: &DecodeStats) -> [u64; 5] {
            [
                s.extra_header_parity,
                s.extra_padding_nonzero,
                s.extra_truncated,
                s.extra_evolution_parity,
                s.evolution_container_errors,
            ]
        }
        let clean = ExtraData {
            header_nibble: 0,
            length_words: 0,
            header_parity_ok: true,
            kind: ExtraKind::Padding,
            parity_ok: None,
            padding_zero: true,
            trailing_bytes: 0,
        };
        let evolution = |frame: Vec<u8>, parity_ok| ExtraData {
            kind: ExtraKind::Evolution { reserved: 0, frame },
            parity_ok: Some(parity_ok),
            ..clean.clone()
        };

        let mut s = DecodeStats::default();
        s.take_extra(0, &clean);
        // an empty frame is not parsed, and a good parity is not a fault
        s.take_extra(1, &evolution(Vec::new(), true));
        assert_eq!(counts(&s), [0; 5]);
        assert_eq!(s.first_problem, None);

        let faults = [
            ExtraData {
                header_parity_ok: false,
                ..clean.clone()
            },
            ExtraData {
                padding_zero: false,
                ..clean.clone()
            },
            ExtraData {
                kind: ExtraKind::Truncated,
                ..clean.clone()
            },
            evolution(Vec::new(), false),
            // version and key id, then a payload id that runs out of bits
            evolution(vec![0x00], true),
        ];
        for (k, extra) in faults.iter().enumerate() {
            let mut s = DecodeStats::default();
            s.take_extra(7, extra);
            let mut expected = [0; 5];
            expected[k] = 1;
            assert_eq!(counts(&s), expected, "fault {k}");
            assert!(
                s.first_problem
                    .as_deref()
                    .is_some_and(|p| p.starts_with("access unit 7: ")),
                "fault {k} is named: {:?}",
                s.first_problem
            );
        }
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
            state_bits: 0,
            state_shift: 0,
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
            state_bits: 0,
            state_shift: 0,
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
            state_bits: 0,
            state_shift: 0,
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

    /// A matrix moves when one of the deltas the stage reads is set, and
    /// only that matrix: a delta beyond the channels the matrices take, or
    /// of a matrix beyond the ones in use, is read by nothing.
    #[test]
    fn a_matrix_moves_by_the_deltas_that_are_read() {
        let still = [[0i32; MAX_CHANNELS]; MAX_MATRICES];
        for mmc in 0..MAX_CHANNELS {
            for matrices in 0..=MAX_MATRICES {
                assert_eq!(
                    moving_matrices(&still, matrices, mmc),
                    [false; MAX_MATRICES]
                );
                for pmi in 0..MAX_MATRICES {
                    for ch in 0..MAX_CHANNELS {
                        for delta in [1, -1, i32::MIN, i32::MAX] {
                            let mut d = still;
                            d[pmi][ch] = delta;
                            let mut expected = [false; MAX_MATRICES];
                            expected[pmi] = pmi < matrices && ch <= mmc;
                            assert_eq!(
                                moving_matrices(&d, matrices, mmc),
                                expected,
                                "a delta of matrix {pmi} at channel {ch}, {matrices} matrices over 0..={mmc}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn matrix_output_rounds_cuts_and_adds_bypassed_bits() {
        assert_eq!(matrix_output(5 << 18, 0, 0, false), 5);
        assert_eq!(matrix_output((5 << 18) + 1, 0, 0, false), 5);
        assert_eq!(matrix_output(-1, 0, 0, false), -1);
        // cut to a quantiser step of two bits
        assert_eq!(matrix_output(0b10111 << 18, 2, 0, false), 0b10100);
        // cut, and then the bits sent apart: the two kinds of substream
        // where nothing else has been measured
        assert_eq!(matrix_output(0b10111 << 18, 2, 0b01, false), 0b10101);
    }

    /// A matrix of a 0x31EA substream that sends a bypassed bit is not cut to
    /// the quantiser step: the sum as it is, and the bit. With the bit clear
    /// too, which is what tells "not cut" from "the bit put where the cut
    /// left room".
    #[test]
    fn a_matrix_that_sends_a_bypassed_bit_is_not_cut_to_the_step() {
        assert_eq!(matrix_output(0b10111 << 18, 2, 0b01, true), 0b11000);
        assert_eq!(matrix_output(0b10111 << 18, 2, 0, true), 0b10111);
        assert_eq!(matrix_output(0b10100 << 18, 2, 0b01, true), 0b10101);
        // without a step there is nothing to cut either way
        assert_eq!(
            matrix_output(0b10111 << 18, 0, 0b01, true),
            matrix_output(0b10111 << 18, 0, 0b01, false)
        );
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
