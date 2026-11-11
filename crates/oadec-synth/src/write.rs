//! The writer: access units from samples and a plan.
//!
//! This is the syntax and the arithmetic of `docs/truehd.md` written from the
//! encoding side. It shares no code with the decoder. A stream it writes
//! carries one substream of one or two channels, which is the whole of a
//! two-channel presentation, and every element of the block syntax under
//! that: both prediction filters with every order, coded filter states, the
//! three code books and plain coding, the offset, the quantiser step, several
//! blocks to a unit; and over the channels the matrices of a `0x31EA`
//! substream with their two noise channels and bypassed LSBs, the output
//! shifts and the channel assignment.
//!
//! Lossless by construction, from the output backwards. The output shift is
//! taken off the samples, then each matrix is undone, the last first: a
//! matrix whose coefficient on the channel it writes is 1 or -1 loses
//! nothing. What is left are the samples of the coded channels, and the
//! residual of each is what remains of it when the prediction the decoder
//! will make is taken off. The writer runs the decoder's recursions to know
//! what they will give; it does not search for good filters or matrices, it
//! is told them.

use oadec_bits::{CRC8_RESTART, CRC8_SUBSTREAM, CRC8_SUBSTREAM_INIT, CRC16_MAJOR_SYNC};
use oadec_bits::{fold_nibble, xor_bytes};

use crate::bits::BitWriter;

/// The sampling rates of the format: the 48 kHz family and the 44.1 kHz one.
pub const RATES: [u32; 6] = [44_100, 48_000, 88_200, 96_000, 176_400, 192_000];

/// The most taps the two filters of a channel have between them.
pub const MAX_TAPS: usize = 8;

/// The restart sync word of a substream with MLP-style noise channels: the
/// one the substream of a two-channel presentation carries.
const RESTART_SYNC: u64 = 0x31EA;
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
    /// Channels, 1 or 2.
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
        }
    }

    /// The output channel of coded channel `ch`.
    fn output_of(&self, ch: usize) -> usize {
        self.assignment.get(ch).copied().unwrap_or(ch)
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
    /// A coefficient for each channel and then for the two noise channels,
    /// in units of `2^-frac_bits`. The one on the channel the matrix writes
    /// is 1 or -1: a matrix can be undone exactly with no other.
    pub coefficients: Vec<i32>,
    /// One more bit a sample, sent as it is and added to the sum, which is
    /// then not cut to a quantiser step (`lsb_bypass_used`).
    pub bypass: bool,
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
        }
    }
}

/// One block: so many samples of every channel, and what codes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockPlan {
    /// Samples, 8 at least.
    pub size: usize,
    /// The coding of each channel from this block on. A block says of it
    /// only what differs from the block before.
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
}

/// The blocks of one access unit, whose sizes add up to the samples of a
/// unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitPlan {
    /// The blocks, in order.
    pub blocks: Vec<BlockPlan>,
}

impl UnitPlan {
    /// A unit of blocks of these sizes, all coded alike.
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
                })
                .collect(),
        }
    }

    /// The unit with its first block sending these guard bits.
    #[must_use]
    pub fn guarded(mut self, guards: u8) -> Self {
        if let Some(first) = self.blocks.first_mut() {
            first.guards = Some(guards);
        }
        self
    }

    /// The unit with these matrices in force in every block.
    #[must_use]
    pub fn matrixed(mut self, matrices: &[Matrix]) -> Self {
        for block in &mut self.blocks {
            block.matrices = matrices.to_vec();
        }
        self
    }

    /// The unit with these output shifts in force in every block.
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
            // The two-channel presentation is plain stereo (modifier 0). The
            // six- and eight-channel ones are copies of it: nothing indicated
            // by their modifiers, and an assignment of left and right.
            w.put(2, 0);
            w.put(2, 0);
            w.put(5, 1);
            w.put(2, 0);
            w.put(13, 1);
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
    // one substream
    w.put(4, 1);
    w.put(4, 0);
    // which presentations it carries
    w.put(
        8,
        if layout.family == Family::Mlp {
            0x05
        } else {
            0x14
        },
    );
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

