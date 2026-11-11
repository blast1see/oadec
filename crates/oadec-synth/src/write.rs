//! The writer: access units from samples and a plan.
//!
//! This is the syntax and the arithmetic of `docs/truehd.md` written from the
//! encoding side. It shares no code with the decoder. A stream it writes
//! carries a substream of one or two channels, which is the whole of a
//! two-channel presentation, and every element of the block syntax under
//! that: both prediction filters with every order, coded filter states, the
//! three code books and plain coding, the offset, the quantiser step, several
//! blocks to a unit; and over the channels the matrices of a `0x31EA`
//! substream with their two noise channels and bypassed LSBs, the output
//! shifts and the channel assignment. Over that substream it may carry one
//! or two more, which bring the channels to six and to eight: each codes the
//! channels it adds and has matrices of its own over every channel so far,
//! with the table of dither of a `0x31EB` substream or with noise channels
//! again.
//!
//! Lossless by construction, from the output backwards. The output shift is
//! taken off the samples, then each matrix is undone, the last first: a
//! matrix whose coefficient on the channel it writes is 1 or -1 loses
//! nothing. What is left are the samples of the coded channels, and the
//! residual of each is what remains of it when the prediction the decoder
//! will make is taken off. The writer runs the decoder's recursions to know
//! what they will give; it does not search for good filters or matrices, it
//! is told them.
//!
//! With several substreams that is how the top one is written: its
//! presentation is the samples the stream is made from. The substreams under
//! it code channels the top one has settled by then, so their presentations
//! are not free. Each is what its own matrices make of those channels; the
//! writer works that out forwards, as a decoder does, and hands it back with
//! the stream. Such a matrix is never undone, and may weigh the channel it
//! writes by anything.

use oadec_bits::{CRC8_RESTART, CRC8_SUBSTREAM, CRC8_SUBSTREAM_INIT, CRC16_MAJOR_SYNC};
use oadec_bits::{fold_nibble, xor_bytes};

use crate::bits::BitWriter;

/// The sampling rates of the format: the 48 kHz family and the 44.1 kHz one.
pub const RATES: [u32; 6] = [44_100, 48_000, 88_200, 96_000, 176_400, 192_000];

/// The most taps the two filters of a channel have between them.
pub const MAX_TAPS: usize = 8;

/// The restart sync word of a substream with MLP-style noise channels: the
/// one the substream of a two-channel presentation carries.
const RESTART_SYNC_NOISE: u64 = 0x31EA;
/// The restart sync word of a substream with a table of dither in their
/// place: the one encoders give the substreams over the first.
const RESTART_SYNC_DITHER: u64 = 0x31EB;
/// The dither a `0x31EB` substream draws on: 256 values that are part of the
/// format, named by the top eight bits of the noise register. The two public
/// decoders carry the same table (`noise_table` in FFmpeg, `DITHER_LUT` in
/// truehdd).
#[rustfmt::skip]
pub const DITHER: [i8; 256] = [
     30,  51,  22,  54,   3,   7,  -4,  38,  14,  55,  46,  81,  22,  58,  -3,   2,
     52,  31,  -7,  51,  15,  44,  74,  30,  85, -17,  10,  33,  18,  80,  28,  62,
     10,  32,  23,  69,  72,  26,  35,  17,  73,  60,   8,  56,   2,   6,  -2,  -5,
     51,   4,  11,  50,  66,  76,  21,  44,  33,  47,   1,  26,  64,  48,  57,  40,
     38,  16, -10, -28,  92,  22, -18,  29, -10,   5, -13,  49,  19,  24,  70,  34,
     61,  48,  30,  14,  -6,  25,  58,  33,  42,  60,  67,  17,  54,  17,  22,  30,
     67,  44,  -9,  50, -11,  43,  40,  32,  59,  82,  13,  49, -14,  55,  60,  36,
     48,  49,  31,  47,  15,  12,   4,  65,   1,  23,  29,  39,  45,  -2,  84,  69,
      0,  72,  37,  57,  27,  41, -15, -16,  35,  31,  14,  61,  24,   0,  27,  24,
     16,  41,  55,  34,  53,   9,  56,  12,  25,  29,  53,   5,  20, -20,  -8,  20,
     13,  28,  -3,  78,  38,  16,  11,  62,  46,  29,  21,  24,  46,  65,  43, -23,
     89,  18,  74,  21,  38, -12,  19,  12, -19,   8,  15,  33,   4,  57,   9,  -8,
     36,  35,  26,  28,   7,  83,  63,  79,  75,  11,   3,  87,  37,  47,  34,  40,
     39,  19,  20,  42,  27,  34,  39,  77,  13,  42,  59,  64,  45,  -1,  32,  37,
     45,  -5,  53,  -6,   7,  36,  50,  23,   6,  32,   9, -21,  18,  71,  27,  52,
    -25,  31,  35,  42,  -1,  68,  63,  52,  26,  43,  66,  37,  41,  25,  40,  70,
];
/// Eighteen bits that end a stream, and the thirteen that follow them when no
/// count of padding samples does.
const TERMINATOR: u64 = 0x348D3;
const TERMINATOR_TAIL: u64 = 0x1234;
/// What is XORed into the parity byte that closes a segment.
const PARITY_XOR: u8 = 0xA9;
/// `huff_lsbs` after a restart header, and the most a channel may ask for.
const DEFAULT_LSBS: u8 = 24;
/// The guard bits: bit 0 for the guards themselves, then the offset, filter
/// B, filter A, the quantiser steps, the output shifts, the matrices and the
/// block size.
const GUARD_GUARDS: u8 = 1 << 0;
const GUARD_OFFSET: u8 = 1 << 1;
const GUARD_FILTER_B: u8 = 1 << 2;
const GUARD_FILTER_A: u8 = 1 << 3;
const GUARD_STEPS: u8 = 1 << 4;
const GUARD_SHIFTS: u8 = 1 << 5;
const GUARD_MATRICES: u8 = 1 << 6;
const GUARD_BLOCK_SIZE: u8 = 1 << 7;
/// The block size in force after a restart header.
const DEFAULT_BLOCK: usize = 8;
/// One past the largest sample, and the largest residual, of 24 bits.
const LIMIT: i64 = 1 << 23;

/// Which of the two syntaxes the major sync announces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// TrueHD: major sync `F8726FBA`.
    TrueHd,
    /// MLP as DVD-Audio carries it: major sync `F8726FBB`.
    Mlp,
}

/// A prediction filter as it is sent.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Filter {
    /// Fractional bits of the coefficients (`coeff_q`), 8 to 15.
    pub precision: u8,
    /// Left shift the decoder applies to every coefficient it reads.
    pub shift: u8,
    /// The coefficients as coded, before the shift. None is no filter.
    pub coefficients: Vec<i16>,
    /// The state an IIR filter is given: the shift the decoder applies and
    /// the values as coded, one for each tap. `None` sends no state.
    pub state: Option<(u8, Vec<i32>)>,
}

impl Filter {
    /// No filter.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// A filter of the given coefficients at `precision` fractional bits.
    #[must_use]
    pub fn new(precision: u8, coefficients: &[i16]) -> Self {
        Self {
            precision,
            shift: 0,
            coefficients: coefficients.to_vec(),
            state: None,
        }
    }

    /// Taps.
    #[must_use]
    pub fn order(&self) -> usize {
        self.coefficients.len()
    }

    /// The coefficients the decoder ends up with.
    fn scaled(&self) -> [i64; MAX_TAPS] {
        let mut out = [0; MAX_TAPS];
        for (slot, &c) in out.iter_mut().zip(&self.coefficients) {
            *slot = i64::from(c) << self.shift;
        }
        out
    }
}

/// How one channel is coded.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Coding {
    /// Filter A, on the past outputs: up to eight taps.
    pub fir: Filter,
    /// Filter B, on the past differences between output and prediction: up
    /// to four taps, and may be given a state.
    pub iir: Filter,
    /// The code book, 1 to 3, or 0 for none (`huff_type`).
    pub book: u8,
    /// `huff_offset`.
    pub offset: i16,
    /// `quant_step_size`: the low bits every sample of the channel leaves
    /// clear and the stream does not spend bits on.
    pub step: u8,
    /// `huff_lsbs`, or `None` for the least that holds the block.
    pub lsbs: Option<u8>,
}

/// What a stream is, apart from its samples.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// The syntax.
    pub family: Family,
    /// Sampling rate, one of [`RATES`].
    pub rate: u32,
    /// Channels of the first substream, 1 or 2: the two-channel
    /// presentation.
    pub channels: usize,
    /// Access units from one major sync to the next.
    pub sync_interval: usize,
    /// A parity byte and a CRC close every segment (`crc_present`).
    pub trailer: bool,
    /// Left shift of the two noise channels the matrices may draw on
    /// (`dither_shift`), 0 to 15.
    pub noise_shift: u8,
    /// The 23-bit register the noise channels are drawn from, as each restart
    /// header in turn states it. Where the list ends a restart header states
    /// the register as it stands, which is what an encoder that never stops
    /// writes; an empty list starts it at 0, where it stays and the noise is
    /// silence.
    pub noise_seeds: Vec<u32>,
    /// Which output channel each coded channel is (`ch_assign`); empty for
    /// each its own.
    pub assignment: Vec<usize>,
    /// The restart headers set `error_protect`, and every block states the
    /// size of its data in front of it and a check byte behind.
    pub protection: Option<Protection>,
    /// The major sync announces heavy dynamic range control (`flags` bit 13)
    /// and every restart header carries its update: a gain in 9 bits of
    /// two's complement and a time in 3 bits.
    pub heavy_drc: Option<(i16, u8)>,
    /// The substreams over the first, the lowest first: one brings the
    /// channels to six, a second to eight. What the fields above say of
    /// noise, assignment and protection is said of the first substream; each
    /// of these says its own.
    pub above: Vec<Above>,
}

/// A substream over the first. It codes the channels it adds to those of the
/// substreams under it, and its matrices, output shifts and channel
/// assignment are over all of them: its presentation is every channel so
/// far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Above {
    /// Channels it codes: four in the second substream, two in the third.
    pub channels: usize,
    /// Its restart sync word is `0x31EB`, as encoders write it: no noise
    /// channels, and a table of dither a matrix may draw on. Without, it is
    /// `0x31EA` and has the noise channels of the first substream.
    pub dither: bool,
    /// `dither_shift`: the left shift of the noise channels. A `0x31EB`
    /// substream states it and has no use for it.
    pub noise_shift: u8,
    /// The 23-bit register its noise or its dither is drawn from, as each of
    /// its restart headers in turn states it; read as [`Layout::noise_seeds`]
    /// is.
    pub noise_seeds: Vec<u32>,
    /// Which output channel each channel of its presentation is; empty for
    /// each its own.
    pub assignment: Vec<usize>,
    /// Its restart headers set `error_protect`.
    pub protection: Option<Protection>,
}

impl Above {
    /// A substream of `channels` channels with a table of dither, the
    /// register starting at 0, each channel its own output.
    #[must_use]
    pub fn new(channels: usize) -> Self {
        Self {
            channels,
            dither: true,
            noise_shift: 0,
            noise_seeds: Vec::new(),
            assignment: Vec::new(),
            protection: None,
        }
    }
}

/// What a protected block states of itself. Neither field has a public
/// definition: the description names them and says no more.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Protection {
    /// What `block_data_bits` counts beyond the bits of the block data: 0
    /// for those alone, 16 with the field itself.
    pub counted_beyond_the_data: u16,
    /// `block_header_CRC`, as it is written: how it is computed is not
    /// known.
    pub check: u8,
}

impl Layout {
    /// A layout with a major sync every eight access units, the check bytes
    /// of every segment, no noise and each channel its own output.
    #[must_use]
    pub fn new(family: Family, rate: u32, channels: usize) -> Self {
        Self {
            family,
            rate,
            channels,
            sync_interval: 8,
            trailer: true,
            noise_shift: 0,
            noise_seeds: Vec::new(),
            assignment: Vec::new(),
            protection: None,
            heavy_drc: None,
            above: Vec::new(),
        }
    }

    /// Channels of the widest presentation: those of every substream
    /// together, which is how many the samples of a stream have.
    #[must_use]
    pub fn total_channels(&self) -> usize {
        self.channels + self.above.iter().map(|above| above.channels).sum::<usize>()
    }

    /// Samples in an access unit: 40 at the base rates, 80 and 160 above.
    #[must_use]
    pub fn samples_per_unit(&self) -> usize {
        40 * (self.rate / 44_100) as usize
    }

    fn rate_code(&self) -> u64 {
        match self.rate {
            48_000 => 0,
            96_000 => 1,
            192_000 => 2,
            44_100 => 8,
            88_200 => 9,
            _ => 10,
        }
    }
}

/// A primitive matrix: one channel replaced by a weighted sum of them all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Matrix {
    /// The channel the matrix writes (`matrix_ch`).
    pub out: usize,
    /// Fractional bits of the coefficients as they are sent (`frac_bits`), 0
    /// to 14.
    pub frac_bits: u8,
    /// A coefficient for each channel the substream's matrices are over, and
    /// in a `0x31EA` substream for the two noise channels after them, in
    /// units of `2^-frac_bits`. In the top substream the one on the channel
    /// the matrix writes is 1 or -1: a matrix can be undone exactly with no
    /// other. Under the top nothing is undone and it may be anything.
    pub coefficients: Vec<i32>,
    /// One more bit a sample, sent as it is and added to the sum, which is
    /// then not cut to a quantiser step (`lsb_bypass_used`).
    pub bypass: bool,
    /// `dither_scale` of a matrix in a `0x31EB` substream: 0 for no dither,
    /// or how far over 11 bits a value of the dither table is shifted up
    /// into the sum, whose fractional bits are 18.
    pub dither: u8,
}

impl Matrix {
    /// A matrix that adds to channel `out` the other channels and the noise
    /// by `weights`, in units of `2^-frac_bits`; the weight given for `out`
    /// itself only says whether the channel is kept (not negative) or turned
    /// over (negative).
    #[must_use]
    pub fn new(out: usize, frac_bits: u8, weights: &[i32]) -> Self {
        let mut coefficients = weights.to_vec();
        if let Some(own) = coefficients.get_mut(out) {
            *own = if *own < 0 { -1 } else { 1 } << frac_bits;
        }
        Self {
            out,
            frac_bits,
            coefficients,
            bypass: false,
            dither: 0,
        }
    }
}

/// One block of a substream: so many samples of every channel it codes, and
/// what codes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockPlan {
    /// Samples, 8 at least.
    pub size: usize,
    /// The coding of each channel the substream codes from this block on. A
    /// block says of it only what differs from the block before.
    pub coding: Vec<Coding>,
    /// The matrices from this block on, in the order the decoder applies
    /// them. All are sent again when any differs.
    pub matrices: Vec<Matrix>,
    /// The left shift of each output from this block on (`output_shift`), -8
    /// to 7; empty for none.
    pub shifts: Vec<i8>,
    /// The guard bits this block sends, which say from here on which of the
    /// fields of a block header a flag is spent on; `None` sends none and
    /// leaves them as they are, all set after a restart header.
    pub guards: Option<u8>,
    /// The quantiser steps a block of a substream over the first states for
    /// the channels under its own, which it does not code but its matrices
    /// may write. Empty for what the substream that codes each channel has
    /// in force at the first sample of the block, which is what encoders
    /// write.
    pub steps_below: Vec<u8>,
}

/// The blocks of one access unit: for each substream a run of them whose
/// sizes add up to the samples of a unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitPlan {
    /// The blocks of the first substream, in order.
    pub blocks: Vec<BlockPlan>,
    /// The blocks of each substream over the first.
    pub above: Vec<Vec<BlockPlan>>,
}

impl UnitPlan {
    /// A unit of one substream in blocks of these sizes, all coded alike.
    #[must_use]
    pub fn uniform(sizes: &[usize], coding: &[Coding]) -> Self {
        Self {
            blocks: sizes
                .iter()
                .map(|&size| BlockPlan {
                    size,
                    coding: coding.to_vec(),
                    matrices: Vec::new(),
                    shifts: Vec::new(),
                    guards: None,
                    steps_below: Vec::new(),
                })
                .collect(),
            above: Vec::new(),
        }
    }

    /// The unit with the blocks of `next` for one more substream over the
    /// ones it has, and for any that `next` has over its own first.
    #[must_use]
    pub fn over(mut self, next: Self) -> Self {
        self.above.push(next.blocks);
        self.above.extend(next.above);
        self
    }

    /// The unit with the first block of its first substream sending these
    /// guard bits.
    #[must_use]
    pub fn guarded(mut self, guards: u8) -> Self {
        if let Some(first) = self.blocks.first_mut() {
            first.guards = Some(guards);
        }
        self
    }

    /// The unit with these matrices in force in every block of its first
    /// substream. The blocks of a substream over the first are given theirs
    /// before [`UnitPlan::over`] puts them there.
    #[must_use]
    pub fn matrixed(mut self, matrices: &[Matrix]) -> Self {
        for block in &mut self.blocks {
            block.matrices = matrices.to_vec();
        }
        self
    }

    /// The unit with these output shifts in force in every block of its
    /// first substream.
    #[must_use]
    pub fn shifted(mut self, shifts: &[i8]) -> Self {
        for block in &mut self.blocks {
            block.shifts = shifts.to_vec();
        }
        self
    }
}

/// A written stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stream {
    /// The elementary stream.
    pub bytes: Vec<u8>,
    /// Where each access unit starts.
    pub unit_offsets: Vec<usize>,
    /// The presentation of each substream under the top one, the first
    /// substream's first: the samples of each of its output channels, as its
    /// matrices make them of the channels the stream codes. Empty for a
    /// stream of one substream, whose presentation is the samples it was
    /// written from.
    pub below: Vec<Vec<Vec<i32>>>,
}