/// What the decoder holds for the substream, kept in step with it from one
/// access unit to the next.
#[derive(Debug, Clone)]
struct Running {
    channels: Vec<Channel>,
    block_size: usize,
    /// The check over every output since the last restart header.
    check: u32,
    /// The matrices in force.
    matrices: Vec<Matrix>,
    /// The output shift of each channel in force.
    shifts: Vec<i8>,
    /// The register the two noise channels are drawn from.
    noise: u32,
    /// Where the bypassed LSBs come from when nothing decides them: any bit
    /// will do, and these are the same on every run.
    bypass: u64,
    /// Restart headers written so far.
    restarts: usize,
    /// The largest output shift of the stream, which every restart header
    /// states (`max_shift`).
    max_shift: u8,
    /// The guard bits in force.
    guards: u8,
}

impl Running {
    fn new(layout: &Layout, plans: &[UnitPlan]) -> Self {
        let max_shift = plans
            .iter()
            .flat_map(|plan| &plan.blocks)
            .flat_map(|block| &block.shifts)
            .map(|&shift| shift.max(0) as u8)
            .max()
            .unwrap_or(0);
        Self {
            channels: Vec::new(),
            block_size: DEFAULT_BLOCK,
            check: 0,
            matrices: Vec::new(),
            shifts: vec![0; layout.channels],
            noise: 0,
            bypass: 0x9E37_79B9_7F4A_7C15,
            restarts: 0,
            max_shift,
            guards: 0xFF,
        }
    }