/// The fewest bits of two's complement that hold every value; 1 for none.
fn signed_bits(values: impl Iterator<Item = i64>) -> u32 {
    let mut bits = 1;
    for v in values {
        let magnitude = if v < 0 { !v } else { v } as u64;
        bits = bits.max(65 - magnitude.leading_zeros());
    }
    bits
}

/// The values a code book has codes for, lowest and highest; without a book
/// the code is no bits and stands for 0.
fn book_range(book: u8) -> (i64, i64) {
    match book {
        0 => (0, 0),
        1 => (-7, 10),
        2 => (-7, 8),
        _ => (-7, 7),
    }
}

/// What the decoder takes off a coded value for its sign: half the range of
/// the LSBs without a book, and with one a power of two that puts the middle
/// of the book's range on zero.
fn sign_bias(book: u8, lsb_bits: u32) -> i64 {
    if book == 0 {
        if lsb_bits > 0 { 1 << (lsb_bits - 1) } else { 0 }
    } else {
        let shift = lsb_bits as i32 + 2 - i32::from(book);
        if shift < 0 { 0 } else { 1 << shift }
    }
}

/// Writes the code of `value` in `book`: a leading 1 and the value for the
/// centre group; `00` or `01` and a run of zeros ended by a 1 for the
/// negative and the positive chain.
fn put_code(w: &mut BitWriter, book: u8, value: i64) {
    let (centre_bits, positive_base) = match book {
        1 => (2, 4),
        2 => (1, 2),
        _ => (0, 1),
    };
    if (0..positive_base).contains(&value) {
        w.put(1, 1);
        w.put(centre_bits, value as u64);
    } else if value < 0 {
        w.put(2, 0b00);
        w.put((-value - 1) as u32, 0);
        w.put(1, 1);
    } else {
        w.put(2, 0b01);
        w.put((value - positive_base) as u32, 0);
        w.put(1, 1);
    }
}

fn put_filter(w: &mut BitWriter, filter: &Filter, iir: bool) -> Result<(), String> {
    let most = if iir { 4 } else { MAX_TAPS };
    if filter.order() > most {
        return Err(format!(
            "a filter of {} taps where {most} are the most",
            filter.order()
        ));
    }
    w.put(4, filter.order() as u64);
    if filter.order() == 0 {
        return Ok(());
    }
    if !(8..=15).contains(&filter.precision) {
        return Err(format!("coefficient precision {}", filter.precision));
    }
    let bits = signed_bits(filter.coefficients.iter().map(|&c| i64::from(c)));
    if bits + u32::from(filter.shift) > 16 {
        return Err(format!(
            "coefficients of {bits} bits shifted by {} do not fit in 16",
            filter.shift
        ));
    }
    w.put(4, u64::from(filter.precision));
    w.put(5, u64::from(bits));
    w.put(3, u64::from(filter.shift));
    for &c in &filter.coefficients {
        if i64::from(c) << filter.shift == -32768 {
            return Err("a coefficient of -32768 is not allowed".into());
        }
        w.put_signed(bits, i64::from(c));
    }
    match &filter.state {
        None => w.put_bool(false),
        Some(_) if !iir => return Err("filter A carries no state".into()),
        Some((shift, values)) => {
            if values.len() != filter.order() {
                return Err("a state has one value for each tap".into());
            }
            let bits = if values.iter().all(|&v| v == 0) {
                0
            } else {
                signed_bits(values.iter().map(|&v| i64::from(v)))
            };
            if bits > 15 || *shift > 15 {
                return Err(format!("a state of {bits} bits shifted by {shift}"));
            }
            w.put_bool(true);
            w.put(4, u64::from(bits));
            w.put(4, u64::from(*shift));
            if bits > 0 {
                for &v in values {
                    w.put_signed(bits, i64::from(v));
                }
            }
        }
    }
    Ok(())
}

/// What the decoder holds for one channel, kept in step with it.
#[derive(Debug, Clone)]
struct Channel {
    /// The coding the decoder was last told; its `lsbs` is not used.
    held: Coding,
    /// `huff_lsbs` in force.
    lsbs: u8,
    /// Past outputs, the latest first.
    fir_history: [i64; MAX_TAPS],
    /// Past differences between output and prediction, the latest first.
    iir_history: [i64; MAX_TAPS],
}

impl Channel {
    /// The state a restart header leaves: no filter, no book, no offset, no
    /// step, 24 LSBs, empty histories.
    fn after_restart() -> Self {
        Self {
            held: Coding::default(),
            lsbs: DEFAULT_LSBS,
            fir_history: [0; MAX_TAPS],
            iir_history: [0; MAX_TAPS],
        }
    }

    /// The residuals of `samples` under the coding in force, moving the
    /// histories on as the decoder will.
    fn residuals(&mut self, samples: &[i32]) -> Result<Vec<i64>, String> {
        let fir = self.held.fir.scaled();
        let iir = self.held.iir.scaled();
        let (fir_order, iir_order) = (self.held.fir.order(), self.held.iir.order());
        let shift = if fir_order != 0 {
            self.held.fir.precision
        } else {
            self.held.iir.precision
        };
        let mask = !((1i64 << self.held.step) - 1);
        let mut out = Vec::with_capacity(samples.len());
        for &sample in samples {
            let sample = i64::from(sample);
            if sample & !mask != 0 {
                return Err(format!(
                    "sample {sample} has bits under a quantiser step of {}",
                    self.held.step
                ));
            }
            let taps = |coefficients: &[i64], history: &[i64]| -> i64 {
                coefficients.iter().zip(history).map(|(c, h)| c * h).sum()
            };
            let acc = taps(&fir[..fir_order], &self.fir_history)
                + taps(&iir[..iir_order], &self.iir_history);
            let prediction = acc >> shift;
            let difference = sample - prediction;
            let residual = sample - (prediction & mask);
            for (what, value) in [
                ("sample", sample),
                ("difference from the prediction", difference),
                ("residual", residual),
            ] {
                if !(-LIMIT..LIMIT).contains(&value) {
                    return Err(format!("a {what} of {value} leaves 24 bits"));
                }
            }
            out.push(residual);
            self.fir_history.copy_within(0..MAX_TAPS - 1, 1);
            self.fir_history[0] = sample;
            self.iir_history.copy_within(0..MAX_TAPS - 1, 1);
            self.iir_history[0] = difference;
        }
        Ok(out)
    }
}

/// The least `huff_lsbs` under which every residual has a code, or the one
/// asked for if they all do.
fn choose_lsbs(coding: &Coding, residuals: &[i64]) -> Result<u8, String> {
    let (low, high) = book_range(coding.book);
    let fits = |lsb_bits: u32| {
        let bias = sign_bias(coding.book, lsb_bits) - i64::from(coding.offset);
        residuals.iter().all(|&r| {
            let code = ((r >> coding.step) + bias) >> lsb_bits;
            (low..=high).contains(&code)
        })
    };
    let step = u32::from(coding.step);
    let most = u32::from(DEFAULT_LSBS);
    if step > most {
        return Err(format!("a quantiser step of {step}"));
    }
    if let Some(lsbs) = coding.lsbs {
        let lsbs = u32::from(lsbs);
        if lsbs < step || lsbs > most || !fits(lsbs - step) {
            return Err(format!("the block does not fit in huff_lsbs {lsbs}"));
        }
        return Ok(lsbs as u8);
    }
    (0..=most - step)
        .find(|&lsb_bits| fits(lsb_bits))
        .map(|lsb_bits| (lsb_bits + step) as u8)
        .ok_or_else(|| "the block fits in no huff_lsbs up to 24".to_string())
}

/// The check word of one sample across the channels, as the decoder XORs it:
/// the outputs in the order of the coded channels they come from.
fn check_word(samples: impl Iterator<Item = i32>) -> u32 {
    samples.enumerate().fold(0, |check, (ch, v)| {
        check ^ ((v as u32 & 0x00FF_FFFF) << (ch & 7))
    })
}

/// The eight bits of a check word that a restart header carries.
fn fold_check(word: u32) -> u8 {
    let x = word ^ (word >> 16);
    (x ^ (x >> 8)) as u8
}

fn major_sync(layout: &Layout, peak_data_rate: u64) -> Vec<u8> {
    let mut w = BitWriter::new();
    match layout.family {
        Family::TrueHd => {
            w.put(32, 0xF872_6FBA);
            w.put(4, layout.rate_code());
            // neither multichannel type, two reserved bits
            w.put(4, 0);
            // The two-channel presentation is plain stereo (modifier 0), and
            // nothing is indicated by the modifiers of the other two. With
            // one substream those are copies of it: an assignment of left
            // and right. A second substream makes the six-channel one left,
            // right, centre, LFE and the two surrounds, of which the
            // eight-channel one is a copy until a third adds the two back
            // channels to it.
            let (six, eight) = match layout.above.len() {
                0 => (0x01, 0x0001),
                1 => (0x0F, 0x000F),
                _ => (0x0F, 0x004F),
            };
            w.put(2, 0);
            w.put(2, 0);
            w.put(5, six);
            w.put(2, 0);
            w.put(13, eight);
        }
        Family::Mlp => {
            w.put(32, 0xF872_6FBB);
            // group 1 at 24 bits and the rate of the stream, no group 2
            w.put(4, 2);
            w.put(4, 0xF);
            w.put(4, layout.rate_code());
            w.put(4, 0xF);
            w.put(11, 0);
            // assignment 0 is one channel, the centre; 1 is left and right
            w.put(5, (layout.channels - 1) as u64);
        }
    }
    w.put(16, 0xB752);
    // flags: bit 14 marks the DVD-Audio syntax, bit 13 announces heavy
    // dynamic range control
    let dvd_audio = if layout.family == Family::Mlp {
        0x4000
    } else {
        0
    };
    let heavy_drc = if layout.heavy_drc.is_some() {
        0x2000
    } else {
        0
    };
    w.put(16, dvd_audio | heavy_drc);
    w.put(16, 0);
    // variable rate, and the peak in units of a sixteenth of the sampling rate
    w.put(1, 1);
    w.put(15, peak_data_rate);
    // the substreams, and no object presentation to say more of
    w.put(4, 1 + layout.above.len() as u64);
    w.put(4, 0);
    // Which substreams each presentation reads (`substream_info`): two bits
    // for the six-channel one from bit 2 and three for the eight-channel
    // one from bit 4. Each reads its own substream and the ones under it,
    // and a presentation without a substream of its own the ones of the
    // presentation under it.
    let substream_info = match (layout.family, layout.above.len()) {
        (Family::Mlp, _) => 0x05,
        (Family::TrueHd, 0) => 0x14,
        (Family::TrueHd, 1) => 0x3C,
        (Family::TrueHd, _) => 0x7C,
    };
    w.put(8, substream_info);
    match layout.family {
        Family::TrueHd => {
            // Channel meaning. No start-up gain of the heavy dynamic range
            // control, no control enabled for any presentation, a reserved
            // bit, no start-up gain.
            w.put(6, 0);
            w.put(3, 0);
            w.put(1, 0);
            w.put(7, 0);
            // For each presentation a dialogue level of -31 dB, at which a
            // decoder that normalises changes nothing, and a mix level of 0;
            // for the two larger ones a source format of 0.
            w.put(6, 31);
            w.put(6, 0);
            w.put(5, 31);
            w.put(6, 0);
            w.put(5, 0);
            w.put(5, 31);
            w.put(6, 0);
            w.put(6, 0);
            // a reserved bit, and no extension
            w.put(2, 0);
        }
        // DVD-Audio lays these eight bytes out otherwise, and no source says
        // how; they are left at zero
        Family::Mlp => w.put(64, 0),
    }
    let crc = CRC16_MAJOR_SYNC.update_bytes(0, w.bytes());
    w.put(16, u64::from(crc));
    w.into_bytes()
}

/// What a block sends for one channel.
#[derive(Debug, Clone, Copy)]
struct Sent {
    params: bool,
    fir: bool,
    iir: bool,
    offset: bool,
}

/// A substream as the writer goes through it: where its channels are, and
/// what its restart headers say.
#[derive(Debug, Clone, Copy)]
struct Sub<'a> {
    /// Its place among the substreams.
    index: usize,
    /// No substream is over it: the samples the stream is made from are its
    /// presentation.
    top: bool,
    /// The first channel it codes (`min_chan`).
    first: usize,
    /// The last channel it codes, which is the last its matrices reach
    /// (`max_chan`, `max_matrix_chan`).
    last: usize,
    /// Its restart sync word is `0x31EB`: a table of dither where a `0x31EA`
    /// substream has two noise channels.
    dither: bool,
    noise_shift: u8,
    noise_seeds: &'a [u32],
    assignment: &'a [usize],
    protection: Option<Protection>,
}

impl Sub<'_> {
    /// The channels its matrices are over: every channel up to its last.
    fn reach(&self) -> usize {
        self.last + 1
    }

    /// The channels it codes.
    fn coded(&self) -> usize {
        self.reach() - self.first
    }

    /// What a matrix of it has a coefficient for: the channels, and behind
    /// them the two noise channels of a `0x31EA` substream.
    fn columns(&self) -> usize {
        self.reach() + if self.dither { 0 } else { 2 }
    }

    /// The output channel of channel `ch` of its matrices.
    fn output_of(&self, ch: usize) -> usize {
        self.assignment.get(ch).copied().unwrap_or(ch)
    }
}

impl Layout {
    /// The substreams, the first first.
    fn subs(&self) -> Vec<Sub<'_>> {
        let mut subs = vec![Sub {
            index: 0,
            top: self.above.is_empty(),
            first: 0,
            last: self.channels - 1,
            dither: false,
            noise_shift: self.noise_shift,
            noise_seeds: &self.noise_seeds,
            assignment: &self.assignment,
            protection: self.protection,
        }];
        let mut first = self.channels;
        for (i, above) in self.above.iter().enumerate() {
            subs.push(Sub {
                index: i + 1,
                top: i + 1 == self.above.len(),
                first,
                last: first + above.channels - 1,
                dither: above.dither,
                noise_shift: above.noise_shift,
                noise_seeds: &above.noise_seeds,
                assignment: &above.assignment,
                protection: above.protection,
            });
            first += above.channels;
        }
        subs
    }
}

/// The blocks a unit has for the substream at `index`.
fn blocks_of(plan: &UnitPlan, index: usize) -> &[BlockPlan] {
    if index == 0 {
        &plan.blocks
    } else {
        &plan.above[index - 1]
    }
}

/// What the decoder holds for a substream, kept in step with it from one
/// access unit to the next.
#[derive(Debug, Clone)]
struct Running {
    /// The channels the substream codes.
    channels: Vec<Channel>,
    block_size: usize,
    /// The check over every output of its presentation since the last
    /// restart header.
    check: u32,
    /// The matrices in force.
    matrices: Vec<Matrix>,
    /// The output shift in force of each channel the matrices are over.
    shifts: Vec<i8>,
    /// The quantiser step in force of each of those channels, as this
    /// substream has stated it.
    steps: Vec<u8>,
    /// The register the noise channels or the dither are drawn from.
    noise: u32,
    /// The dither of the access unit in hand, in a substream that has a
    /// table of it.
    table: Vec<i64>,
    /// Where the bypassed LSBs come from when nothing decides them: any bit
    /// will do, and these are the same on every run.
    bypass: u64,
    /// Restart headers written so far.
    restarts: usize,
    /// The largest output shift of the substream, which every restart header
    /// states (`max_shift`).
    max_shift: u8,
    /// The guard bits in force.
    guards: u8,
}

impl Running {
    fn new(sub: &Sub<'_>, plans: &[UnitPlan]) -> Self {
        let max_shift = plans
            .iter()
            .flat_map(|plan| blocks_of(plan, sub.index))
            .flat_map(|block| &block.shifts)
            .map(|&shift| shift.max(0) as u8)
            .max()
            .unwrap_or(0);
        Self {
            channels: Vec::new(),
            block_size: DEFAULT_BLOCK,
            check: 0,
            matrices: Vec::new(),
            shifts: vec![0; sub.reach()],
            steps: vec![0; sub.reach()],
            noise: 0,
            table: Vec::new(),
            // each substream draws bits of its own
            bypass: 0x9E37_79B9_7F4A_7C15 ^ (sub.index as u64).wrapping_mul(0xD1B5_4A32_D192_ED03),
            restarts: 0,
            max_shift,
            guards: 0xFF,
        }
    }

    /// The state a restart header leaves, and the register it states.
    fn restart(&mut self, sub: &Sub<'_>) {
        self.channels = vec![Channel::after_restart(); sub.coded()];
        self.block_size = DEFAULT_BLOCK;
        self.matrices.clear();
        self.shifts = vec![0; sub.reach()];
        self.steps = vec![0; sub.reach()];
        self.guards = 0xFF;
        if let Some(&seed) = sub.noise_seeds.get(self.restarts) {
            self.noise = seed;
        }
        self.restarts += 1;
    }

    fn next_bypass_bit(&mut self) -> i64 {
        self.bypass ^= self.bypass << 13;
        self.bypass ^= self.bypass >> 7;
        self.bypass ^= self.bypass << 17;
        (self.bypass >> 32 & 1) as i64
    }

    /// Draws the dither of one access unit from the register: as many values
    /// as the power of two that holds the samples of a unit, each the entry
    /// of the table that the top eight bits of the register name, the
    /// register moving on by eight bits after each.
    fn draw_dither(&mut self, per_unit: usize) {
        self.table.clear();
        for _ in 0..per_unit.next_power_of_two() {
            let top = self.noise >> 15;
            self.table.push(i64::from(DITHER[top as usize]));
            self.noise = ((self.noise << 8) ^ top ^ (top << 5)) & 0x7F_FFFF;
        }
    }
}