    /// The state a restart header leaves, and the noise register it states.
    fn restart(&mut self, layout: &Layout) {
        self.channels = vec![Channel::after_restart(); layout.channels];
        self.block_size = DEFAULT_BLOCK;
        self.matrices.clear();
        self.shifts = vec![0; layout.channels];
        self.guards = 0xFF;
        if let Some(&seed) = layout.noise_seeds.get(self.restarts) {
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

/// Whether the syntax can say `matrix` for a substream of `channels`
/// channels, and the writer undo it.
fn check_matrix(matrix: &Matrix, channels: usize) -> Result<(), String> {
    if matrix.out >= channels {
        return Err(format!("a matrix that writes channel {}", matrix.out));
    }
    if matrix.frac_bits > 14 {
        return Err(format!("a matrix of {} fractional bits", matrix.frac_bits));
    }
    if matrix.coefficients.len() != channels + 2 {
        return Err("a matrix has a coefficient for each channel and two for the noise".into());
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
    if matrix.coefficients[matrix.out].abs() != 1 << matrix.frac_bits {
        return Err(format!(
            "the matrix on channel {} does not keep the channel at 1 or -1, and cannot be undone",
            matrix.out
        ));
    }
    Ok(())
}

/// What lies under the matrices of a block.
struct Unmatrixed {
    /// The samples of each coded channel.
    samples: Vec<Vec<i32>>,
    /// At each position, the bypassed LSB of each matrix.
    bypassed: Vec<Vec<bool>>,
}

/// The samples of the coded channels for `size` positions from `at`, and the
/// bypassed LSB of each matrix at each position: the outputs with the shifts
/// taken off and the matrices in force undone, the last first.
#[allow(
    clippy::needless_range_loop,
    reason = "one position of every channel at a time, the channels in the order of the outputs"
)]
fn coded_samples(
    layout: &Layout,
    block: &BlockPlan,
    run: &mut Running,
    pcm: &[Vec<i32>],
    at: usize,
    size: usize,
) -> Result<Unmatrixed, String> {
    let channels = layout.channels;
    let mut coded = vec![Vec::with_capacity(size); channels];
    let mut bypassed = Vec::with_capacity(size);
    let mut row = vec![0i64; channels + 2];
    for n in at..at + size {
        for (ch, value) in row.iter_mut().take(channels).enumerate() {
            let output = i64::from(pcm[layout.output_of(ch)][n]);
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
        (row[channels], row[channels + 1]) = noise_pair(&mut run.noise, layout.noise_shift);
        let mut bits = vec![false; run.matrices.len()];
        for pmi in (0..run.matrices.len()).rev() {
            let matrix = &run.matrices[pmi];
            let out = matrix.out;
            let step = block.coding[out].step;
            let mask = !((1i64 << step) - 1);
            let scale = 18 - u32::from(matrix.frac_bits);
            let others: i64 = row
                .iter()
                .zip(&matrix.coefficients)
                .enumerate()
                .filter(|&(ch, _)| ch != out)
                .map(|(_, (value, &c))| value * (i64::from(c) << scale))
                .sum::<i64>()
                >> 18;
            let keeps = matrix.coefficients[out] > 0;
            // The decoder sums `own + others`, `own` being the channel kept
            // or turned over, and does one of two things with the sum. A
            // matrix that sends no bit cuts it to the quantiser step of its
            // channel. A matrix that sends one adds the bit to the sum as it
            // is, with no cut: that is what Dolby's decoder does, and no
            // real stream has both to say otherwise. Either way `own` is a
            // multiple of the step, as every coded sample is.
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

/// The segment of access unit `k`: its blocks, the termination word if the
/// stream ends with it, and the parity and CRC that close it.
fn segment(
    layout: &Layout,
    k: usize,
    last: bool,
    plan: &UnitPlan,
    pcm: &[Vec<i32>],
    run: &mut Running,
) -> Result<Vec<u8>, String> {
    let per_unit = layout.samples_per_unit();
    let restart = k.is_multiple_of(layout.sync_interval);
    let mut w = BitWriter::new();
    let mut at = k * per_unit;
    for (j, block) in plan.blocks.iter().enumerate() {
        let size = block.size;
        if !(8..=per_unit).contains(&size) {
            return Err(format!("a block of {size} samples"));
        }
        let restart_here = restart && j == 0;
        let seed_before = run.noise;
        if restart_here {
            run.restart(layout);
        }
        // the register a restart header states is the one the first sample
        // after it is drawn from
        let stated_seed = if restart_here { run.noise } else { seed_before };

        // What this block changes above the channels, and the samples of the
        // coded channels under it.
        for (pmi, matrix) in block.matrices.iter().enumerate() {
            check_matrix(matrix, layout.channels).map_err(|e| format!("matrix {pmi}: {e}"))?;
        }
        let shifts = if block.shifts.is_empty() {
            vec![0; layout.channels]
        } else {
            block.shifts.clone()
        };
        if shifts.len() != layout.channels || shifts.iter().any(|s| !(-8..=7).contains(s)) {
            return Err("an output shift for each channel, of -8 to 7".into());
        }
        let matrices_change = block.matrices != run.matrices;
        let shifts_change = shifts != run.shifts;
        run.matrices.clone_from(&block.matrices);
        run.shifts = shifts;
        let Unmatrixed { samples, bypassed } = coded_samples(layout, block, run, pcm, at, size)?;

        // What it changes in the channels, and the residuals under that.
        let steps_change = block
            .coding
            .iter()
            .zip(run.channels.iter())
            .any(|(target, state)| target.step != state.held.step);
        let mut sent = Vec::with_capacity(layout.channels);
        let mut residuals = Vec::with_capacity(layout.channels);
        for (ch, state) in run.channels.iter_mut().enumerate() {
            let target = &block.coding[ch];
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
                .residuals(&samples[ch])
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
                w.put(14, RESTART_SYNC);
                // the samples are due one unit after the unit arrives
                w.put(16, ((k * per_unit) & 0xFFFF) as u64);
                // the first and last channel, the last matrix channel
                w.put(4, 0);
                w.put(4, (layout.channels - 1) as u64);
                w.put(4, (layout.channels - 1) as u64);
                // the noise channels: their shift and the register, and the
                // largest output shift a block may ask for
                w.put(4, u64::from(layout.noise_shift));
                w.put(23, u64::from(stated_seed));
                w.put(4, u64::from(run.max_shift));
                // max_lsbs, and max_bits twice
                w.put(5, u64::from(DEFAULT_LSBS));
                w.put(5, 24);
                w.put(5, 24);
                // whether every block states its size and a check byte
                w.put_bool(layout.protection.is_some());
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
                for ch in 0..layout.channels {
                    w.put(6, layout.output_of(ch) as u64);
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
                for state in &run.channels {
                    w.put(4, u64::from(state.held.step));
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
            run.check ^=
                check_word((0..layout.channels).map(|ch| pcm[layout.output_of(ch)][at + n]));
        }
        if let Some(protection) = layout.protection {
            let stated = data.len_bits() + usize::from(protection.counted_beyond_the_data);
            if stated > 0xFFFF {
                return Err(format!("a block of {stated} bits, which 16 cannot state"));
            }
            w.put(16, stated as u64);
        }
        w.append(&data);
        if let Some(protection) = layout.protection {
            w.put(8, u64::from(protection.check));
        }
        at += size;
        w.put_bool(j + 1 == plan.blocks.len());
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

/// Writes a stream of `plans.len()` access units holding `pcm`, one vector of
/// samples for each channel.
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
    if !(fewest..=2).contains(&layout.channels) || pcm.len() != layout.channels {
        return Err(format!(
            "{fewest} to 2 channels, with samples for each, not {}",
            layout.channels
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
    if !layout.assignment.is_empty() {
        let mut seen = layout.assignment.clone();
        seen.sort_unstable();
        if seen != (0..layout.channels).collect::<Vec<_>>() {
            return Err("an assignment gives every output channel to one coded channel".into());
        }
    }
    if plans.is_empty() || pcm.iter().any(|ch| ch.len() != plans.len() * per_unit) {
        return Err(format!(
            "{} access units of {per_unit} samples each",
            plans.len()
        ));
    }

    // The segments first: the major sync states the peak data rate, which is
    // that of the largest unit.
    let mut run = Running::new(layout, plans);
    let mut segments = Vec::with_capacity(plans.len());
    for (k, plan) in plans.iter().enumerate() {
        if plan
            .blocks
            .iter()
            .any(|b| b.coding.len() != layout.channels)
            || plan.blocks.iter().map(|b| b.size).sum::<usize>() != per_unit
        {
            return Err(format!(
                "access unit {k}: a coding for each channel and blocks of {per_unit} samples in all"
            ));
        }
        let bytes = segment(layout, k, k + 1 == plans.len(), plan, pcm, &mut run)
            .map_err(|e| format!("access unit {k}: {e}"))?;
        segments.push(bytes);
    }
    let unit_len = |k: usize, segment: &[u8]| {
        let sync = if k.is_multiple_of(layout.sync_interval) {
            28
        } else {
            0
        };
        4 + sync + 2 + segment.len()
    };
    let largest = segments
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
    let mut unit_offsets = Vec::with_capacity(segments.len());
    for (k, segment) in segments.iter().enumerate() {
        let with_sync = k.is_multiple_of(layout.sync_interval);
        let len = unit_len(k, segment);
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
        // The directory entry: no extra word, whether the segment is without
        // a restart header, whether parity and CRC close it, a reserved bit,
        // and where it ends, in words.
        let entry = (u16::from(!with_sync) << 14)
            | (u16::from(layout.trailer) << 13)
            | (segment.len() / 2) as u16;
        let directory = entry.to_be_bytes();
        unit.extend_from_slice(&directory);
        // the nibble that makes header and directory fold to 0xF
        let nibble = 0xF ^ fold_nibble(xor_bytes(&unit[..4]) ^ xor_bytes(&directory));
        unit[0] |= nibble << 4;
        unit.extend_from_slice(segment);
        unit_offsets.push(bytes.len());
        bytes.extend_from_slice(&unit);
    }
    Ok(Stream {
        bytes,
        unit_offsets,
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