/// The two noise samples of one position, and the register moved on: the
/// byte from bit 15 up and the byte from bit 7 up, each as a signed number.
fn noise_pair(register: &mut u32, shift: u8) -> (i64, i64) {
    let low = *register >> 7;
    let first = i64::from((*register >> 15) as u8 as i8) << shift;
    let second = i64::from(low as u8 as i8) << shift;
    *register = (low ^ (low << 5) ^ (*register << 16)) & 0x7F_FFFF;
    (first, second)
}

/// What the dither adds to the sum of matrix `pmi` of `count` at position
/// `n` of its access unit, in units of the 18 fractional bits a decoder sums
/// at: nothing at a scale of 0, and otherwise one value of the unit's table
/// shifted up by 11 bits and the scale. The matrices and the positions each
/// step through the table at a pace of their own.
fn dither_term(table: &[i64], matrix: &Matrix, pmi: usize, count: usize, n: usize) -> i64 {
    if matrix.dither == 0 {
        return 0;
    }
    let index = (count - pmi) * (2 * n + 1) + n;
    table[index & (table.len() - 1)] << (11 + u32::from(matrix.dither))
}

/// The sum of `matrix` at one position without the channel it writes: the
/// other channels by their coefficients, the noise channels among them, and
/// the dither, at 18 fractional bits.
fn others(matrix: &Matrix, row: &[i64], dither: i64) -> i64 {
    let scale = 18 - u32::from(matrix.frac_bits);
    let sum: i64 = row
        .iter()
        .zip(&matrix.coefficients)
        .enumerate()
        .filter(|&(ch, _)| ch != matrix.out)
        .map(|(_, (value, &c))| value * (i64::from(c) << scale))
        .sum();
    sum + dither
}

/// Whether the syntax can say `matrix` in substream `sub`, and the writer
/// undo it where it has to.
fn check_matrix(matrix: &Matrix, sub: &Sub<'_>) -> Result<(), String> {
    if matrix.out > sub.last {
        return Err(format!("a matrix that writes channel {}", matrix.out));
    }
    if matrix.frac_bits > 14 {
        return Err(format!("a matrix of {} fractional bits", matrix.frac_bits));
    }
    if matrix.coefficients.len() != sub.columns() {
        return Err(if sub.dither {
            "a matrix has a coefficient for each channel".into()
        } else {
            "a matrix has a coefficient for each channel and two for the noise".into()
        });
    }
    let most = 1i32 << (matrix.frac_bits + 1);
    if matrix
        .coefficients
        .iter()
        .any(|c| !(-most..most).contains(c))
    {
        return Err(format!(
            "a matrix coefficient outside {} bits",
            matrix.frac_bits + 2
        ));
    }
    if matrix.dither > 15 || (matrix.dither != 0 && !sub.dither) {
        return Err(format!(
            "a dither scale of {}: 0 to 15, and only where the substream has a table of dither",
            matrix.dither
        ));
    }
    // only the top substream undoes its matrices
    if sub.top && matrix.coefficients[matrix.out].abs() != 1 << matrix.frac_bits {
        return Err(format!(
            "the matrix on channel {} does not keep the channel at 1 or -1, and cannot be undone",
            matrix.out
        ));
    }
    Ok(())
}

/// The quantiser step of every channel the matrices of `sub` are over, as
/// `block` states them: of the channels the substream codes, the step they
/// are coded with; of the channels under those, what the block says, or with
/// nothing said what the plan of the substream that codes each has there.
fn stated_steps(
    subs: &[Sub<'_>],
    sub: &Sub<'_>,
    plan: &UnitPlan,
    block: &BlockPlan,
    within: usize,
) -> Result<Vec<u8>, String> {
    let mut steps = Vec::with_capacity(sub.reach());
    if block.steps_below.is_empty() {
        steps.extend((0..sub.first).map(|ch| planned_step(subs, plan, ch, within)));
    } else {
        if block.steps_below.len() != sub.first {
            return Err(format!(
                "a quantiser step for each of the {} channels under the ones the substream codes",
                sub.first
            ));
        }
        steps.extend_from_slice(&block.steps_below);
    }
    steps.extend(block.coding.iter().map(|coding| coding.step));
    if let Some(step) = steps.iter().find(|&&step| step > 15) {
        return Err(format!("a quantiser step of {step}"));
    }
    Ok(steps)
}

/// The quantiser step the plan has on channel `ch` at position `within` of
/// the access unit: that of the block there in the substream that codes the
/// channel.
fn planned_step(subs: &[Sub<'_>], plan: &UnitPlan, ch: usize, within: usize) -> u8 {
    let Some(sub) = subs.iter().find(|sub| (sub.first..=sub.last).contains(&ch)) else {
        return 0;
    };
    let mut start = 0;
    for block in blocks_of(plan, sub.index) {
        if within < start + block.size {
            return block.coding.get(ch - sub.first).map_or(0, |c| c.step);
        }
        start += block.size;
    }
    0
}

/// What lies under the matrices of a block of the top substream.
struct Unmatrixed {
    /// The samples of each channel the matrices are over: of every coded
    /// channel of the stream.
    samples: Vec<Vec<i32>>,
    /// At each position, the bypassed LSB of each matrix.
    bypassed: Vec<Vec<bool>>,
}

/// The samples of the coded channels for `size` positions from `at`, which
/// is position `within` of its access unit, and the bypassed LSB of each
/// matrix at each position: the outputs with the shifts taken off and the
/// matrices in force undone, the last first.
#[allow(
    clippy::needless_range_loop,
    reason = "one position of every channel at a time, the channels in the order of the outputs"
)]
fn undo(
    sub: &Sub<'_>,
    run: &mut Running,
    pcm: &[Vec<i32>],
    at: usize,
    within: usize,
    size: usize,
) -> Result<Unmatrixed, String> {
    let reach = sub.reach();
    let count = run.matrices.len();
    let mut coded = vec![Vec::with_capacity(size); reach];
    let mut bypassed = Vec::with_capacity(size);
    let mut row = vec![0i64; sub.columns()];
    for i in 0..size {
        for (ch, value) in row.iter_mut().take(reach).enumerate() {
            let output = i64::from(pcm[sub.output_of(ch)][at + i]);
            let shift = run.shifts[ch];
            *value = if shift >= 0 {
                if output & ((1 << shift) - 1) != 0 {
                    return Err(format!(
                        "output {output} has bits under an output shift of {shift}"
                    ));
                }
                output >> shift
            } else {
                // the decoder shifts right, and the bits it drops are noughts
                output << -shift
            };
        }
        if !sub.dither {
            (row[reach], row[reach + 1]) = noise_pair(&mut run.noise, sub.noise_shift);
        }
        let mut bits = vec![false; count];
        for pmi in (0..count).rev() {
            let matrix = &run.matrices[pmi];
            let out = matrix.out;
            let step = run.steps[out];
            let mask = !((1i64 << step) - 1);
            let dither = dither_term(&run.table, matrix, pmi, count, within + i);
            let others = others(matrix, &row, dither) >> 18;
            let keeps = matrix.coefficients[out] > 0;
            // The decoder sums `own + others`, `own` being the channel kept
            // or turned over, and does one of two things with the sum. A
            // matrix that sends no bit cuts it to the quantiser step of its
            // channel. A matrix that sends one adds the bit to the sum as it
            // is, with no cut: that is what Dolby's decoder does in both
            // kinds of substream, and no real stream has both to say
            // otherwise. Either way `own` is a multiple of the step, as
            // every coded sample is.
            let (lsb, own) = if matrix.bypass {
                // Without a step any bit can be sent. With one the bit is
                // what makes `own` even, and the rest of `own` has to keep
                // to the step by itself: the output of such a matrix is not
                // free, it is a coded sample that keeps to its step, the
                // share of the other channels and the bit.
                let lsb = if step == 0 {
                    run.next_bypass_bit()
                } else {
                    (row[out] - others) & 1
                };
                (lsb, row[out] - lsb - others)
            } else {
                if row[out] & !mask != 0 {
                    return Err(format!(
                        "channel {out}: {} has bits under a quantiser step of {step} at matrix {pmi}",
                        row[out]
                    ));
                }
                // only the part of `others` above the step comes through
                // the cut
                (0, row[out] - (others & mask))
            };
            if own & !mask != 0 {
                return Err(format!(
                    "channel {out}: {} at matrix {pmi}, which sends a bypassed bit, would need a coded \
                     sample with bits under its quantiser step of {step}",
                    row[out]
                ));
            }
            row[out] = if keeps { own } else { -own };
            bits[pmi] = lsb != 0;
        }
        for (channel, &value) in coded.iter_mut().zip(&row) {
            if !(-LIMIT..LIMIT).contains(&value) {
                return Err(format!(
                    "a sample of {value} under the matrices leaves 24 bits"
                ));
            }
            channel.push(value as i32);
        }
        bypassed.push(bits);
    }
    Ok(Unmatrixed {
        samples: coded,
        bypassed,
    })
}

/// What the matrices of a block make in a substream under the top one.
struct Matrixed {
    /// The outputs, by the channel of the matrices each comes from.
    outputs: Vec<Vec<i32>>,
    /// At each position, the bypassed LSB of each matrix.
    bypassed: Vec<Vec<bool>>,
}

/// The presentation of a substream under the top one for `size` positions
/// from `within` in the access unit: its matrices in order over the coded
/// channels, then the output shifts. This is the decoder's side of the
/// arithmetic, which the top substream is spared. Nothing is undone here,
/// and what comes out is whatever the matrices make. The bypassed LSBs,
/// which nothing decides, are drawn.
fn apply(
    sub: &Sub<'_>,
    run: &mut Running,
    coded: &[Vec<i32>],
    within: usize,
    size: usize,
) -> Result<Matrixed, String> {
    let reach = sub.reach();
    let count = run.matrices.len();
    let mut outputs = vec![Vec::with_capacity(size); reach];
    let mut bypassed = Vec::with_capacity(size);
    let mut row = vec![0i64; sub.columns()];
    for i in 0..size {
        for (value, channel) in row.iter_mut().zip(&coded[..reach]) {
            *value = i64::from(channel[within + i]);
        }
        if !sub.dither {
            (row[reach], row[reach + 1]) = noise_pair(&mut run.noise, sub.noise_shift);
        }
        let mut bits = vec![false; count];
        for (pmi, bit) in bits.iter_mut().enumerate() {
            let lsb = if run.matrices[pmi].bypass {
                run.next_bypass_bit()
            } else {
                0
            };
            let matrix = &run.matrices[pmi];
            let out = matrix.out;
            let mask = !((1i64 << run.steps[out]) - 1);
            let scale = 18 - u32::from(matrix.frac_bits);
            let dither = dither_term(&run.table, matrix, pmi, count, within + i);
            let own = row[out] * (i64::from(matrix.coefficients[out]) << scale);
            let sum = (own + others(matrix, &row, dither)) >> 18;
            // a matrix that sends a bit is not cut to the step
            let value = if matrix.bypass {
                sum + lsb
            } else {
                (sum & mask) + lsb
            };
            if !(-LIMIT..LIMIT).contains(&value) {
                return Err(format!(
                    "matrix {pmi} makes {value} of channel {out}, which leaves 24 bits"
                ));
            }
            row[out] = value;
            *bit = lsb != 0;
        }
        for (ch, output) in outputs.iter_mut().enumerate() {
            let shift = run.shifts[ch];
            let value = if shift >= 0 {
                row[ch] << shift
            } else {
                row[ch] >> -shift
            };
            if !(-LIMIT..LIMIT).contains(&value) {
                return Err(format!(
                    "an output of {value} on channel {ch} leaves 24 bits"
                ));
            }
            output.push(value as i32);
        }
        bypassed.push(bits);
    }
    Ok(Matrixed { outputs, bypassed })
}

/// What the passes over an access unit work between.
struct Work<'a> {
    /// The substreams.
    subs: &'a [Sub<'a>],
    /// The samples the stream is made from: the presentation of the top
    /// substream.
    pcm: &'a [Vec<i32>],
    /// Every coded channel over the access unit in hand, as the top
    /// substream leaves them when it undoes its matrices.
    coded: Vec<Vec<i32>>,
    /// The presentation of each substream under the top one, as its matrices
    /// make it: the samples of each output channel, over the whole stream.
    below: Vec<Vec<Vec<i32>>>,
}

/// The segment that substream `s` has in access unit `k`: its blocks, the
/// termination word if the stream ends with it, and the parity and CRC that
/// close it.
fn segment(
    layout: &Layout,
    s: usize,
    k: usize,
    last: bool,
    plan: &UnitPlan,
    work: &mut Work<'_>,
    run: &mut Running,
) -> Result<Vec<u8>, String> {
    let sub = work.subs[s];
    let per_unit = layout.samples_per_unit();
    let restart = k.is_multiple_of(layout.sync_interval);
    let reach = sub.reach();
    let blocks = blocks_of(plan, s);
    let mut w = BitWriter::new();
    // where the block in hand starts in its access unit
    let mut within = 0;
    for (j, block) in blocks.iter().enumerate() {
        let size = block.size;
        if !(8..=per_unit).contains(&size) {
            return Err(format!("a block of {size} samples"));
        }
        let at = k * per_unit + within;
        let restart_here = restart && j == 0;
        let seed_before = run.noise;
        if restart_here {
            run.restart(&sub);
        }
        // the register a restart header states is the one the first sample
        // after it is drawn from
        let stated_seed = if restart_here { run.noise } else { seed_before };
        if j == 0 && sub.dither {
            run.draw_dither(per_unit);
        }

        // What this block changes above the channels, and the samples under
        // it: of the coded channels in the top substream, which undoes its
        // matrices, and of the outputs in any other, which applies them.
        for (pmi, matrix) in block.matrices.iter().enumerate() {
            check_matrix(matrix, &sub).map_err(|e| format!("matrix {pmi}: {e}"))?;
        }
        let shifts = if block.shifts.is_empty() {
            vec![0; reach]
        } else {
            block.shifts.clone()
        };
        if shifts.len() != reach || shifts.iter().any(|s| !(-8..=7).contains(s)) {
            return Err("an output shift for each channel, of -8 to 7".into());
        }
        let steps = stated_steps(work.subs, &sub, plan, block, within)?;
        let matrices_change = block.matrices != run.matrices;
        let shifts_change = shifts != run.shifts;
        let steps_change = steps != run.steps;
        run.matrices.clone_from(&block.matrices);
        run.shifts = shifts;
        run.steps = steps;
        let bypassed = if sub.top {
            let Unmatrixed { samples, bypassed } = undo(&sub, run, work.pcm, at, within, size)?;
            for (channel, samples) in work.coded.iter_mut().zip(samples) {
                channel[within..within + size].copy_from_slice(&samples);
            }
            bypassed
        } else {
            let Matrixed { outputs, bypassed } = apply(&sub, run, &work.coded, within, size)?;
            for (ch, outputs) in outputs.iter().enumerate() {
                work.below[s][sub.output_of(ch)][at..at + size].copy_from_slice(outputs);
            }
            bypassed
        };

        // What it changes in the channels, and the residuals under that.
        let mut sent = Vec::with_capacity(sub.coded());
        let mut residuals = Vec::with_capacity(sub.coded());
        for (i, state) in run.channels.iter_mut().enumerate() {
            let ch = sub.first + i;
            let target = &block.coding[i];
            if target.fir.order() + target.iir.order() > MAX_TAPS {
                return Err(format!("channel {ch}: more than {MAX_TAPS} taps"));
            }
            if target.fir.order() != 0
                && target.iir.order() != 0
                && target.fir.precision != target.iir.precision
            {
                return Err(format!("channel {ch}: two filters of two precisions"));
            }
            if !(-16384..=16383).contains(&i32::from(target.offset)) || target.book > 3 {
                return Err(format!("channel {ch}: an offset or a book out of range"));
            }
            let mut now = Sent {
                params: target.book != state.held.book,
                fir: target.fir != state.held.fir,
                iir: target.iir != state.held.iir,
                offset: target.offset != state.held.offset,
            };
            if now.iir
                && let Some((shift, values)) = &target.iir.state
            {
                // the decoder loads the state it is sent with the filter
                state.iir_history = [0; MAX_TAPS];
                for (slot, &v) in state.iir_history.iter_mut().zip(values) {
                    *slot = i64::from(v) << shift;
                }
            }
            state.held = target.clone();
            let coded = state
                .residuals(&work.coded[ch][within..within + size])
                .map_err(|e| format!("channel {ch}: {e}"))?;
            let lsbs = choose_lsbs(target, &coded).map_err(|e| format!("channel {ch}: {e}"))?;
            now.params |= now.fir || now.iir || now.offset || lsbs != state.lsbs;
            state.lsbs = lsbs;
            sent.push(now);
            residuals.push(coded);
        }
        let header = restart_here
            || block.guards.is_some()
            || size != run.block_size
            || matrices_change
            || shifts_change
            || steps_change
            || sent.iter().any(|s| s.params);

        w.put_bool(header);
        if header {
            w.put_bool(restart_here);
            if restart_here {
                let start = w.len_bits();
                w.put(
                    14,
                    if sub.dither {
                        RESTART_SYNC_DITHER
                    } else {
                        RESTART_SYNC_NOISE
                    },
                );
                // the samples are due one unit after the unit arrives
                w.put(16, ((k * per_unit) & 0xFFFF) as u64);
                // the first and last channel, the last matrix channel
                w.put(4, sub.first as u64);
                w.put(4, sub.last as u64);
                w.put(4, sub.last as u64);
                // the noise channels: their shift and the register, and the
                // largest output shift a block may ask for
                w.put(4, u64::from(sub.noise_shift));
                w.put(23, u64::from(stated_seed));
                w.put(4, u64::from(run.max_shift));
                // max_lsbs, and max_bits twice
                w.put(5, u64::from(DEFAULT_LSBS));
                w.put(5, 24);
                w.put(5, 24);
                // whether every block states its size and a check byte
                w.put_bool(sub.protection.is_some());
                // the check over every sample since the restart header before
                w.put(8, u64::from(fold_check(run.check)));
                run.check = 0;
                // no high-resolution output timing, two reserved bits; then
                // the update of the heavy dynamic range control, or a bit
                // that says there is none and the twelve bits it leaves
                w.put(3, 0);
                match layout.heavy_drc {
                    Some((gain, time)) => {
                        w.put_bool(true);
                        w.put_signed(9, i64::from(gain));
                        w.put(3, u64::from(time));
                    }
                    None => w.put(13, 0),
                }
                for ch in 0..reach {
                    w.put(6, sub.output_of(ch) as u64);
                }
                let len = w.len_bits() - start;
                let crc = CRC8_RESTART.update_bits(0, w.bytes(), start, len);
                w.put(8, u64::from(crc));
            }
            // A flag is in the stream only while its guard bit is set, and
            // every guard is set after a restart header. The guards a block
            // sends hold for the rest of its own header.
            if run.guards & GUARD_GUARDS != 0 {
                w.put_bool(block.guards.is_some());
                if let Some(guards) = block.guards {
                    w.put(8, u64::from(guards));
                    run.guards = guards;
                }
            } else if block.guards.is_some() {
                return Err("the guards have given up the flag that sends new ones".into());
            }
            let guards = run.guards;
            // the flag of a field: written if its guard is set, and the
            // field cannot be sent if it is not
            let flag = |w: &mut BitWriter, guard: u8, sent: bool, what: &str| {
                if guards & guard != 0 {
                    w.put_bool(sent);
                    Ok(())
                } else if sent {
                    Err(format!("the guards leave no flag for {what}"))
                } else {
                    Ok(())
                }
            };
            flag(
                &mut w,
                GUARD_BLOCK_SIZE,
                size != run.block_size,
                "a block size",
            )?;
            if size != run.block_size {
                w.put(9, size as u64);
                run.block_size = size;
            }
            flag(&mut w, GUARD_MATRICES, matrices_change, "the matrices")?;
            if matrices_change {
                w.put(4, run.matrices.len() as u64);
                for matrix in &run.matrices {
                    w.put(4, matrix.out as u64);
                    w.put(4, u64::from(matrix.frac_bits));
                    w.put_bool(matrix.bypass);
                    // a flag for each coefficient, and only the ones that
                    // are not nought behind it
                    for &c in &matrix.coefficients {
                        w.put_bool(c != 0);
                        if c != 0 {
                            w.put_signed(u32::from(matrix.frac_bits) + 2, i64::from(c));
                        }
                    }
                    // where a `0x31EA` matrix has its noise channels
                    if sub.dither {
                        w.put(4, u64::from(matrix.dither));
                    }
                }
            }
            flag(&mut w, GUARD_SHIFTS, shifts_change, "the output shifts")?;
            if shifts_change {
                for &shift in &run.shifts {
                    w.put_signed(4, i64::from(shift));
                }
            }
            flag(&mut w, GUARD_STEPS, steps_change, "the quantiser steps")?;
            if steps_change {
                // of every channel up to the last coded one, the ones under
                // the substream's own included
                for &step in &run.steps {
                    w.put(4, u64::from(step));
                }
            }
            for (state, now) in run.channels.iter().zip(&sent) {
                w.put_bool(now.params);
                if !now.params {
                    continue;
                }
                flag(&mut w, GUARD_FILTER_A, now.fir, "filter A")?;
                if now.fir {
                    put_filter(&mut w, &state.held.fir, false)?;
                }
                flag(&mut w, GUARD_FILTER_B, now.iir, "filter B")?;
                if now.iir {
                    put_filter(&mut w, &state.held.iir, true)?;
                }
                flag(&mut w, GUARD_OFFSET, now.offset, "an offset")?;
                if now.offset {
                    w.put_signed(15, i64::from(state.held.offset));
                }
                w.put(2, u64::from(state.held.book));
                w.put(5, u64::from(state.lsbs));
            }
        }

        // The samples: for each one, the bypassed LSB of each matrix that
        // sends one, then the code and the LSBs of each channel. A protected
        // block states their size in front and a check byte behind.
        let outputs: &[Vec<i32>] = if sub.top { work.pcm } else { &work.below[s] };
        let mut data = BitWriter::new();
        for n in 0..size {
            for (matrix, &bit) in run.matrices.iter().zip(&bypassed[n]) {
                if matrix.bypass {
                    data.put_bool(bit);
                }
            }
            for (state, coded) in run.channels.iter().zip(&residuals) {
                let coding = &state.held;
                let lsb_bits = u32::from(state.lsbs - coding.step);
                let value = (coded[n] >> coding.step) + sign_bias(coding.book, lsb_bits)
                    - i64::from(coding.offset);
                if coding.book != 0 {
                    put_code(&mut data, coding.book, value >> lsb_bits);
                }
                data.put(lsb_bits, (value & ((1i64 << lsb_bits) - 1)) as u64);
            }
            run.check ^= check_word((0..reach).map(|ch| outputs[sub.output_of(ch)][at + n]));
        }
        if let Some(protection) = sub.protection {
            let stated = data.len_bits() + usize::from(protection.counted_beyond_the_data);
            if stated > 0xFFFF {
                return Err(format!("a block of {stated} bits, which 16 cannot state"));
            }
            w.put(16, stated as u64);
        }
        w.append(&data);
        if let Some(protection) = sub.protection {
            w.put(8, u64::from(protection.check));
        }
        within += size;
        w.put_bool(j + 1 == blocks.len());
    }
    w.align(16);
    if last {
        // the stream ends here, with no samples to discard
        w.put(18, TERMINATOR);
        w.put_bool(false);
        w.put(13, TERMINATOR_TAIL);
    }
    if layout.trailer {
        let parity = xor_bytes(w.bytes()) ^ PARITY_XOR;
        let crc = CRC8_SUBSTREAM.update_bytes(CRC8_SUBSTREAM_INIT, w.bytes());
        w.put(8, u64::from(parity));
        w.put(8, u64::from(crc));
    }
    Ok(w.into_bytes())
}

/// Whether the writer can write the substreams a layout has over its first:
/// one that brings the channels to six and one more that brings them to
/// eight, which are the layouts its major sync names.
fn check_above(layout: &Layout) -> Result<(), String> {
    if layout.above.is_empty() {
        return Ok(());
    }
    if layout.family != Family::TrueHd || layout.channels != 2 {
        return Err(
            "substreams over the first are TrueHD's here, and the first has two channels".into(),
        );
    }
    if layout.above.len() > 2
        || layout
            .above
            .iter()
            .zip([4, 2])
            .any(|(above, channels)| above.channels != channels)
    {
        return Err(
            "a second substream of four channels and a third of two: six channels and eight are \
             the layouts the major sync of this writer names"
                .into(),
        );
    }
    if layout
        .above
        .iter()
        .any(|above| above.noise_shift > 15 || above.noise_seeds.iter().any(|&s| s > 0x7F_FFFF))
    {
        return Err("a noise shift of 0 to 15 and a register of 23 bits".into());
    }
    Ok(())
}

/// Writes a stream of `plans.len()` access units holding `pcm`, one vector of
/// samples for each channel of its widest presentation.
///
/// # Errors
/// When the plan asks for something the syntax cannot say, or the samples do
/// not fit what the plan codes them with.
pub fn write(layout: &Layout, plans: &[UnitPlan], pcm: &[Vec<i32>]) -> Result<Stream, String> {
    let per_unit = layout.samples_per_unit();
    if !RATES.contains(&layout.rate) {
        return Err(format!("{} Hz is not a rate of the format", layout.rate));
    }
    let fewest = match layout.family {
        // the two-channel presentation of a TrueHD stream is two channels
        Family::TrueHd => 2,
        Family::Mlp => 1,
    };
    if !(fewest..=2).contains(&layout.channels)
        || (layout.above.is_empty() && pcm.len() != layout.channels)
    {
        return Err(format!(
            "{fewest} to 2 channels, with samples for each, not {}",
            layout.channels
        ));
    }
    check_above(layout)?;
    if pcm.len() != layout.total_channels() {
        return Err(format!(
            "samples for each of the {} channels of the substreams together, not for {}",
            layout.total_channels(),
            pcm.len()
        ));
    }
    if layout.sync_interval == 0 || layout.sync_interval > 128 {
        return Err("a major sync at least every 128 access units".into());
    }
    if let Some((gain, time)) = layout.heavy_drc
        && (layout.family != Family::TrueHd || !(-256..256).contains(&gain) || time > 7)
    {
        return Err(
            "heavy dynamic range control is TrueHD's: a gain of 9 bits, a time of 3".into(),
        );
    }
    if layout.noise_shift > 15 || layout.noise_seeds.iter().any(|&seed| seed > 0x7F_FFFF) {
        return Err("a noise shift of 0 to 15 and a register of 23 bits".into());
    }
    let subs = layout.subs();
    for sub in &subs {
        if !sub.assignment.is_empty() {
            let mut seen = sub.assignment.to_vec();
            seen.sort_unstable();
            if seen != (0..sub.reach()).collect::<Vec<_>>() {
                return Err("an assignment gives every output channel to one coded channel".into());
            }
        }
    }
    if plans.is_empty() || pcm.iter().any(|ch| ch.len() != plans.len() * per_unit) {
        return Err(format!(
            "{} access units of {per_unit} samples each",
            plans.len()
        ));
    }
    if plans
        .iter()
        .any(|plan| plan.above.len() != layout.above.len())
    {
        return Err(format!(
            "every access unit has blocks for each of the {} substreams",
            subs.len()
        ));
    }
    // which substream an error is in, where there are several
    let place = |k: usize, s: usize| {
        if subs.len() == 1 {
            format!("access unit {k}")
        } else {
            format!("access unit {k}, substream {s}")
        }
    };

    // The segments first: the major sync states the peak data rate, which is
    // that of the largest unit.
    let top = subs.len() - 1;
    let mut runs: Vec<Running> = subs.iter().map(|sub| Running::new(sub, plans)).collect();
    let mut work = Work {
        subs: &subs,
        pcm,
        coded: vec![vec![0; per_unit]; layout.total_channels()],
        below: subs[..top]
            .iter()
            .map(|sub| vec![vec![0; plans.len() * per_unit]; sub.reach()])
            .collect(),
    };
    let mut units = Vec::with_capacity(plans.len());
    for (k, plan) in plans.iter().enumerate() {
        for sub in &subs {
            let blocks = blocks_of(plan, sub.index);
            if blocks.iter().any(|b| b.coding.len() != sub.coded())
                || blocks.iter().map(|b| b.size).sum::<usize>() != per_unit
            {
                return Err(format!(
                    "{}: a coding for each channel and blocks of {per_unit} samples in all",
                    place(k, sub.index)
                ));
            }
        }
        // The top substream first: undoing its matrices is what settles the
        // samples every substream codes.
        let mut segments = vec![Vec::new(); subs.len()];
        for s in std::iter::once(top).chain(0..top) {
            segments[s] = segment(
                layout,
                s,
                k,
                k + 1 == plans.len(),
                plan,
                &mut work,
                &mut runs[s],
            )
            .map_err(|e| format!("{}: {e}", place(k, s)))?;
        }
        units.push(segments);
    }
    let unit_len = |k: usize, segments: &[Vec<u8>]| {
        let sync = if k.is_multiple_of(layout.sync_interval) {
            28
        } else {
            0
        };
        4 + sync + 2 * segments.len() + segments.iter().map(Vec::len).sum::<usize>()
    };
    let largest = units
        .iter()
        .enumerate()
        .map(|(k, s)| unit_len(k, s))
        .max()
        .unwrap_or(0);
    // Bits of the largest unit over the time of one, in units of a sixteenth
    // of the sampling rate; and one more, because a reader turns the field
    // into bits a second by rounding, and the largest unit must not come out
    // over its own peak by the fraction that loses.
    let peak = ((largest * 8 * 16).div_ceil(per_unit) as u64 + 1).min(0x7FFF);
    let sync = major_sync(layout, peak);

    let mut bytes = Vec::new();
    let mut unit_offsets = Vec::with_capacity(units.len());
    for (k, segments) in units.iter().enumerate() {
        let with_sync = k.is_multiple_of(layout.sync_interval);
        let len = unit_len(k, segments);
        let words = len / 2;
        if !len.is_multiple_of(2) || words > 0x0FFF {
            return Err(format!("access unit {k} of {len} bytes"));
        }
        // a unit arrives one unit before its samples are due
        let input_timing = (k as i64 - 1) * per_unit as i64;
        let mut unit = vec![
            (words >> 8) as u8,
            words as u8,
            (input_timing >> 8) as u8,
            input_timing as u8,
        ];
        if with_sync {
            unit.extend_from_slice(&sync);
        }
        // The directory, an entry for each substream: no extra word, whether
        // the segment is without a restart header, whether parity and CRC
        // close it, a reserved bit, and where it ends, in words from the
        // first segment.
        let mut directory = Vec::with_capacity(2 * segments.len());
        let mut end = 0;
        for segment in segments {
            end += segment.len() / 2;
            let entry =
                (u16::from(!with_sync) << 14) | (u16::from(layout.trailer) << 13) | end as u16;
            directory.extend_from_slice(&entry.to_be_bytes());
        }
        unit.extend_from_slice(&directory);
        // the nibble that makes header and directory fold to 0xF
        let nibble = 0xF ^ fold_nibble(xor_bytes(&unit[..4]) ^ xor_bytes(&directory));
        unit[0] |= nibble << 4;
        for segment in segments {
            unit.extend_from_slice(segment);
        }
        unit_offsets.push(bytes.len());
        bytes.extend_from_slice(&unit);
    }
    Ok(Stream {
        bytes,
        unit_offsets,
        below: work.below,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fewest_signed_bits_hold_the_value_and_no_fewer() {
        for (value, bits) in [
            (0i64, 1),
            (-1, 1),
            (1, 2),
            (-2, 2),
            (127, 8),
            (-128, 8),
            (128, 9),
            (-129, 9),
        ] {
            assert_eq!(signed_bits([value].into_iter()), bits, "{value}");
        }
        assert_eq!(signed_bits([3, -100, 5].into_iter()), 8);
        assert_eq!(signed_bits(std::iter::empty()), 1);
    }

    /// The codes of each book as the document gives them, written out.
    #[test]
    fn the_codes_of_each_book() {
        let code = |book: u8, value: i64| {
            let mut w = BitWriter::new();
            put_code(&mut w, book, value);
            let bits = w.len_bits();
            let word = w
                .bytes()
                .iter()
                .fold(0u32, |acc, &b| (acc << 8) | u32::from(b));
            (bits, word >> (w.bytes().len() * 8 - bits))
        };
        // book 1: a 1 and two bits for 0 to 3
        assert_eq!(code(1, 0), (3, 0b100));
        assert_eq!(code(1, 3), (3, 0b111));
        assert_eq!(code(1, 4), (3, 0b011));
        assert_eq!(code(1, 10), (9, 0b01_0000001));
        assert_eq!(code(1, -1), (3, 0b001));
        assert_eq!(code(1, -7), (9, 0b00_0000001));
        // book 2: a 1 and one bit for 0 and 1
        assert_eq!(code(2, 1), (2, 0b11));
        assert_eq!(code(2, 2), (3, 0b011));
        assert_eq!(code(2, 8), (9, 0b01_0000001));
        // book 3: a 1 for 0
        assert_eq!(code(3, 0), (1, 0b1));
        assert_eq!(code(3, 1), (3, 0b011));
        assert_eq!(code(3, 7), (9, 0b01_0000001));
        assert_eq!(code(3, -3), (5, 0b00_001));
        for book in 1..=3 {
            let (low, high) = book_range(book);
            for value in low..=high {
                assert!(code(book, value).0 <= 9, "book {book}, value {value}");
            }
        }
    }

    #[test]
    fn the_check_word_folds_to_the_byte_a_restart_header_carries() {
        assert_eq!(fold_check(0), 0);
        assert_eq!(fold_check(0x0000_0001), 0x01);
        assert_eq!(fold_check(0x0100_0000), 0x01);
        assert_eq!(fold_check(0x1234_5678), 0x12 ^ 0x34 ^ 0x56 ^ 0x78);
        // channel 1 enters one bit up
        assert_eq!(check_word([1, 1].into_iter()), 0b11);
        assert_eq!(check_word([-1, 0].into_iter()), 0x00FF_FFFF);
    }

    #[test]
    fn the_least_lsbs_that_hold_a_block_are_chosen() {
        let plain = Coding::default();
        // -4 to 3 is three bits of two's complement
        assert_eq!(choose_lsbs(&plain, &[-4, 0, 3]), Ok(3));
        assert_eq!(choose_lsbs(&plain, &[-5, 0, 3]), Ok(4));
        assert_eq!(choose_lsbs(&plain, &[0, 0]), Ok(0));
        // a step takes its bits off the value and adds them to huff_lsbs
        let stepped = Coding {
            step: 2,
            ..Coding::default()
        };
        assert_eq!(choose_lsbs(&stepped, &[-16, 12]), Ok(5));
        // the offset moves the range
        let offset = Coding {
            offset: 100,
            ..Coding::default()
        };
        assert_eq!(choose_lsbs(&offset, &[100, 99, 100]), Ok(1));
        // a book covers a few values with no LSBs at all
        let book = Coding {
            book: 1,
            ..Coding::default()
        };
        assert_eq!(choose_lsbs(&book, &[-7, 10 - 2, 0]), Ok(0));
        // and a width that is asked for is kept, or refused
        let fixed = Coding {
            lsbs: Some(12),
            ..Coding::default()
        };
        assert_eq!(choose_lsbs(&fixed, &[-4, 3]), Ok(12));
        assert!(choose_lsbs(&fixed, &[1 << 12]).is_err());
    }
}
